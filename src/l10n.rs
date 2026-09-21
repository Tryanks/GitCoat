//! UI localisation: the supported languages, per-request language selection
//! and the `tr!` helper around `rust_i18n::t!`.
//!
//! Translations live in `locales/<tag>.yml` and are compiled into the binary
//! by `rust_i18n::i18n!` (see `lib.rs`). Requests run concurrently, so the
//! process-wide `rust_i18n::set_locale` is never used: every lookup passes
//! the request's [`Lang`] explicitly.
//!
//! Selection order for a request:
//! 1. the `gitcoat_lang` cookie when it holds a supported tag,
//! 2. the `Accept-Language` header, negotiated by q-value,
//! 3. English.
//!
//! Only Git *presentation* is localised; commit messages, author names, ref
//! names, file names and object ids are shown verbatim.

use http::HeaderMap;
use topcoat::{context::Cx, router::request::headers};

/// Name of the cookie that pins the UI language.
pub const COOKIE_NAME: &str = "gitcoat_lang";

/// Lifetime of the language cookie in seconds (one year).
pub const COOKIE_MAX_AGE: u32 = 31_536_000;

/// A UI language with a translation file under `locales/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    /// English (`en`): the default and the fallback for missing keys.
    En,
    /// Simplified Chinese (`zh-CN`).
    ZhCn,
}

impl Lang {
    /// The language used when nothing else applies.
    pub const DEFAULT: Lang = Lang::En;

