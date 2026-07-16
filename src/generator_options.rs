//! Shared command-line options for repo-local generators.

use std::{
    env, io,
    path::{Path, PathBuf},
    process::Command,
};

/// Parsed options for repo-local generator commands.
pub(crate) struct RepoToolOptions {
    /// Repository root the generator should inspect.
    pub(crate) repo: PathBuf,
    /// Whether to verify generated files instead of writing them.
    pub(crate) check: bool,
}

/// Parses common generator options.
pub(crate) fn parse_repo_tool_args(
    args: &[String],
    command: &str,
) -> Result<RepoToolOptions, String> {
    let mut repo = None;
    let mut check = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--check" => check = true,
            "--repo" => {
                index += 1;
                let value = args.get(index).ok_or_else(|| {
                    format!("--repo requires a path\n{}", repo_tool_usage(command))
                })?;
                repo = Some(PathBuf::from(value));
            }
            "-h" | "--help" => return Err(repo_tool_usage(command)),
            other => {
                return Err(format!(
                    "unknown {command} argument `{other}`\n{}",
                    repo_tool_usage(command)
                ));
            }
        }
        index += 1;
    }

    let start = match repo {
        Some(path) => path.canonicalize().map_err(display_io)?,
        None => env::current_dir().map_err(display_io)?,
    };
    Ok(RepoToolOptions {
        repo: find_repo_root(&start)?,
        check,
    })
}

fn repo_tool_usage(command: &str) -> String {
    format!("usage: xtask {command} [--check] [--repo <path>]")
}

/// Finds the public repository root for a generator command.
pub(crate) fn find_repo_root(start: &Path) -> Result<PathBuf, String> {
    for dir in start.ancestors() {
        if !dir.join("Cargo.toml").is_file() {
            continue;
        }
        if is_git_root(dir) {
            return Ok(dir.to_path_buf());
        }
        if let Some(root) = cargo_workspace_root(dir)? {
            return Ok(root);
        }
    }
    Err(format!("could not find repo root from {}", start.display()))
}

fn is_git_root(dir: &Path) -> bool {
    let git = dir.join(".git");
    git.is_dir() || git.is_file()
}

fn cargo_workspace_root(dir: &Path) -> Result<Option<PathBuf>, String> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(dir.join("Cargo.toml"))
        .output()
        .map_err(display_io)?;
    if !output.status.success() {
        return Ok(None);
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("parse cargo metadata: {err}"))?;
    Ok(json["workspace_root"].as_str().map(PathBuf::from))
}

fn display_io(err: io::Error) -> String {
    err.to_string()
}
