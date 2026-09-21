//! UI localisation: language negotiation, the `/lang` cookie endpoint, the
//! switcher markup, localised pages and translation-file parity.

mod common;

use std::collections::BTreeSet;

use common::{fixtures::TempRepo, get, get_with_headers, test_app};
use gitcoat::{
    app::url::lang_url,
    l10n::{Lang, lookup, message_keys},
};
use http::StatusCode;

const ZH: (&str, &str) = ("accept-language", "zh-CN,zh;q=0.9,en;q=0.8");

/// Extract the value of the `<html lang="...">` attribute.
fn html_lang(html: &str) -> &str {
    let start = html.find("<html lang=\"").expect("html element") + "<html lang=\"".len();
    let end = html[start..].find('"').unwrap() + start;
    &html[start..end]
}

#[tokio::test]
async fn default_is_english() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let home = get(&app, "/").await;
    assert_eq!(home.status, StatusCode::OK);
    assert_eq!(html_lang(&home.body), "en");
    assert!(home.body.contains(">Commits</a>"), "English tab");
    assert!(home.body.contains(">Code</a>"), "English tab");
    assert!(home.body.contains("<title>repo · GitCoat</title>"));
}

#[tokio::test]
async fn accept_language_selects_chinese() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let home = get_with_headers(&app, "/", &[ZH]).await;
    assert_eq!(home.status, StatusCode::OK);
    assert_eq!(html_lang(&home.body), "zh-CN");
    assert!(
        home.body.contains(">提交</a>"),
        "Chinese Commits tab: {}",
        home.body
    );
    assert!(home.body.contains(">代码</a>"), "Chinese Code tab");
    assert!(
        home.body.contains("aria-label=\"切换主题\""),
        "theme toggle label"
    );
    assert!(
        home.body.contains("data-label-dark=\"切换到深色主题\""),
        "script labels exposed via data attributes"
    );
    assert!(
        home.body.contains("placeholder=\"查找分支或标签\""),
        "ref picker placeholder"
    );
    assert!(home.body.contains(">复制克隆地址</span>"), "copy label");
    assert!(
        home.body.contains("data-copied-label=\"已复制\""),
        "copy feedback label"
    );
    // Git data stays verbatim.
    assert!(home.body.contains("GitCoat fixture"), "README content");
    assert!(
        home.body.contains("class=\"repo-title__name\">repo<"),
        "repo name"
    );
}

#[tokio::test]
async fn accept_language_prefers_english_when_ranked_higher() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let home = get_with_headers(&app, "/", &[("accept-language", "en-US,en;q=0.9,zh;q=0.5")]).await;
    assert_eq!(html_lang(&home.body), "en");
    // A later range with a higher q-value wins over an earlier one.
    let home = get_with_headers(&app, "/", &[("accept-language", "en;q=0.3, zh-CN;q=0.9")]).await;
    assert_eq!(html_lang(&home.body), "zh-CN");
}

#[tokio::test]
async fn traditional_chinese_alone_falls_back_to_english() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    for value in ["zh-TW", "zh-HK,zh-Hant;q=0.9", "zh-Hant-TW"] {
        let home = get_with_headers(&app, "/", &[("accept-language", value)]).await;
        assert_eq!(html_lang(&home.body), "en", "{value}");
    }
    // ...but a generic `zh` further down the list still gets Simplified Chinese.
    let home = get_with_headers(&app, "/", &[("accept-language", "zh-TW,zh;q=0.8")]).await;
    assert_eq!(html_lang(&home.body), "zh-CN");
}

#[tokio::test]
async fn cookie_overrides_accept_language() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let home = get_with_headers(
        &app,
        "/",
        &[("accept-language", "en"), ("cookie", "gitcoat_lang=zh-CN")],
    )
    .await;
    assert_eq!(html_lang(&home.body), "zh-CN");
    let home = get_with_headers(&app, "/", &[ZH, ("cookie", "a=1; gitcoat_lang=en; b=2")]).await;
    assert_eq!(html_lang(&home.body), "en");
}

#[tokio::test]
async fn invalid_cookie_falls_back_to_negotiation() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let home = get_with_headers(&app, "/", &[ZH, ("cookie", "gitcoat_lang=xx")]).await;
    assert_eq!(html_lang(&home.body), "zh-CN");
    let home = get_with_headers(&app, "/", &[("cookie", "gitcoat_lang=zh-TW")]).await;
    assert_eq!(html_lang(&home.body), "en");
}

