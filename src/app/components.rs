//! Small shared view components: inline icons, copy button, time and size
//! formatting.

use std::time::{SystemTime, UNIX_EPOCH};

use jiff::{
    Timestamp as JiffTimestamp,
    tz::{Offset, TimeZone},
};
use topcoat::{
    Result,
    view::{View, component, view},
};

use crate::{
    git::Timestamp,
    l10n::{Lang, tr},
};

// --- Icons ------------------------------------------------------------------
//
// Every icon is a 16px `currentColor` outline drawn with simple shapes. The
// same drawings live in `static/icons/*.svg`.

macro_rules! icon {
    ($(#[$meta:meta])* $name:ident, $body:expr) => {
        $(#[$meta])*
        #[component]
        pub async fn $name() -> Result<impl View> {
            Ok(view! {
                <svg
                    class="icon"
                    width="16"
                    height="16"
                    viewBox="0 0 16 16"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.5"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                    focusable="false"
                >
                    <path d=$body></path>
                </svg>
            })
        }
    };
}

icon!(
    /// A folder.
    icon_folder,
    "M1.5 3.5a1 1 0 0 1 1-1h3.2l1.6 1.6h6.2a1 1 0 0 1 1 1v7.4a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z"
);
icon!(
    /// A plain file.
    icon_file,
    "M3.5 1.5h5.6l3.4 3.4v9.6h-9zM9 1.5v3.5h3.5"
);
icon!(
    /// A symbolic link (file with an arrow).
    icon_symlink,
    "M3.5 1.5h5.6l3.4 3.4v9.6h-9zM9 1.5v3.5h3.5M6 11.5l3-3M6.5 8.5H9v2.5"
);
icon!(
    /// A submodule (nested box).
    icon_submodule,
    "M2 2h12v12H2zM5.5 5.5h5v5h-5z"
);
icon!(
    /// A branch.
    icon_branch,
    "M4.5 3a1.5 1.5 0 1 0 0 .01M4.5 13a1.5 1.5 0 1 0 0 .01M11.5 4a1.5 1.5 0 1 0 0 .01M4.5 4.5v7M11.5 5.5c0 3-7 2-7 6"
);
icon!(
    /// A tag.
    icon_tag,
    "M1.5 2.5v5l7 7 6-6-7-7h-5zM5 5a.5.5 0 1 0 0 .01"
);
icon!(
    /// A commit (dot on a line).
    icon_commit,
    "M8 5.5a2.5 2.5 0 1 0 0 5a2.5 2.5 0 1 0 0-5M1.5 8h4M10.5 8h4"
);
icon!(
    /// Two overlapping sheets, for copy buttons.
    icon_copy,
    "M5.5 5.5h8v8h-8zM2.5 10.5v-8h8"
);
icon!(
    /// A check mark.
    icon_check,
    "M2.5 8.5l3.5 3.5 7.5-8"
);
icon!(
    /// The sun, for the light theme.
    icon_sun,
    "M8 5a3 3 0 1 0 0 6a3 3 0 1 0 0-6M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1 1M11.6 11.6l1 1M3.4 12.6l1-1M11.6 4.4l1-1"
);
icon!(
    /// The moon, for the dark theme.
    icon_moon,
    "M13.5 9.5A5.5 5.5 0 0 1 6.5 2.5a5.5 5.5 0 1 0 7 7z"
);
icon!(
    /// A repository (book).
    icon_repo,
    "M3 1.5h10v11H4.5a1.5 1.5 0 0 0 0 3H13M3 1.5v12a1.5 1.5 0 0 0 1.5 1.5M6 4.5h4"
);
icon!(
    /// A small downward chevron.
    icon_chevron,
    "M4 6l4 4 4-4"
);

// --- Copy button ------------------------------------------------------------

/// What a copy button puts on the clipboard.
#[derive(Debug, Clone, Copy)]
pub enum CopySource<'a> {
    /// A literal string.
    Text(&'a str),
    /// A root-relative URL; the script prefixes the page origin at click time.
    Href(&'a str),
}

/// A button that copies `source` to the clipboard; `label` is the visible
/// text (replaced by "Copied" for a moment on success, then restored). The
/// read-only input is the fallback the script reveals when the clipboard API
/// is unavailable. The localised script strings travel as `data-*`
/// attributes.
#[component]
pub async fn copy_button(lang: Lang, source: CopySource<'_>, label: &str) -> Result<impl View> {
    let (text, href) = match source {
        CopySource::Text(text) => (Some(text), None),
        CopySource::Href(href) => (None, Some(href)),
    };
    let value = text.or(href).unwrap_or_default();
    Ok(view! {
        <span class="copy-wrap">
            <button
                type="button"
                class="btn btn--small copy"
                data-copy=(text)
                data-copy-href=(href)
                data-label=(label)
                data-copied-label=(tr!(lang, "copy.copied"))
                data-fallback-label-mac=(tr!(lang, "copy.fallback_mac"))
                data-fallback-label=(tr!(lang, "copy.fallback_other"))
                title=(label)
            >
                icon_copy()
                <span class="copy__feedback">(label)</span>
            </button>
            <input
                class="copy__fallback"
                type="text"
                readonly=""
                value=(value)
                tabindex="-1"
                aria-hidden="true"
                hidden=""
            >
        </span>
    })
}

// --- Object ids -------------------------------------------------------------

/// The abbreviated form of an object id.
pub fn oid_short(oid: &str) -> &str {
    &oid[..7.min(oid.len())]
}

// --- Time -------------------------------------------------------------------

/// Current Unix time in seconds.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Human-friendly distance from `now` to `unix` ("3 hours ago") in `lang`.
///
/// `rust_i18n` has no plural rules, so each unit has a `.one` and a `.other`
/// key and the choice is made here (`zh-CN` simply carries the same text in
/// both).
pub fn relative_time(lang: Lang, unix: i64, now: i64) -> String {
    let delta = now.saturating_sub(unix);
    if delta < 45 {
        return tr!(lang, "time.just_now");
    }
    let (count, unit) = if delta < 90 * 60 {
        (delta.div_euclid(60).max(1), "minutes")
    } else if delta < 36 * 3600 {
        (delta.div_euclid(3600).max(1), "hours")
    } else if delta < 14 * 86_400 {
        (delta.div_euclid(86_400), "days")
    } else if delta < 60 * 86_400 {
        (delta.div_euclid(7 * 86_400), "weeks")
    } else if delta < 2 * 365 * 86_400 {
        (delta.div_euclid(30 * 86_400).max(2), "months")
    } else {
        (delta.div_euclid(365 * 86_400), "years")
    };
    let form = if count == 1 { "one" } else { "other" };
    // The key is assembled at runtime, so `t!`'s literal-key form is bypassed.
    let key = format!("time.{unit}_ago.{form}");
    rust_i18n::t!(&key, locale = lang.tag(), count = count).into_owned()
}

/// Format a timestamp in its own UTC offset with a `strftime` pattern.
fn format_timestamp(ts: Timestamp, pattern: &str) -> Option<String> {
    let offset = Offset::from_seconds(ts.tz_offset_minutes.checked_mul(60)?).ok()?;
    let zoned = JiffTimestamp::from_second(ts.unix)
        .ok()?
        .to_zoned(TimeZone::fixed(offset));
    Some(zoned.strftime(pattern).to_string())
}

/// RFC 3339 form for `<time datetime>`, e.g. `2024-01-02T03:04:05+02:00`.
pub fn rfc3339(ts: Timestamp) -> String {
    format_timestamp(ts, "%Y-%m-%dT%H:%M:%S%:z").unwrap_or_else(|| ts.unix.to_string())
}

/// Readable absolute form, e.g. `2024-01-02 03:04:05 +02:00`.
pub fn absolute_time(ts: Timestamp) -> String {
    format_timestamp(ts, "%Y-%m-%d %H:%M:%S %:z").unwrap_or_else(|| ts.unix.to_string())
}

/// `<time>` element showing a relative time with the absolute time as title.
#[component]
pub async fn time_ago(lang: Lang, ts: Timestamp) -> Result<impl View> {
    let relative = relative_time(lang, ts.unix, now_unix());
    Ok(view! { <time datetime=(rfc3339(ts)) title=(absolute_time(ts))>(relative)</time> })
}

// --- Sizes ------------------------------------------------------------------

/// Human-readable size with binary units (`1.5 KiB`), exact bytes below 1 KiB.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(150 * 1024), "150 KiB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MiB");
    }

    #[test]
    fn relative_times() {
        let now = 1_700_000_000;
        let en = |unix| relative_time(Lang::En, unix, now);
        assert_eq!(en(now), "just now");
        assert_eq!(en(now + 100), "just now");
        assert_eq!(en(now - 60), "1 minute ago");
        assert_eq!(en(now - 5 * 60), "5 minutes ago");
        assert_eq!(en(now - 3 * 3600), "3 hours ago");
        assert_eq!(en(now - 2 * 86_400), "2 days ago");
        assert_eq!(en(now - 21 * 86_400), "3 weeks ago");
        assert_eq!(en(now - 100 * 86_400), "3 months ago");
        assert_eq!(en(now - 800 * 86_400), "2 years ago");
        let zh = |unix| relative_time(Lang::ZhCn, unix, now);
        assert_eq!(zh(now), "刚刚");
        assert_eq!(zh(now - 60), "1 分钟前");
        assert_eq!(zh(now - 5 * 60), "5 分钟前");
        assert_eq!(zh(now - 3 * 3600), "3 小时前");
        assert_eq!(zh(now - 2 * 86_400), "2 天前");
        assert_eq!(zh(now - 21 * 86_400), "3 周前");
        assert_eq!(zh(now - 100 * 86_400), "3 个月前");
        assert_eq!(zh(now - 800 * 86_400), "2 年前");
    }

    #[test]
    fn absolute_times() {
        let ts = Timestamp {
            unix: 1_704_164_645,
            tz_offset_minutes: 0,
        };
        assert_eq!(rfc3339(ts), "2024-01-02T03:04:05+00:00");
        let ts = Timestamp {
            unix: 1_704_164_645,
            tz_offset_minutes: 120,
        };
        assert_eq!(rfc3339(ts), "2024-01-02T05:04:05+02:00");
        let ts = Timestamp {
            unix: 1_704_164_645,
            tz_offset_minutes: -330,
        };
        assert_eq!(rfc3339(ts), "2024-01-01T21:34:05-05:30");
        assert_eq!(absolute_time(ts), "2024-01-01 21:34:05 -05:30");
        assert_eq!(
            rfc3339(Timestamp {
                unix: -1,
                tz_offset_minutes: 0
            }),
            "1969-12-31T23:59:59+00:00"
        );
        // Out-of-range values degrade to the raw seconds instead of panicking.
        assert_eq!(
            rfc3339(Timestamp {
                unix: i64::MAX,
                tz_offset_minutes: 0
            }),
            i64::MAX.to_string()
        );
    }

    #[test]
    fn short_oid() {
        assert_eq!(oid_short("0123456789abcdef"), "0123456");
        assert_eq!(oid_short("abc"), "abc");
    }
}
