use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use toml::Value;

use super::io::display_io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RepoEntry {
    pub(super) name: String,
    pub(super) kind: String,
    pub(super) local_path: String,
    pub(super) checkout_path: PathBuf,
    pub(super) contains_code: bool,
    pub(super) crate_names: Vec<String>,
    pub(super) source_paths: Vec<String>,
    pub(super) validation_command: String,
    pub(super) docs_command: String,
    pub(super) pin: String,
    pub(super) publish_to_github: bool,
    pub(super) status: RepoStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RepoStatus {
    Clean,
    Dirty,
    Missing,
    MissingCargoToml,
    NotGit,
}

impl RepoStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Dirty => "dirty",
            Self::Missing => "missing",
            Self::MissingCargoToml => "missing-cargo-toml",
            Self::NotGit => "not-git",
        }
    }

    pub(super) fn diagnostic_kind(self) -> Option<&'static str> {
        match self {
            Self::Clean => None,
            Self::Dirty => Some("repo-dirty"),
            Self::Missing => Some("repo-missing"),
            Self::MissingCargoToml => Some("cargo-toml-missing"),
            Self::NotGit => Some("repo-not-git"),
        }
    }
}

pub(super) fn read_repos_manifest(
    control_root: &Path,
    manifest_path: &Path,
) -> Result<Vec<RepoEntry>, String> {
    let text = fs::read_to_string(manifest_path).map_err(display_io)?;
    let manifest = text
        .parse::<Value>()
        .map_err(|err| format!("parse {}: {err}", manifest_path.display()))?;
    let repos = manifest
        .get("repo")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{} has no [[repo]] entries", manifest_path.display()))?;

    repos
        .iter()
        .map(|repo| repo_entry(control_root, repo))
        .collect()
}

fn repo_entry(control_root: &Path, repo: &Value) -> Result<RepoEntry, String> {
    let name = required_str(repo, "name")?.to_owned();
    let kind = required_str(repo, "kind")?.to_owned();
    let local_path = required_str(repo, "local_path")?.to_owned();
    let checkout_path = resolve_path(control_root, &local_path);
    let contains_code = repo
        .get("contains_code")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let crate_names = string_array(repo, "crate_names")?;
    let source_paths = string_array(repo, "source_paths")?;
    let validation_command = optional_str(repo, "validation_command").to_owned();
    let docs_command = optional_str(repo, "docs_command").to_owned();
    let pin = optional_str(repo, "commit").to_owned();
    let publish_to_github = repo
        .get("publish_to_github")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let status = repo_status(&checkout_path, contains_code);

    Ok(RepoEntry {
        name,
        kind,
        local_path,
        checkout_path,
        contains_code,
        crate_names,
        source_paths,
        validation_command,
        docs_command,
        pin,
        publish_to_github,
        status,
    })
}

fn repo_status(path: &Path, contains_code: bool) -> RepoStatus {
    if !path.is_dir() {
        return RepoStatus::Missing;
    }
    if contains_code && !path.join("Cargo.toml").is_file() {
        return RepoStatus::MissingCargoToml;
    }
    if !path.join(".git").is_dir() {
        return RepoStatus::NotGit;
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("status")
        .arg("--short")
        .output();
    match output {
        Ok(output) if output.status.success() && output.stdout.is_empty() => RepoStatus::Clean,
        Ok(output) if output.status.success() => RepoStatus::Dirty,
        _ => RepoStatus::NotGit,
    }
}

fn resolve_path(control_root: &Path, local_path: &str) -> PathBuf {
    let path = PathBuf::from(local_path);
    if path.is_absolute() {
        path
    } else {
        control_root.join(path)
    }
}

fn required_str<'a>(repo: &'a Value, key: &str) -> Result<&'a str, String> {
    repo.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("repo entry missing string field {key}"))
}

fn optional_str<'a>(repo: &'a Value, key: &str) -> &'a str {
    repo.get(key).and_then(Value::as_str).unwrap_or("")
}

fn string_array(repo: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(values) = repo.get(key) else {
        return Ok(Vec::new());
    };
    values
        .as_array()
        .ok_or_else(|| format!("repo entry field {key} must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("repo entry field {key} must contain strings"))
        })
        .collect()
}
