use std::{
    fs, io,
    path::{Path, PathBuf},
};

use super::site::AtelierSiteOptions;

const DEFAULT_CACHE: &str = ".sim/atelier/site.json";

pub(super) fn editable_roots(options: &AtelierSiteOptions) -> Result<Vec<String>, String> {
    let mut roots = Vec::new();
    if let Some(manifest) = &options.repos_manifest {
        roots.extend(editable_roots_from_manifest(manifest)?);
    }
    roots.extend(options.editable_roots.iter().cloned());
    Ok(roots)
}

pub(super) fn cache_path(options: &AtelierSiteOptions) -> PathBuf {
    options
        .cache_path
        .clone()
        .unwrap_or_else(|| options.control_root.join(DEFAULT_CACHE))
}

pub(super) fn write_cache(path: &Path, content: &str) -> Result<bool, String> {
    let current = fs::read_to_string(path).ok();
    if current.as_deref() == Some(content) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(display_io)?;
    }
    fs::write(path, content).map_err(display_io)?;
    Ok(true)
}

pub(super) fn check_cache(
    path: &Path,
    content: &str,
    refresh_command: &str,
) -> Result<bool, String> {
    match fs::read_to_string(path) {
        Ok(current) if current == content => Ok(false),
        Ok(_) => Err(format!(
            "{} is stale; run `{refresh_command}`",
            path.display(),
        )),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(format!(
            "{} is missing; run `{refresh_command}`",
            path.display(),
        )),
        Err(err) => Err(display_io(err)),
    }
}

pub(super) fn normalize_roots(roots: Vec<String>) -> Vec<String> {
    let mut roots = roots
        .into_iter()
        .map(|root| root.replace(std::path::MAIN_SEPARATOR, "/"))
        .map(|root| root.trim_end_matches('/').to_owned())
        .filter(|root| !root.is_empty())
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots
}

pub(super) fn is_meta_workspace(root: &str) -> bool {
    root.split('/').any(|part| part == ".meta-workspace")
}

pub(super) fn display_io(err: io::Error) -> String {
    err.to_string()
}

fn editable_roots_from_manifest(path: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(path).map_err(display_io)?;
    let manifest = text
        .parse::<toml::Value>()
        .map_err(|err| format!("parse {}: {err}", path.display()))?;
    let repos = manifest
        .get("repo")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| format!("{} has no [[repo]] entries", path.display()))?;
    let mut roots = Vec::new();
    for repo in repos {
        if let Some(local_path) = repo.get("local_path").and_then(toml::Value::as_str) {
            roots.push(local_path.to_owned());
        }
    }
    Ok(roots)
}