    /// Every supported language, in switcher order.
    pub fn all() -> &'static [Lang] {
        &[Lang::En, Lang::ZhCn]
    }

    /// BCP 47 tag; also the locale name for `rust_i18n` and the file stem
    /// under `locales/`.
    pub fn tag(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::ZhCn => "zh-CN",
        }
    }

    /// The language's own name, for the switcher.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::ZhCn => "简体中文",
        }
    }

    /// The language whose tag is exactly `tag` (ASCII case-insensitive), as
    /// used for the cookie value and the `/lang?set=` parameter.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        Lang::all()
            .iter()
            .copied()
            .find(|lang| lang.tag().eq_ignore_ascii_case(tag.trim()))
    }

    /// Map one `Accept-Language` range onto a supported language.
    ///
    /// `en` and any `en-*` map to English. `zh`, `zh-CN`, `zh-SG` and every
    /// `zh-Hans*` map to Simplified Chinese. Traditional Chinese ranges
    /// (`zh-TW`, `zh-HK`, `zh-MO`, `zh-Hant*`) are *not* matched because no
    /// Traditional Chinese translation exists yet; a visitor asking only for
    /// them gets English rather than Simplified Chinese. `*` never matches.
    pub fn match_range(range: &str) -> Option<Lang> {
        let range = range.trim().to_ascii_lowercase();
        let (primary, rest) = match range.split_once('-') {
            Some((primary, rest)) => (primary, Some(rest)),
            None => (range.as_str(), None),
        };
        match primary {
            "en" => Some(Lang::En),
            "zh" => {
                let region = rest
                    .map(|r| r.split('-').next().unwrap_or(""))
                    .unwrap_or("");
                match region {
                    "" | "cn" | "sg" | "hans" => Some(Lang::ZhCn),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The language pinned by the `gitcoat_lang` cookie, if it is supported.
    pub fn from_cookie(headers: &HeaderMap) -> Option<Lang> {
        headers
            .get_all(http::header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .find(|(name, _)| name.trim() == COOKIE_NAME)
            .and_then(|(_, value)| Lang::from_tag(value))
    }

    /// Negotiate from an `Accept-Language` header value: ranges are ordered
    /// by descending q-value (ties keep header order), `q=0` ranges are
    /// dropped, and the first range with a supported match wins.
    pub fn from_accept_language(value: &str) -> Option<Lang> {
        let mut ranges: Vec<(u32, usize, &str)> = value
            .split(',')
            .enumerate()
            .filter_map(|(index, item)| {
                let mut parts = item.split(';');
                let range = parts.next()?.trim();
                if range.is_empty() {
                    return None;
                }
                let quality = parts
                    .filter_map(|param| param.trim().split_once('='))
                    .find(|(name, _)| name.trim().eq_ignore_ascii_case("q"))
                    .map(|(_, q)| parse_quality(q.trim()))
                    .unwrap_or(1000);
                (quality > 0).then_some((quality, index, range))
            })
            .collect();
        ranges.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ranges
            .into_iter()
            .find_map(|(_, _, range)| Lang::match_range(range))
    }

    /// The language for a request: cookie, then `Accept-Language`, then
    /// [`Lang::DEFAULT`].
    pub fn negotiate(headers: &HeaderMap) -> Lang {
        Lang::from_cookie(headers)
            .or_else(|| {
                headers
                    .get_all(http::header::ACCEPT_LANGUAGE)
                    .iter()
                    .filter_map(|value| value.to_str().ok())
                    .find_map(Lang::from_accept_language)
            })
            .unwrap_or(Lang::DEFAULT)
    }
}

/// Parse a q-value (`0`, `1`, `0.8`, `.5`) into thousandths; malformed values
/// count as `0` and anything above `1` is clamped.
fn parse_quality(q: &str) -> u32 {
    let (int, frac) = match q.split_once('.') {
        Some((int, frac)) => (int, frac),
        None => (q, ""),
    };
    let int: u32 = match int {
        "" => 0,
        digits if digits.bytes().all(|b| b.is_ascii_digit()) => digits.parse().unwrap_or(0),
        _ => return 0,
    };
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    let mut thousandths = 0;
    for (place, digit) in frac.bytes().take(3).enumerate() {
        thousandths += u32::from(digit - b'0') * 10u32.pow(2 - place as u32);
    }
    (int * 1000 + thousandths).min(1000)
}

/// The language of the current request. Cheap and deterministic, so pages
/// simply call it once and hand the result to their components.
pub fn lang(cx: &Cx) -> Lang {
    Lang::negotiate(headers(cx))
}

/// The `Set-Cookie` value that pins `lang` for a year. `HttpOnly` because no
/// script needs it; no `Secure`, since GitCoat speaks plain HTTP behind a
/// TLS-terminating proxy (see the README).
pub fn cookie_value(lang: Lang) -> String {
    format!(
        "{COOKIE_NAME}={}; Path=/; Max-Age={COOKIE_MAX_AGE}; SameSite=Lax; HttpOnly",
        lang.tag()
    )
}

/// Every translation key defined for `lang`'s locale file (flattened,
/// e.g. `nav.code`), straight from the compiled-in backend.
pub fn message_keys(lang: Lang) -> Vec<String> {
    crate::_rust_i18n_backend()
        .messages_for_locale(lang.tag())
        .unwrap_or_default()
        .into_iter()
        .map(|(key, _)| key.into_owned())
        .collect()
}

/// Translate `key` for `lang` *without* the English fallback, or `None`
/// when the locale file lacks the key.
pub fn lookup(lang: Lang, key: &str) -> Option<String> {
    crate::_rust_i18n_backend()
        .translate(lang.tag(), key)
        .map(|value| value.into_owned())
}

/// `tr!(lang, "key")` / `tr!(lang, "key", name = value, ...)`: the text for
/// `key` in `lang`, falling back to English and then to the key itself.
macro_rules! tr {
    ($lang:expr, $key:literal $(, $name:ident = $value:expr)* $(,)?) => {
        ::rust_i18n::t!($key, locale = $lang.tag() $(, $name = $value)*).into_owned()
    };
}
pub(crate) use tr;

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        map
    }

    #[test]
    fn tags_round_trip() {
        for lang in Lang::all() {
            assert_eq!(Lang::from_tag(lang.tag()), Some(*lang));
        }
        assert_eq!(Lang::from_tag("ZH-cn"), Some(Lang::ZhCn));
        assert_eq!(Lang::from_tag("xx"), None);
        assert_eq!(Lang::from_tag(""), None);
    }

    #[test]
    fn ranges() {
        assert_eq!(Lang::match_range("en"), Some(Lang::En));
        assert_eq!(Lang::match_range("en-GB"), Some(Lang::En));
        assert_eq!(Lang::match_range("zh"), Some(Lang::ZhCn));
        assert_eq!(Lang::match_range("zh-CN"), Some(Lang::ZhCn));
        assert_eq!(Lang::match_range("zh-SG"), Some(Lang::ZhCn));
        assert_eq!(Lang::match_range("zh-Hans"), Some(Lang::ZhCn));
        assert_eq!(Lang::match_range("zh-Hans-TW"), Some(Lang::ZhCn));
        assert_eq!(Lang::match_range("zh-TW"), None);
        assert_eq!(Lang::match_range("zh-HK"), None);
        assert_eq!(Lang::match_range("zh-Hant"), None);
        assert_eq!(Lang::match_range("zh-Hant-CN"), None);
        assert_eq!(Lang::match_range("*"), None);
        assert_eq!(Lang::match_range("fr"), None);
    }

    #[test]
    fn quality_values() {
        assert_eq!(parse_quality("1"), 1000);
        assert_eq!(parse_quality("1.0"), 1000);
        assert_eq!(parse_quality("0.8"), 800);
        assert_eq!(parse_quality("0.85"), 850);
        assert_eq!(parse_quality(".5"), 500);
        assert_eq!(parse_quality("0"), 0);
        assert_eq!(parse_quality("0.0"), 0);
        assert_eq!(parse_quality("abc"), 0);
        assert_eq!(parse_quality("7"), 1000);
    }

    #[test]
    fn accept_language_negotiation() {
        assert_eq!(
            Lang::from_accept_language("zh-CN,zh;q=0.9,en;q=0.8"),
            Some(Lang::ZhCn)
        );
        assert_eq!(
            Lang::from_accept_language("en-US,en;q=0.9,zh;q=0.5"),
            Some(Lang::En)
        );
        // q-values reorder: a later range with a higher q wins.
        assert_eq!(
            Lang::from_accept_language("en;q=0.5, zh-CN;q=0.9"),
            Some(Lang::ZhCn)
        );
        // Unsupported ranges are skipped, q=0 ranges are dropped.
        assert_eq!(
            Lang::from_accept_language("zh-TW, fr;q=0.9, zh;q=0.8"),
            Some(Lang::ZhCn)
        );
        assert_eq!(
            Lang::from_accept_language("zh-CN;q=0, en;q=0.1"),
            Some(Lang::En)
        );
        assert_eq!(Lang::from_accept_language("zh-TW"), None);
        assert_eq!(Lang::from_accept_language("fr, *"), None);
        assert_eq!(Lang::from_accept_language(""), None);
        assert_eq!(Lang::from_accept_language(",;q=,"), None);
    }

    #[test]
    fn request_negotiation() {
        assert_eq!(Lang::negotiate(&headers(&[])), Lang::En);
        assert_eq!(
            Lang::negotiate(&headers(&[("accept-language", "zh-CN")])),
            Lang::ZhCn
        );
        assert_eq!(
            Lang::negotiate(&headers(&[
                ("accept-language", "en"),
                ("cookie", "other=1; gitcoat_lang=zh-CN")
            ])),
            Lang::ZhCn
        );
        assert_eq!(
            Lang::negotiate(&headers(&[
                ("accept-language", "zh-CN"),
                ("cookie", "gitcoat_lang=xx")
            ])),
            Lang::ZhCn
        );
        assert_eq!(
            Lang::negotiate(&headers(&[
                ("cookie", "a=b"),
                ("cookie", "gitcoat_lang=en"),
                ("accept-language", "zh-CN")
            ])),
            Lang::En
        );
    }

    #[test]
    fn cookie_attributes() {
        assert_eq!(
            cookie_value(Lang::ZhCn),
            "gitcoat_lang=zh-CN; Path=/; Max-Age=31536000; SameSite=Lax; HttpOnly"
        );
    }

    #[test]
    fn translation_helpers() {
        assert_eq!(tr!(Lang::En, "nav.code"), "Code");
        assert_eq!(tr!(Lang::ZhCn, "nav.code"), "代码");
        assert_eq!(tr!(Lang::En, "commits.page", page = 3), "Page 3");
        assert_eq!(tr!(Lang::ZhCn, "commits.page", page = 3), "第 3 页");
        assert_eq!(lookup(Lang::ZhCn, "nav.code").as_deref(), Some("代码"));
        assert_eq!(lookup(Lang::ZhCn, "does.not.exist"), None);
        assert!(message_keys(Lang::En).contains(&"nav.code".to_owned()));
    }
}
