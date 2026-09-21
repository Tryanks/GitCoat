//! The web application: router, shared state and pages.

use std::sync::Arc;

use topcoat::{
    asset::{AssetConfig, RouterBuilderAssetExt},
    router::{
        Router,
        error::{bad_request, not_found},
    },
};

use crate::{
    config::Config,
    git::{GitError, Repo},
};

pub mod blob;
pub mod commit;
pub mod commits;
pub mod components;
pub mod healthz;
pub mod home;
pub mod lang;
pub mod layout;
pub mod raw;
pub mod refpicker;
pub mod tree;
pub mod url;

/// Application-wide state shared with every request through the app context.
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub repo: Arc<Repo>,
}

/// Build the router with every page and route registered by hand.
///
/// `assets` is the bundle produced by `topcoat asset bundle` for this very
/// build (rendering an asset that is not in the bundle panics).
pub fn router(state: AppState, assets: impl Into<AssetConfig>) -> Router {
    Router::builder()
        .layout(layout::root_layout)
        .page(home::home)
        .page(tree::tree)
        .page(commits::commits)
        .page(commit::commit)
        .page(blob::blob)
        .page(layout::not_found)
        .route(healthz::healthz)
        .route(lang::lang)
        .route(raw::raw)
        .assets(assets)
        .app_context(state)
        .build()
}

/// Map a Git access failure to the response the layout renders: not found →
/// 404, invalid input → 400, everything else → 500 (logged).
pub fn git_error(error: GitError) -> topcoat::Error {
    match error {
        GitError::NotFound(_) => not_found().into(),
        GitError::InvalidInput(message) => bad_request(message).into(),
        other => {
            tracing::error!(error = %other, "repository operation failed");
            other.into()
        }
    }
}
