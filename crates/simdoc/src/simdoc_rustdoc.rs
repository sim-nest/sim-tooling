// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::bounded_process::run_bounded;
use crate::worktree::Worktree;

/// Ceiling for one `cargo metadata` document as read here.
const MAX_METADATA_BYTES: usize = 256 * 1024 * 1024;
/// Ceiling for Cargo diagnostics.
const MAX_DIAGNOSTIC_BYTES: usize = 1024 * 1024;

pub(crate) fn run_api_docs(
    root: &Path,
    force_docbuild: bool,
    resolver: Option<&crate::resolver_input::ResolverInput>,
) -> Result<(), String> {
    // Checked before the fingerprint cache below, not after: a `target`
    // that is a link would send Cargo's writes elsewhere, and a link whose
    // target happens to already hold a matching fingerprint file would
    // otherwise let the cache-hit return skip this refusal entirely.
    if fs::symlink_metadata(root.join("target")).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(format!(
            "refused: {} is a symlink; cargo doc would write through it",
            root.join("target").display()
        ));
    }
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

    let manifests = resolver
        .map(|input| input.manifest.as_path())
        .into_iter()
        .collect::<Vec<_>>();
    let lock_path = resolver.map_or_else(
        || root.join("Cargo.lock"),
        |input| input.manifest.with_file_name("Cargo.lock"),
    );
    // The lock that decides which registry crates are built must be the
    // repository's own (a tracked file), or the shared resolver's (bound by
    // its digest). An untracked repository lock is refused, not read.
    let lock = if resolver.is_some() {
        fs::read_to_string(lock_path).ok()
    } else if crate::owned::is_owned_file(&lock_path) {
        crate::owned::read_to_string(&lock_path).ok()
    } else {
        // No tracked lock (a library that ignores its own): no committed
        // dependency graph exists to build against, and a lock that anyone
        // may have written must not choose which registry crates run. The
        // docs build only verifies; the repository's own CI builds it.
        println!("simdoc: no tracked Cargo.lock; skipping cargo doc");
        return Ok(());
    };
    // The shared resolver route validates its complete path-package closure
    // itself (resolver_input::validate, called before this function ever
    // runs). The standalone route has no such validation elsewhere: a
    // tracked member manifest may still declare a path dependency reaching
    // an untracked or out-of-repository directory, whose build script or
    // proc macro `cargo doc` would still compile despite `--no-deps` (which
    // only suppresses generated doc pages for non-member packages, not their
    // compilation). Refuse before running cargo, not after.
    if resolver.is_none() {
        require_path_dependencies_owned(root, &root.join("Cargo.toml"), lock.as_deref())?;
    }
    // Cargo builds into a fresh private target directory: a `target` left in
    // the tree (untracked, so anyone's) is never reused, whatever it holds.
    // `cargo doc` here only verifies the dependency graph; nothing it writes
    // is read back.
    let target = crate::cargo_home::private_dir("doc-target")?;
    let mut command = crate::tools::tools()?.cargo_in(root, &manifests, lock.as_deref())?;
    command.env("CARGO_TARGET_DIR", &target);
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
        .status()
        .map_err(|err| format!("cargo doc: {err}"));
    let _ = fs::remove_dir_all(&target);
    let status = status?;
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