#[tokio::test]
async fn lang_endpoint_sets_cookie_and_redirects_back() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    // `back` is a query value, so the switcher encodes it once more.
    let back = "/tree?ref=refs%2Fheads%2Fmain";
    let response = get(&app, &lang_url("zh-CN", back)).await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(response.header("location"), back);
    let cookie = response.header("set-cookie");
    assert!(cookie.starts_with("gitcoat_lang=zh-CN;"), "{cookie}");
    assert!(cookie.contains("Path=/"), "{cookie}");
    assert!(cookie.contains("Max-Age=31536000"), "{cookie}");
    assert!(cookie.contains("SameSite=Lax"), "{cookie}");
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(
        !cookie.contains("Secure"),
        "plain HTTP behind the proxy: {cookie}"
    );
    assert_eq!(response.header("cache-control"), "no-store");

    // A `back` pasted without that extra encoding decodes to the equivalent URL.
    let response = get(&app, "/lang?set=zh-CN&back=/tree?ref=refs%2Fheads%2Fmain").await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(response.header("location"), "/tree?ref=refs/heads/main");

    // Switching back to English works the same way.
    let response = get(&app, "/lang?set=en&back=/commits").await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert!(
        response
            .header("set-cookie")
            .starts_with("gitcoat_lang=en;")
    );
    assert_eq!(response.header("location"), "/commits");
}

#[tokio::test]
async fn lang_endpoint_rejects_unknown_language() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    for path in ["/lang?set=xx", "/lang", "/lang?set=", "/lang?set=zh-TW"] {
        let response = get(&app, path).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(response.header("set-cookie"), "", "{path}");
    }
}

#[tokio::test]
async fn lang_endpoint_is_not_an_open_redirect() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    for back in [
        "//evil.com",
        "https://evil.com/",
        "http://evil.com",
        "/%5Cevil.com",
        "evil",
        "javascript:alert(1)",
        "",
    ] {
        let path = format!("/lang?set=zh-CN&back={back}");
        let response = get(&app, &path).await;
        assert_eq!(response.status, StatusCode::SEE_OTHER, "{path}");
        assert_eq!(response.header("location"), "/", "{path}");
    }
}

#[tokio::test]
async fn pages_vary_on_cookie_and_accept_language() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    for path in ["/", "/commits", "/blob?path=README.md", "/nope"] {
        let response = get(&app, path).await;
        let vary = response.header("vary").to_ascii_lowercase();
        assert!(vary.contains("cookie"), "{path}: Vary = {vary:?}");
        assert!(vary.contains("accept-language"), "{path}: Vary = {vary:?}");
    }
}

#[tokio::test]
async fn switcher_links_every_language_and_marks_the_current_one() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let page = get(&app, "/tree?ref=refs%2Fheads%2Fmain&path=src").await;
    let html = &page.body;
    assert!(html.contains("class=\"lang-switch btn-group\""), "switcher");
    assert!(html.contains("aria-label=\"Language\""));
    let back = "%2Ftree%3Fref%3Drefs%252Fheads%252Fmain%26path%3Dsrc";
    assert!(
        html.contains(&format!(
            "href=\"/lang?set=en&amp;back={back}\" hreflang=\"en\" lang=\"en\" rel=\"alternate\" aria-current=\"true\">English</a>"
        )),
        "current English link: {html}"
    );
    assert!(
        html.contains(&format!(
            "href=\"/lang?set=zh-CN&amp;back={back}\" hreflang=\"zh-CN\" lang=\"zh-CN\" rel=\"alternate\">简体中文</a>"
        )),
        "Chinese link: {html}"
    );
    let page = get_with_headers(&app, "/", &[ZH]).await;
    assert!(page.body.contains("aria-label=\"语言\""));
    assert!(
        page.body.contains(
            "hreflang=\"zh-CN\" lang=\"zh-CN\" rel=\"alternate\" aria-current=\"true\">简体中文</a>"
        ),
        "Chinese marked current"
    );
    assert!(
        page.body
            .contains("hreflang=\"en\" lang=\"en\" rel=\"alternate\">English</a>"),
        "English not current"
    );
    // The switcher and the theme toggle share the right-hand cluster.
    assert!(
        page.body
            .contains("<div class=\"topbar__actions\"><nav class=\"lang-switch")
    );
    assert!(page.body.contains("theme-toggle\""));
}

