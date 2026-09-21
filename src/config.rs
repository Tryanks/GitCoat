//! Command line and environment configuration.

use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;

/// Runtime configuration, resolved from CLI flags, environment and defaults
/// (in that priority order).
#[derive(Debug, Clone, Parser)]
#[command(
    name = "gitcoat",
    version,
    about = "Read-only web UI for a single Git repository"
)]
pub struct Config {
    /// Path to the repository (bare, or a normal checkout's root).
    #[arg(long, env = "GITCOAT_REPO", value_name = "PATH")]
    pub repo: PathBuf,

    /// Socket address to listen on.
    #[arg(
        long,
        env = "GITCOAT_BIND",
        default_value = "127.0.0.1:3000",
        value_name = "ADDR"
    )]
    pub bind: SocketAddr,

    /// Display name of the repository (defaults to the directory name).
    #[arg(long, env = "GITCOAT_NAME", value_name = "NAME")]
    pub name: Option<String>,

    /// Short description shown in the header.
    #[arg(long, env = "GITCOAT_DESCRIPTION", value_name = "TEXT")]
    pub description: Option<String>,

    /// Clone URL offered to visitors (shown with a copy button).
    #[arg(long, env = "GITCOAT_CLONE_URL", value_name = "URL")]
    pub clone_url: Option<String>,
}

impl Config {
    /// Parse the process arguments and environment.
    pub fn from_args() -> Self {
        Self::parse()
    }

    /// The repository name shown in the UI: `--name`, or the directory name
    /// with a trailing `.git` removed.
    pub fn repo_name(&self) -> String {
        if let Some(name) = &self.name
            && !name.trim().is_empty()
        {
            return name.trim().to_owned();
        }
        default_name(&self.repo)
    }
}

fn default_name(repo: &std::path::Path) -> String {
    let file_name = repo
        .components()
        .rev()
        .find_map(|component| match component {
            std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .unwrap_or_else(|| "repository".to_owned());
    let stripped = file_name.strip_suffix(".git").unwrap_or(&file_name);
    if stripped.is_empty() {
        file_name
    } else {
        stripped.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Config {
        Config::try_parse_from(std::iter::once("gitcoat").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn name_defaults_to_directory_without_git_suffix() {
        assert_eq!(
            parse(&["--repo", "/srv/git/hello.git"]).repo_name(),
            "hello"
        );
        assert_eq!(
            parse(&["--repo", "/srv/git/hello.git/"]).repo_name(),
            "hello"
        );
        assert_eq!(parse(&["--repo", "/srv/git/hello"]).repo_name(), "hello");
        assert_eq!(parse(&["--repo", "/srv/git/.git"]).repo_name(), ".git");
    }

    #[test]
    fn explicit_name_wins() {
        assert_eq!(
            parse(&["--repo", "/x/y.git", "--name", "Custom"]).repo_name(),
            "Custom"
        );
    }

    #[test]
    fn bind_defaults_to_localhost() {
        let config = parse(&["--repo", "/x"]);
        assert_eq!(config.bind, "127.0.0.1:3000".parse::<SocketAddr>().unwrap());
        assert_eq!(
            parse(&["--repo", "/x", "--bind", "0.0.0.0:8080"])
                .bind
                .port(),
            8080
        );
    }

    #[test]
    fn repo_is_required() {
        assert!(Config::try_parse_from(["gitcoat"]).is_err());
    }
}
