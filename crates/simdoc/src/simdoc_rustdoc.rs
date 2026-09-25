// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn run_api_docs(
    root: &Path,
    force_docbuild: bool,
    resolver: Option<&crate::resolver_input::ResolverInput>,
) -> Result<(), String> {
    let fingerprint = docbuild_fingerprint(root, resolver);
    let cache = root.join("target").join(".simdoc-docbuild-fingerprint");
    let force = force_docbuild || env::var("SIMDOC_FORCE_DOCS").is_ok();
    if !force
        && let Some(current) = &fingerprint
        && fs::read_to_string(&cache).is_ok_and(|cached| cached.trim() == current)
    {
        println!("simdoc: doc inputs unchanged; skipping cargo doc");
        return Ok(());
    }

    let mut command = crate::tools::tools()?.cargo();
    // Documentation generation is a verifier of the repository's selected
    // dependency graph, not an authority to rewrite it.  In particular, a
    // shared constellation resolver may make newer packages visible than the
    // standalone lock selected by the owning repository.
    command.args(["doc", "--locked", "--offline"]);
    // The shared resolver route runs on the one validated identity: its
    // retained manifest path and its retained package selection.
    match resolver {
        Some(input) => {
            command.arg("--manifest-path").arg(&input.manifest);
            if input.selected.is_empty() {
                command.arg("--workspace");
            } else {
                for package in &input.selected {
                    command.args(["-p", package]);
                }
            }
        }
        None => {
            command.arg("--workspace");
        }
    }
    let status = command
        .arg("--no-deps")
        .current_dir(root)
        .status()
        .map_err(|err| format!("cargo doc: {err}"))?;
    if status.success() {
        if let Some(input) = resolver {
            input.remeasure(root)?;
        }
        if let Some(current) = &fingerprint {
            let _ = fs::create_dir_all(cache.parent().unwrap_or(root));
            let _ = fs::write(&cache, current);
        }
        Ok(())
    } else {
        Err(format!("cargo doc failed with status {status}"))
    }
}

fn docbuild_fingerprint(
    root: &Path,
    resolver: Option<&crate::resolver_input::ResolverInput>,
) -> Option<String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut inputs = Vec::new();
    collect_doc_inputs(root, root, &mut inputs).ok()?;
    inputs.sort();

    let mut hasher = DefaultHasher::new();
    rustc_version().hash(&mut hasher);
    resolver
        .map(|input| (input.manifest.clone(), input.cache_key()))
        .hash(&mut hasher);
    for rel in &inputs {
        rel.hash(&mut hasher);
        crate::owned::read(root.join(rel)).ok()?.hash(&mut hasher);
    }
    Some(format!("{:016x}", hasher.finish()))
}

const SKIPPED_DIRECTORIES: [&str; 6] = [
    ".git",
    ".meta-workspace",
    ".sim",
    "target",
    "generated-reports",
    "split-reports",
];

/// Owned files beneath `dir` outside the skipped directories.
fn owned_files_outside_skipped(dir: &Path) -> Vec<PathBuf> {
    crate::owned::files_under(dir)
        .into_iter()
        .filter(|path| {
            path.strip_prefix(dir).is_ok_and(|relative| {
                relative.parent().is_none_or(|parent| {
                    parent.components().all(|component| {
                        !SKIPPED_DIRECTORIES
                            .iter()
                            .any(|skipped| component.as_os_str() == *skipped)
                    })
                })
            })
        })
        .collect()
}

fn collect_doc_inputs(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    for path in owned_files_outside_skipped(dir) {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.ends_with(".rs") || name == "Cargo.toml" || name == "Cargo.lock" {
            files.push(relative_slash(root, &path)?);
        }
    }
    Ok(())
}

fn rustc_version() -> String {
    crate::tools::tools()
        .ok()
        .and_then(|tools| tools.rustc().arg("--version").output().ok())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default()
}

pub(crate) fn repo_packages(root: &Path) -> Result<Vec<String>, String> {
    let manifest = root.join("Cargo.toml");
    let text = crate::owned::read_to_string(&manifest)
        .map_err(|err| format!("read {}: {err}", manifest.display()))?;
    let mut names = Vec::new();
    if let Some(name) = package_name(&text) {
        names.push(name);
    }
    for member in workspace_members(&text) {
        for dir in expand_member(root, &member) {
            if let Ok(member_text) = crate::owned::read_to_string(dir.join("Cargo.toml"))
                && let Some(name) = package_name(&member_text)
                && !names.contains(&name)
            {
                names.push(name);
            }
        }
    }
    Ok(names)
}

fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package
            && let Some(rest) = trimmed.strip_prefix("name")
            && let Some(value) = rest.trim_start().strip_prefix('=')
        {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

fn workspace_members(manifest: &str) -> Vec<String> {
    let Some(start) = manifest.find("members") else {
        return Vec::new();
    };
    let after = &manifest[start..];
    let (Some(open), Some(close)) = (after.find('['), after.find(']')) else {
        return Vec::new();
    };
    if close < open {
        return Vec::new();
    }
    after[open + 1..close]
        .split(',')
        .map(|entry| entry.trim().trim_matches('"').trim().to_owned())
        .filter(|entry| !entry.is_empty())
        .collect()
}

fn expand_member(root: &Path, member: &str) -> Vec<PathBuf> {
    match member.strip_suffix("/*") {
        Some(prefix) => {
            let base = root.join(prefix);
            let mut dirs = crate::owned::files_under(&base)
                .into_iter()
                .filter(|path| {
                    path.strip_prefix(&base).is_ok_and(|relative| {
                        relative.components().count() == 2
                            && relative
                                .file_name()
                                .is_some_and(|name| name == "Cargo.toml")
                    })
                })
                .filter_map(|path| path.parent().map(Path::to_path_buf))
                .collect::<Vec<_>>();
            dirs.sort();
            dirs
        }
        None => vec![root.join(member)],
    }
}

fn relative_slash(root: &Path, path: &Path) -> Result<String, String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|err| format!("relative path {}: {err}", path.display()))?;
    Ok(rel
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

#[cfg(test)]
mod tests {
    use super::{package_name, workspace_members};

    #[test]
    fn package_name_reads_package_section_only() {
        let manifest =
            "[package]\nname = \"sim-shape\"\n\n[workspace.package]\nname = \"ignored\"\n";
        assert_eq!(package_name(manifest), Some("sim-shape".to_owned()));
    }

    #[test]
    fn package_name_absent_for_virtual_manifest() {
        assert_eq!(
            package_name("[workspace]\nmembers = [\"crates/a\"]\n"),
            None
        );
    }

    #[test]
    fn workspace_members_parses_multiline_array() {
        let manifest =
            "[workspace]\nmembers = [\n    \"crates/a\",\n    \"crates/b\",\n    \"xtask\",\n]\n";
        assert_eq!(
            workspace_members(manifest),
            vec![
                "crates/a".to_owned(),
                "crates/b".to_owned(),
                "xtask".to_owned()
            ]
        );
    }

    #[test]
    fn workspace_members_empty_when_absent_or_empty() {
        assert!(workspace_members("[package]\nname = \"x\"\n").is_empty());
        assert!(workspace_members("[workspace]\nmembers = [\n]\n").is_empty());
    }
}
