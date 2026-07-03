use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn run_api_docs(root: &Path, force_docbuild: bool) -> Result<(), String> {
    let fingerprint = docbuild_fingerprint(root);
    let cache = root.join("target").join(".simdoc-docbuild-fingerprint");
    let force = force_docbuild || env::var("SIMDOC_FORCE_DOCS").is_ok();
    if !force
        && let Some(current) = &fingerprint
        && fs::read_to_string(&cache).is_ok_and(|cached| cached.trim() == current)
    {
        println!("simdoc: doc inputs unchanged; skipping cargo doc");
        return Ok(());
    }

    let mut command = Command::new("cargo");
    command.arg("doc");
    match env::var("SIMDOC_CARGO_MANIFEST_PATH") {
        Ok(manifest_path) => {
            command.args(["--manifest-path", &manifest_path]);
            let allowed = meta_member_names(&manifest_path);
            let packages: Vec<String> = if allowed.is_empty() {
                Vec::new()
            } else {
                repo_packages(root)?
                    .into_iter()
                    .filter(|package| allowed.contains(package))
                    .collect()
            };
            if packages.is_empty() {
                command.arg("--workspace");
            } else {
                for package in &packages {
                    command.args(["-p", package]);
                }
            }
        }
        Err(_) => {
            command.arg("--workspace");
        }
    }
    let status = command
        .arg("--no-deps")
        .current_dir(root)
        .status()
        .map_err(|err| format!("cargo doc: {err}"))?;
    if status.success() {
        if let Some(current) = &fingerprint {
            let _ = fs::create_dir_all(cache.parent().unwrap_or(root));
            let _ = fs::write(&cache, current);
        }
        Ok(())
    } else {
        Err(format!("cargo doc failed with status {status}"))
    }
}

fn docbuild_fingerprint(root: &Path) -> Option<String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut inputs = Vec::new();
    collect_doc_inputs(root, root, &mut inputs).ok()?;
    inputs.sort();

    let mut hasher = DefaultHasher::new();
    rustc_version().hash(&mut hasher);
    env::var("SIMDOC_CARGO_MANIFEST_PATH")
        .unwrap_or_default()
        .hash(&mut hasher);
    for rel in &inputs {
        rel.hash(&mut hasher);
        fs::read(root.join(rel)).ok()?.hash(&mut hasher);
    }
    Some(format!("{:016x}", hasher.finish()))
}

fn collect_doc_inputs(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|err| format!("read {}: {err}", dir.display()))? {
        let entry = entry.map_err(|err| format!("read {}: {err}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry
            .file_type()
            .map_err(|err| format!("stat {}: {err}", path.display()))?
            .is_dir()
        {
            if matches!(
                name.as_ref(),
                ".git" | ".meta-workspace" | "target" | "generated-reports" | "split-reports"
            ) {
                continue;
            }
            collect_doc_inputs(root, &path, files)?;
        } else if name.ends_with(".rs") || name == "Cargo.toml" || name == "Cargo.lock" {
            files.push(relative_slash(root, &path)?);
        }
    }
    Ok(())
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn repo_packages(root: &Path) -> Result<Vec<String>, String> {
    let manifest = root.join("Cargo.toml");
    let text = fs::read_to_string(&manifest)
        .map_err(|err| format!("read {}: {err}", manifest.display()))?;
    let mut names = Vec::new();
    if let Some(name) = package_name(&text) {
        names.push(name);
    }
    for member in workspace_members(&text) {
        for dir in expand_member(root, &member) {
            if let Ok(member_text) = fs::read_to_string(dir.join("Cargo.toml"))
                && let Some(name) = package_name(&member_text)
                && !names.contains(&name)
            {
                names.push(name);
            }
        }
    }
    Ok(names)
}

fn meta_member_names(manifest_path: &str) -> Vec<String> {
    let Ok(text) = fs::read_to_string(manifest_path) else {
        return Vec::new();
    };
    workspace_members(&text)
        .into_iter()
        .filter_map(|member| member.rsplit('/').next().map(str::to_owned))
        .collect()
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
            let mut dirs = Vec::new();
            if let Ok(entries) = fs::read_dir(root.join(prefix)) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        dirs.push(entry.path());
                    }
                }
            }
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
    use super::{meta_member_names, package_name, workspace_members};

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

    #[test]
    fn meta_member_names_missing_manifest_is_empty() {
        assert!(meta_member_names("/no/such/Cargo.toml").is_empty());
    }
}
