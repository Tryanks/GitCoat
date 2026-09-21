//! The branch/tag dropdown shared by the tree and commits pages.

use topcoat::{
    Result,
    view::{View, component, view},
};

use super::components::{icon_branch, icon_chevron, icon_commit, icon_tag};
use crate::{
    git::{RefKind, RefList, ResolvedRef},
    l10n::{Lang, tr},
};

/// `<details>` dropdown listing branches and tags. `link` builds the URL an
/// item points at from its full ref name, so each page decides where a
/// switch leads (same path on the tree page, first history page on commits).
///
/// The script in `static/app.js` relies on this markup: one
/// `.ref-picker__group[data-kind]` per list, `.ref-picker__item` entries and
/// a `.ref-picker__empty--filter` message for a search without matches.
#[component]
pub async fn ref_picker<F>(
    lang: Lang,
    refs: &RefList,
    resolved: &ResolvedRef,
    link: F,
) -> Result<impl View>
where
    F: Fn(&str) -> String + Send + Sync,
{
    let show_tags_first = resolved.kind == RefKind::Tag;
    Ok(view! {
        <details class="ref-picker">
            <summary
                class="btn ref-picker__summary"
                aria-label=(tr!(lang, "refs.switch"))
            >
                match resolved.kind {
                    RefKind::Branch => icon_branch(),
                    RefKind::Tag => icon_tag(),
                    RefKind::Commit => icon_commit(),
                }
                <span class="ref-picker__name truncate">(resolved.short.clone())</span>
                icon_chevron()
            </summary>
            <div class="ref-picker__menu">
                <input
                    class="ref-picker__search"
                    type="search"
                    placeholder=(tr!(lang, "refs.search"))
                    aria-label=(tr!(lang, "refs.search"))
                    autocomplete="off"
                >
                <div class="ref-picker__tabs">
                    <button
                        type="button"
                        data-kind="branch"
                        aria-pressed=(if show_tags_first { "false" } else { "true" })
                    >
                        (tr!(lang, "refs.branches"))
                    </button>
                    <button
                        type="button"
                        data-kind="tag"
                        aria-pressed=(if show_tags_first { "true" } else { "false" })
                    >
                        (tr!(lang, "refs.tags"))
                    </button>
                </div>
                <div
                    class="ref-picker__group"
                    data-kind="branch"
                    hidden=(show_tags_first)
                >
                    for branch in &refs.branches {
                        <a
                            class="ref-picker__item"
                            data-kind="branch"
                            href=(link(&branch.name))
                            aria-selected=(if branch.name == resolved.name {
                                "true"
                            } else {
                                "false"
                            })
                        >
                            (branch.short.clone())
                        </a>
                    }
                    if refs.branches.is_empty() {
                        <div class="ref-picker__empty">
                            (tr!(lang, "refs.no_branches"))
                        </div>
                    }
                </div>
                <div
                    class="ref-picker__group"
                    data-kind="tag"
                    hidden=(!show_tags_first)
                >
                    for tag in &refs.tags {
                        <a
                            class="ref-picker__item"
                            data-kind="tag"
                            href=(link(&tag.name))
                            aria-selected=(if tag.name == resolved.name {
                                "true"
                            } else {
                                "false"
                            })
                        >
                            (tag.short.clone())
                        </a>
                    }
                    if refs.tags.is_empty() {
                        <div class="ref-picker__empty">(tr!(lang, "refs.no_tags"))</div>
                    }
                </div>
                <div class="ref-picker__empty ref-picker__empty--filter" hidden="">
                    (tr!(lang, "refs.no_match"))
                </div>
            </div>
        </details>
    })
}
