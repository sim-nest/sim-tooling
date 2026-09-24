//! Shared command-line options for repo-local generators.

use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

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
            "--locked",
            "--offline",
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