#[tokio::test]
async fn error_pages_are_localised() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let missing = get_with_headers(&app, "/nope", &[ZH]).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(html_lang(&missing.body), "zh-CN");
    assert!(
        missing.body.contains("<h1>页面未找到</h1>"),
        "{}",
        missing.body
    );
    assert!(missing.body.contains(">返回仓库</a>"));

    let bad = get_with_headers(&app, "/tree?path=..%2Fx", &[ZH]).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert!(bad.body.contains("<h1>请求无效</h1>"), "{}", bad.body);

    let no_path = get_with_headers(&app, "/tree?path=does/not/exist", &[ZH]).await;
    assert_eq!(no_path.status, StatusCode::NOT_FOUND);
    assert!(no_path.body.contains("<h1>未找到路径</h1>"));
    assert!(
        no_path.body.contains("main 上不存在 does/not/exist。"),
        "{}",
        no_path.body
    );
    assert!(no_path.body.contains(">浏览 main 的根目录</a>"));

    let no_ref = get_with_headers(&app, "/tree?ref=refs%2Fheads%2Fnope", &[ZH]).await;
    assert_eq!(no_ref.status, StatusCode::NOT_FOUND);
    assert!(no_ref.body.contains("<h1>未找到引用</h1>"));

    let no_file = get_with_headers(&app, "/blob?path=nope.txt", &[ZH]).await;
    assert_eq!(no_file.status, StatusCode::NOT_FOUND);
    assert!(no_file.body.contains("<h1>未找到文件</h1>"));
}

#[tokio::test]
async fn commits_page_uses_chinese_relative_times() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let page = get_with_headers(&app, "/commits", &[ZH]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("main 的提交</h1>"), "{}", page.body);
    assert!(page.body.contains(" 提交于 <time "), "committed");
    // Fixture commits are dated 2024, so the relative form is in years.
    assert!(page.body.contains(" 年前</time>"), "{}", page.body);
    assert!(
        page.body.contains("<span class=\"badge\">合并</span>"),
        "merge badge"
    );
    assert!(page.body.contains(">上一页</span>") || page.body.contains(">上一页</a>"));
    assert!(page.body.contains(">下一页</span>") || page.body.contains(">下一页</a>"));
    assert!(page.body.contains(">第 1 页</span>"));
    assert!(page.body.contains("aria-label=\"分页\""));

    let en = get(&app, "/commits").await;
    assert!(en.body.contains(" years ago</time>"), "{}", en.body);
    assert!(en.body.contains(">Page 1</span>"));
}

#[tokio::test]
async fn blob_and_commit_pages_are_localised() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let blob = get_with_headers(&app, "/blob?path=src%2Fmain.rs", &[ZH]).await;
    assert_eq!(blob.status, StatusCode::OK);
    assert!(
        blob.body.contains("<span class=\"badge\">文本</span>"),
        "{}",
        blob.body
    );
    assert!(blob.body.contains(" 行</span>"), "line count");
    assert!(blob.body.contains(">原始文件</a>"));
    assert!(blob.body.contains(">永久链接</span>"));
    assert!(blob.body.contains(">复制路径</span>"));
    assert!(blob.body.contains("aria-label=\"路径\""));

    let readme = get_with_headers(&app, "/blob?path=README.md", &[ZH]).await;
    assert!(readme.body.contains(">渲染视图</a>"));
    assert!(readme.body.contains(">源码</a>"));
    assert!(
        readme
            .body
            .contains("<span class=\"badge\">Markdown</span>")
    );

    let commit = get_with_headers(&app, &format!("/commit/{}", repo.merge_oid), &[ZH]).await;
    assert_eq!(commit.status, StatusCode::OK);
    assert!(commit.body.contains("<dt>作者</dt>"), "{}", commit.body);
    assert!(commit.body.contains("<dt>父提交</dt>"));
    assert!(commit.body.contains(">浏览文件</a>"));
    assert!(commit.body.contains(">复制完整 ID</span>"));
    assert!(
        commit
            .body
            .contains("合并提交：显示相对于第一个父提交 <span class=\"oid mono\">")
    );
    assert!(commit.body.contains("个文件已更改"), "{}", commit.body);
    assert!(commit.body.contains("<span class=\"badge\">已新增</span>"));
    assert!(commit.body.contains(">查看文件</a>"));

    let root = get_with_headers(&app, &format!("/commit/{}", repo.root_oid), &[ZH]).await;
    assert!(root.body.contains("根提交（与空树比较）"));
}

