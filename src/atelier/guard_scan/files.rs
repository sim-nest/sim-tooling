use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use super::super::{index_manifest::RepoEntry, io::display_io};

pub(super) fn listed_files(repo: &RepoEntry) -> Result<Vec<PathBuf>, String> {
    if !repo.checkout_path.is_dir() {
        return Ok(Vec::new());
    }
    if repo.checkout_path.join(".git").is_dir() {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo.checkout_path)
            .arg("ls-files")
            .output()
            .map_err(display_io)?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .filter(|path| should_scan_path(path))
                .collect());
        }
    }
    let mut files = Vec::new();
    collect_files(&repo.checkout_path, Path::new(""), &mut files)?;
    Ok(files)
}

fn collect_files(root: &Path, relative: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root.join(relative)).map_err(display_io)? {
        let entry = entry.map_err(display_io)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if should_skip_dir(name.as_ref()) {
            continue;
        }
        let relative_path = relative.join(name.as_ref());
        if entry.file_type().map_err(display_io)?.is_dir() {
            collect_files(root, &relative_path, files)?;
        } else if should_scan_path(&relative_path) {
            files.push(relative_path);
        }
    }
    Ok(())
}

pub(super) fn read_repo_text(repo: &RepoEntry, relative: &Path) -> Result<String, String> {
    fs::read_to_string(repo.checkout_path.join(relative)).map_err(display_io)
}

pub(super) fn git_status_paths(root: &Path, pathspec: &str) -> Result<Vec<String>, String> {
    if !root.join(".git").is_dir() {
        return Ok(Vec::new());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("status")
        .arg("--short")
        .arg("--")
        .arg(pathspec)
        .output()
        .map_err(display_io)?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.get(3..))
        .map(str::to_owned)
        .collect())
}

pub(super) fn git_remotes(root: &Path) -> Result<Vec<String>, String> {
    if !root.join(".git").is_dir() {
        return Ok(Vec::new());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("remote")
        .arg("-v")
        .output()
        .map_err(display_io)?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect())
}

pub(super) fn is_text_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if matches!(name, "README" | "README.md" | "Cargo.toml") {
        return true;
    }
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("md" | "rs" | "sh" | "toml" | "txt" | "json" | "yaml" | "yml" | "example")
    )
}

pub(super) fn is_public_doc_or_comment_file(path: &Path) -> bool {
    if path.file_name().and_then(|name| name.to_str()) == Some("README.md") {
        return true;
    }
    if path.starts_with("docs") && path.extension().and_then(|ext| ext.to_str()) == Some("md") {
        return true;
    }
    path.extension().and_then(|ext| ext.to_str()) == Some("rs")
}

pub(super) fn is_rust_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("rs")
}

pub(super) fn is_source_rust_file(path: &Path) -> bool {
    is_rust_file(path) && !path.starts_with("docs")
}

fn should_skip_dir(name: &str) -> bool {
    matches!(name, ".git" | "target" | ".sim" | ".meta-workspace")
        || (name.starts_with('.') && name != ".env.example")
}

fn should_scan_path(path: &Path) -> bool {
    if path.starts_with("target") || path.starts_with(".sim") || path.starts_with(".meta-workspace")
    {
        return false;
    }
    !path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .any(|part| part.starts_with('.') && part != ".env.example")
}

pub(super) fn dependency_path_value(line: &str) -> Option<&str> {
    let (_, after) = line.split_once("path")?;
    let (_, after_equals) = after.split_once('=')?;
    let trimmed = after_equals.trim_start();
    let quote = trimmed.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &trimmed[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(&rest[..end])
}

pub(super) fn path_has_meta_workspace(path: &str) -> bool {
    path.split(['/', '\\'])
        .any(|part| part == ".meta-workspace")
}

pub(super) fn line_contains(line: &str, needle: &str) -> bool {
    if needle
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch == ' ')
    {
        line.contains(needle)
    } else {
        line.to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
    }
}

pub(super) fn line_number(text: &str, byte_index: usize) -> usize {
    text[..byte_index]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}
