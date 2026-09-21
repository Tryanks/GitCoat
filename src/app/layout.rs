//! The root layout: document shell, top bar, tabs, language switcher, footer
//! and the branded error pages.

use topcoat::{
    Result,
    asset::{Asset, asset},
    context::{Cx, app_context},
    router::{
        HeaderValue, Slot, StatusCode,
        error::{BadRequestError, NotFoundError},
        header::VARY,
        layout, not_found,
        request::uri,
    },
    view::{Unescaped, View, component, error_boundary, view},
};

use super::{
    AppState,
    components::{icon_moon, icon_repo, icon_sun},
    url::{commits_url, lang_url, parse_query},
};
use crate::l10n::{Lang, lang, tr};

/// The stylesheet (written by the styling worker).
pub const APP_CSS: Asset = asset!("../../static/app.css");
/// The tiny script for the theme toggle, copy buttons and the ref picker.
pub const APP_JS: Asset = asset!("../../static/app.js");

/// Applies the stored theme before first paint so there is no flash. Kept
/// free of `<`, `>` and `&` so it needs no escaping considerations.
const THEME_SCRIPT: &str = "(function(){try{var t=localStorage.getItem('gitcoat-theme');\
if(t==='dark'||t==='light'){document.documentElement.setAttribute('data-theme',t)}}catch(e){}})();";

/// Pages differ by cookie and `Accept-Language`, so caches must key on both.
const VARY_VALUE: HeaderValue = HeaderValue::from_static("Cookie, Accept-Language");

// Catch-all so unknown URLs render the branded 404 through the layout.
not_found!("/");

#[layout("/")]
pub async fn root_layout(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    let state = app_context::<AppState>(cx);
    let lang = lang(cx);
    let name = state.config.repo_name();
    let description = state
        .config
        .description
        .clone()
        .filter(|d| !d.trim().is_empty());
    let path = uri(cx).path().to_owned();
    let query = uri(cx).query().unwrap_or("");
    let current_ref = parse_query(query)
        .into_iter()
        .find(|(key, _)| key == "ref")
        .map(|(_, value)| value)
        .unwrap_or_default();
    let commits_href = commits_url(&current_ref);
    let commits_active = path == "/commits" || path.starts_with("/commit/");
    let code_active = !commits_active;
    let title = tr!(lang, "layout.title", name = name);
    // Where the language switcher sends the browser back to.
    let back = if query.is_empty() {
        path.clone()
    } else {
        format!("{path}?{query}")
    };

    let vary = (VARY, VARY_VALUE);

    Ok(view! {
        (vary)
        <!DOCTYPE html>
        <html lang=(lang.tag())>
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1">
                <title>(title)</title>
                <link rel="stylesheet" href=(APP_CSS)>
                <script>(Unescaped::new_unchecked(THEME_SCRIPT))</script>
                <script src=(APP_JS) defer=""></script>
            </head>
            <body>
                <header class="topbar">
                    <div class="topbar__inner container">
                        <a class="repo-title" href="/">
                            <span class="repo-title__icon">icon_repo()</span>
                            <span class="repo-title__name">(name)</span>
                        </a>
                        if let Some(description) = description {
                            <span class="repo-desc muted">(description)</span>
                        }
                        <nav class="tabs" aria-label=(tr!(lang, "nav.label"))>
                            <a
                                class="tab"
                                href="/"
                                aria-current=(code_active.then_some("page"))
                            >
                                (tr!(lang, "nav.code"))
                            </a>
                            <a
                                class="tab"
                                href=(commits_href)
                                aria-current=(commits_active.then_some("page"))
                            >
                                (tr!(lang, "nav.commits"))
                            </a>
                        </nav>
                        <div class="topbar__actions">
                            lang_switcher(lang: lang, back: &back)
                            <button
                                type="button"
                                class="btn btn--icon theme-toggle"
                                aria-label=(tr!(lang, "theme.toggle"))
                                title=(tr!(lang, "theme.toggle"))
                                data-label-dark=(tr!(lang, "theme.to_dark"))
                                data-label-light=(tr!(lang, "theme.to_light"))
                            >
                                icon_sun()
                                icon_moon()
                            </button>
                        </div>
                    </div>
                </header>
                <main class="container">
                    error_boundary(
                        fallback: |error| {
                            let (status, heading, message, detail) = if error
                                .downcast_ref::<NotFoundError>()
                                .is_some() {
                                (
                                    StatusCode::NOT_FOUND,
                                    tr!(lang, "error.not_found.title"),
                                    tr!(lang, "error.not_found.text"),
                                    None,
                                )
                            } else if let Some(bad) = error.downcast_ref::<
                                BadRequestError,
                            >() {
                                (
                                    StatusCode::BAD_REQUEST,
                                    tr!(lang, "error.bad_request.title"),
                                    tr!(lang, "error.bad_request.text"),
                                    Some(bad.description().to_owned()),
                                )
                            } else {
                                tracing::error!(error = %error, "request failed");
                                (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    tr!(lang, "error.internal.title"),
                                    tr!(lang, "error.internal.text"),
                                    None,
                                )
                            };
                            Ok(
                                view! {
                                    error_page(
                                        lang: lang,
                                        status: status,
                                        heading: &heading,
                                        message: &message,
                                        detail: detail.as_deref()
                                    )
                                },
                            )
                        },
                        (slot)
                    )
                </main>
                <footer class="footer container">
                    <span>"GitCoat"</span>
                    " "
                    <span class="muted">(crate::VERSION)</span>
                </footer>
            </body>
        </html>
    })
}

/// Links to every language; the current one is marked with `aria-current`.
#[component]
async fn lang_switcher(lang: Lang, back: &str) -> Result<impl View> {
    Ok(view! {
        <nav class="lang-switch btn-group" aria-label=(tr!(lang, "lang.label"))>
            for item in Lang::all() {
                let item = *item;
                <a
                    class="btn btn--small"
                    href=(lang_url(item.tag(), back))
                    hreflang=(item.tag())
                    lang=(item.tag())
                    rel="alternate"
                    aria-current=((item == lang).then_some("true"))
                >
                    (item.native_name())
                </a>
            }
        </nav>
    })
}

/// A branded error page; sets the response status. `detail` is the technical
/// reason (from the request parser or the Git layer), shown verbatim.
#[component]
pub async fn error_page(
    lang: Lang,
    status: StatusCode,
    heading: &str,
    message: &str,
    detail: Option<&str>,
) -> Result<impl View> {
    Ok(view! {
        (status)
        <section class="error-page">
            <div class="error-page__code">(status.as_u16())</div>
            <h1>(heading)</h1>
            <p>(message)</p>
            if let Some(detail) = detail {
                <p class="error-page__detail muted">
                    <code class="mono">(detail)</code>
                </p>
            }
            <div class="error-page__actions">
                <a class="btn" href="/">(tr!(lang, "error.back_home"))</a>
            </div>
        </section>
    })
}