/// Refuses `manifest`'s repository (`root`'s own workspace, no shared
/// resolver) unless every path dependency its full dependency graph reaches
/// -- not only its workspace members, which `--no-deps` alone would report
/// -- lies inside `root` and is owned entirely by `root`'s own Git worktree.
/// `cargo doc --no-deps` still compiles every dependency's build script and
/// proc macro; it only skips generating doc pages for them. An untracked or
/// out-of-repository path dependency would let that compilation execute code
/// nobody reviewed, so this runs before `cargo doc`, not after.
fn require_path_dependencies_owned(
    root: &Path,
    manifest: &Path,
    lock: Option<&str>,
) -> Result<(), String> {
    let worktree = Worktree::open(root)?;
    let mut command = crate::tools::tools()?.cargo_in(root, &[manifest], lock)?;
    command
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest);
    let captured = run_bounded(
        command,
        "standalone cargo metadata",
        MAX_METADATA_BYTES,
        MAX_DIAGNOSTIC_BYTES,
    )?;
    if !captured.status.success() {
        return Err(format!(
            "cargo metadata failed for {}: {}",
            manifest.display(),
            String::from_utf8_lossy(&captured.stderr).trim()
        ));
    }
    let metadata: Value = serde_json::from_slice(&captured.stdout)
        .map_err(|err| format!("parse cargo metadata: {err}"))?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or("cargo metadata has no packages")?
        .iter()
        .filter_map(|package| Some((package["id"].as_str()?, package)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let members = metadata["workspace_members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let edges = metadata["resolve"]["nodes"]
        .as_array()
        .ok_or("cargo metadata has no resolve graph")?
        .iter()
        .filter_map(|node| {
            let deps = node["deps"]
                .as_array()?
                .iter()
                .filter_map(|dep| dep["pkg"].as_str())
                .collect::<Vec<_>>();
            Some((node["id"].as_str()?, deps))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut pending = members;
    let mut reached = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if reached.insert(id) {
            pending.extend(edges.get(id).into_iter().flatten().copied());
        }
    }
    for id in reached {
        let Some(package) = packages.get(id) else {
            continue;
        };
        if !package["source"].is_null() {
            continue;
        }
        let manifest_path = package["manifest_path"]
            .as_str()
            .ok_or("cargo metadata package has no manifest_path")?;
        let name = package["name"].as_str().unwrap_or("<unnamed>");
        let dir = Path::new(manifest_path)
            .parent()
            .ok_or("cargo metadata package manifest has no parent")?;
        let bound = require_directory_owned(&worktree, dir)
            .map_err(|why| format!("refused: path dependency {name} ({manifest_path}): {why}"))?;
        require_targets_bound(package, &bound)
            .map_err(|why| format!("refused: path dependency {name} ({manifest_path}): {why}"))?;
    }
    Ok(())
}

/// Every metadata target's `src_path` (`[lib]`, `[[bin]]`, `[[test]]`,
/// `[[example]]`, `[[bench]]`, and the `build` script) must canonicalize to
/// a file `bound` already validated. A manifest may name any path, absolute
/// or relative, including one outside its own package directory entirely,
/// or one inside that directory but beneath a directory
/// `require_directory_owned` does not walk (`.git`, `target`): neither is
/// caught by validating the package directory alone.
fn require_targets_bound(
    package: &Value,
    bound: &std::collections::BTreeSet<PathBuf>,
) -> Result<(), String> {
    for target in package["targets"].as_array().into_iter().flatten() {
        let name = target["name"].as_str().unwrap_or("<unnamed>");
        let source = target["src_path"]
            .as_str()
            .ok_or("cargo metadata target has no src_path")?;
        let canonical = Path::new(source)
            .canonicalize()
            .map_err(|err| format!("target {name} reads {source}: {err}"))?;
        if !bound.contains(&canonical) {
            return Err(format!(
                "target {name} reads {source}, which is not part of the package's owned closure"
            ));
        }
    }
    Ok(())
}

/// Directories Cargo never reads as compilation input from a fresh, private
/// target directory: Git's own internal metadata (never tracked by Git
/// itself, so it would otherwise always refuse) and stale build output.
const IGNORED_SOURCE_DIRECTORIES: [&str; 2] = [".git", "target"];

/// Every entry beneath `dir`, recursively, must be an ordinary file this
/// repository's own worktree tracks; a symlink (to a file or a directory)
/// refuses immediately, matching this module's deny-by-default reading of
/// what a build may reach. Returns the canonicalized set of files validated
/// this way, so a caller can additionally require some other named path
/// (a metadata target's `src_path`) to be exactly one of them.
fn require_directory_owned(
    worktree: &Worktree,
    dir: &Path,
) -> Result<std::collections::BTreeSet<PathBuf>, String> {
    let mut bound = std::collections::BTreeSet::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in
            fs::read_dir(&current).map_err(|err| format!("{}: {err}", current.display()))?
        {
            let entry = entry.map_err(|err| format!("{}: {err}", current.display()))?;
            let kind = entry
                .file_type()
                .map_err(|err| format!("{}: {err}", entry.path().display()))?;
            if kind.is_dir() {
                if IGNORED_SOURCE_DIRECTORIES
                    .contains(&entry.file_name().to_string_lossy().as_ref())
                {
                    continue;
                }
                pending.push(entry.path());
            } else {
                worktree.owned_file(&entry.path(), "path dependency source")?;
                let canonical = entry
                    .path()
                    .canonicalize()
                    .map_err(|err| format!("{}: {err}", entry.path().display()))?;
                bound.insert(canonical);
            }
        }
    }
    Ok(bound)
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

/// Every owned file outside the skipped directories, not only `.rs`,
/// `Cargo.toml`, and `Cargo.lock`: a source file's `include_str!` or
/// `include_bytes!` can name any other tracked file (a README, a data
/// asset), and cargo doc actually recompiles it whenever such an asset
/// changes, so a fingerprint that ignores it would reuse a stale cache
/// against docs that no longer match the source. Fingerprinting too much
/// only costs a rebuild that was not strictly needed; fingerprinting too
/// little reuses docs that are wrong.
fn collect_doc_inputs(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    for path in owned_files_outside_skipped(dir) {
        files.push(relative_slash(root, &path)?);
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
    use std::{
        fs,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        collect_doc_inputs, package_name, require_path_dependencies_owned, workspace_members,
    };

    fn git(dir: &std::path::Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .unwrap()
                .success()
        );
    }

    /// A workspace `repo` whose member `app` path-depends on `helper`,
    /// placed either inside `repo` (tracked) or in a sibling directory
    /// (untracked, outside `repo`).
    fn layout(label: &str, helper_outside: bool) -> (std::path::PathBuf, std::path::PathBuf) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("{label}-{}-{stamp}", std::process::id()));
        let repo = base.join("repo");
        let write = |relative: &str, text: &str| {
            let path = repo.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "Cargo.toml",
            "[workspace]\nresolver = \"3\"\nmembers = [\"app\"]\n",
        );
        let helper_path = if helper_outside {
            "../../outside/helper"
        } else {
            "../helper"
        };
        write(
            "app/Cargo.toml",
            &format!(
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
                 [dependencies]\nhelper = {{ path = \"{helper_path}\" }}\n"
            ),
        );
        write("app/src/lib.rs", "");
        if helper_outside {
            let outside = base.join("outside/helper");
            fs::create_dir_all(outside.join("src")).unwrap();
            fs::write(
                outside.join("Cargo.toml"),
                "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            )
            .unwrap();
            fs::write(outside.join("src/lib.rs"), "pub fn one() {}\n").unwrap();
        } else {
            write(
                "helper/Cargo.toml",
                "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            );
            write("helper/src/lib.rs", "pub fn one() {}\n");
        }
        git(&repo, &["init", "--quiet"]);
        git(&repo, &["add", "-A"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "--no-verify",
                "-mfixture",
            ],
        );
        let status = Command::new(env!("CARGO"))
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(repo.join("Cargo.toml"))
            .status()
            .unwrap();
        assert!(status.success());
        (base, repo)
    }

    #[test]
    fn a_path_dependency_outside_the_repository_is_refused() {
        let (base, repo) = layout("rustdoc-outside-dep", true);
        let lock = fs::read_to_string(repo.join("Cargo.lock")).unwrap();
        let err = require_path_dependencies_owned(&repo, &repo.join("Cargo.toml"), Some(&lock))
            .unwrap_err();
        assert!(err.contains("helper") && err.contains("refused"), "{err}");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_path_dependency_tracked_inside_the_repository_is_allowed() {
        let (base, repo) = layout("rustdoc-inside-dep", false);
        let lock = fs::read_to_string(repo.join("Cargo.lock")).unwrap();
        require_path_dependencies_owned(&repo, &repo.join("Cargo.toml"), Some(&lock)).unwrap();
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_target_path_outside_its_own_package_directory_is_refused() {
        // helper's own directory is validated fine (Cargo.toml + src/lib.rs
        // are both tracked, inside helper/), but its [lib] path points
        // somewhere else entirely -- require_directory_owned alone would
        // never visit that other location, so only a per-target src_path
        // check catches this.
        let (base, repo) = layout("rustdoc-target-escape", false);
        fs::write(
            repo.join("helper/Cargo.toml"),
            "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [lib]\npath = \"../../outside/evil.rs\"\n",
        )
        .unwrap();
        fs::create_dir_all(base.join("outside")).unwrap();
        fs::write(base.join("outside/evil.rs"), "pub fn one() {}\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "--no-verify",
                "-mretarget",
            ],
        );
        let lock = fs::read_to_string(repo.join("Cargo.lock")).unwrap();
        let err = require_path_dependencies_owned(&repo, &repo.join("Cargo.toml"), Some(&lock))
            .unwrap_err();
        assert!(
            err.contains("evil.rs") && err.contains("not part of the package's owned closure"),
            "{err}"
        );
        let _ = fs::remove_dir_all(&base);
    }

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
    fn collect_doc_inputs_covers_assets_not_only_rs_and_manifests() {
        // A source file's include_str!/include_bytes! can name any tracked
        // file, and cargo doc actually recompiles when that asset changes;
        // the fingerprint must move too, or a stale cache would be reused.
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("simdoc-doc-inputs-{}-{stamp}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n").unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub const README: &str = include_str!(\"../README.md\");\n",
        )
        .unwrap();
        fs::write(root.join("README.md"), "hello\n").unwrap();
        git(&root, &["init", "--quiet"]);
        git(&root, &["add", "-A"]);

        let scope = crate::owned::enter(&root).unwrap();
        let mut inputs = Vec::new();
        collect_doc_inputs(&root, &root, &mut inputs).unwrap();
        scope.finish().unwrap();

        assert!(inputs.contains(&"README.md".to_owned()), "{inputs:?}");
        assert!(inputs.contains(&"src/lib.rs".to_owned()), "{inputs:?}");
        assert!(inputs.contains(&"Cargo.toml".to_owned()), "{inputs:?}");

        let _ = fs::remove_dir_all(&root);
    }
}
