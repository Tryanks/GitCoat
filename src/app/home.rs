//! `GET /`: the tree of the default ref at the root path.

use topcoat::{Result, context::Cx, router::page, view::View};

use super::tree::render_tree;

#[page("/")]
pub async fn home(cx: &Cx) -> Result<impl View> {
    render_tree(cx, None, "").await
}
