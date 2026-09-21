//! The `gitcoat` binary: parse configuration, open the repository and serve.
//! The stylesheet and script are compiled in, so the executable is
//! self-contained.

use std::{process::ExitCode, sync::Arc};

use gitcoat::{
    app::{AppState, router},
    config::Config,
    git::Repo,
};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let config = Config::from_args();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    match run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run(config: Config) -> Result<(), String> {
    let repo = Repo::open(&config.repo).map_err(|e| e.to_string())?;
    tracing::info!(
        path = %repo.path().display(),
        bare = repo.is_bare(),
        format = repo.object_format().as_str(),
        "opened repository"
    );

    let listener = TcpListener::bind(config.bind).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            format!("cannot listen on {}: address already in use", config.bind)
        } else {
            format!("cannot listen on {}: {e}", config.bind)
        }
    })?;
    let bind = listener.local_addr().unwrap_or(config.bind);
    tracing::info!("listening on http://{bind}");

    let state = AppState {
        config,
        repo: Arc::new(repo),
    };
    topcoat::serve(listener, router(state))
        .await
        .map_err(|e| format!("server error: {e}"))?;
    tracing::info!("shut down");
    Ok(())
}