#[tokio::test]
async fn ref_picker_markup_matches_the_script() {
    // static/app.js drives the picker through these classes; keep them in sync.
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let html = get(&app, "/").await.body;
    assert!(
        html.contains("<div class=\"ref-picker__group\" data-kind=\"branch\">"),
        "{html}"
    );
    assert!(html.contains("<div class=\"ref-picker__group\" data-kind=\"tag\" hidden=\"\">"));
    assert!(
        html.contains("<div class=\"ref-picker__empty ref-picker__empty--filter\" hidden=\"\">")
    );
    assert!(html.contains("class=\"ref-picker__item\" data-kind=\"branch\""));
    assert!(html.contains("class=\"ref-picker__item\" data-kind=\"tag\""));
    assert!(html.contains("class=\"ref-picker__search\" type=\"search\""));
    assert!(html.contains("<button type=\"button\" data-kind=\"branch\" aria-pressed=\"true\">"));
    assert!(html.contains("<button type=\"button\" data-kind=\"tag\" aria-pressed=\"false\">"));
    assert!(!html.contains("ref-picker__list"), "old class name gone");
}

#[tokio::test]
async fn copy_buttons_carry_labels_for_the_script() {
    let repo = TempRepo::rich();
    let app = test_app(&repo.path);
    let blob = get(&app, "/blob?path=src%2Fmain.rs").await.body;
    // Both copy variants: literal text and origin-relative href.
    assert!(
        blob.contains("data-copy=\"src/main.rs\" data-label=\"Copy path\""),
        "{blob}"
    );
    assert!(blob.contains("data-copy-href=\"/blob?ref="), "{blob}");
    assert!(blob.contains("data-label=\"Permalink\""));
    assert!(blob.contains("data-copied-label=\"Copied\""));
    assert!(blob.contains("data-fallback-label-mac=\"Press ⌘+C\""));
    assert!(blob.contains("data-fallback-label=\"Press Ctrl+C\""));
    // Every fallback input starts hidden.
    let inputs = blob.matches("class=\"copy__fallback\"").count();
    let hidden = blob
        .matches("class=\"copy__fallback\" type=\"text\" readonly=\"\" value=\"")
        .count();
    assert!(inputs >= 2, "{inputs} fallback inputs");
    assert_eq!(inputs, hidden);
    assert_eq!(
        blob.matches("tabindex=\"-1\" aria-hidden=\"true\" hidden=\"\">")
            .count(),
        inputs,
        "every fallback input is hidden: {blob}"
    );
}

/// Every key in `en` exists in `zh-CN` and vice versa.
///
/// `t!` silently falls back to English for a missing key, so the check goes
/// straight to the compiled-in backend: `message_keys` lists the flattened
/// keys of one locale file, `lookup` reads a key without any fallback.
#[test]
fn translation_files_have_the_same_keys() {
    let en: BTreeSet<String> = message_keys(Lang::En).into_iter().collect();
    let zh: BTreeSet<String> = message_keys(Lang::ZhCn).into_iter().collect();
    assert!(en.len() > 100, "en has {} keys", en.len());
    let missing_zh: Vec<_> = en.difference(&zh).collect();
    let missing_en: Vec<_> = zh.difference(&en).collect();
    assert!(missing_zh.is_empty(), "missing in zh-CN: {missing_zh:?}");
    assert!(missing_en.is_empty(), "missing in en: {missing_en:?}");
    for key in &en {
        for lang in Lang::all() {
            let value = lookup(*lang, key).unwrap_or_else(|| panic!("{key} in {}", lang.tag()));
            assert!(!value.trim().is_empty(), "{key} is empty in {}", lang.tag());
        }
    }
    // Placeholders must agree between the two files.
    for key in &en {
        let placeholders = |lang| -> BTreeSet<String> {
            let value = lookup(lang, key).unwrap();
            value
                .match_indices("%{")
                .map(|(start, _)| {
                    let end = value[start..].find('}').unwrap() + start;
                    value[start + 2..end].to_owned()
                })
                .collect()
        };
        assert_eq!(placeholders(Lang::En), placeholders(Lang::ZhCn), "{key}");
    }
}

/// Every literal key passed to `tr!` in `src/` exists in the English file.
#[test]
fn every_key_used_in_source_exists() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let en: BTreeSet<String> = message_keys(Lang::En).into_iter().collect();
    let mut used = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                for (index, _) in source.match_indices("tr!(") {
                    let rest = &source[index + "tr!(".len()..];
                    // `tr!(lang, "key"` — the key is the first string literal.
                    let Some(quote) = rest.find('"') else {
                        continue;
                    };
                    let rest = &rest[quote + 1..];
                    let end = rest.find('"').unwrap();
                    let key = &rest[..end];
                    // Real keys are nested (`nav.code`); skip doc-comment examples.
                    if key.contains('.') {
                        used.insert(key.to_owned());
                    }
                }
            }
        }
    }
    assert!(used.len() > 50, "found {} keys", used.len());
    let unknown: Vec<_> = used.difference(&en).collect();
    assert!(unknown.is_empty(), "keys used but not defined: {unknown:?}");
}
