// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    collect_doc_inputs, expand_member, package_name, require_path_dependencies_owned,
    segment_matches_glob, workspace_members,
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
    let err =
        require_path_dependencies_owned(&repo, &repo.join("Cargo.toml"), Some(&lock)).unwrap_err();
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
    let err =
        require_path_dependencies_owned(&repo, &repo.join("Cargo.toml"), Some(&lock)).unwrap_err();
    assert!(
        err.contains("evil.rs") && err.contains("not part of the package's owned closure"),
        "{err}"
    );
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn package_name_reads_package_section_only() {
    let manifest = "[package]\nname = \"sim-shape\"\n\n[workspace.package]\nname = \"ignored\"\n";
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
fn package_name_reads_a_single_quoted_name() {
    // TOML allows a literal (single-quoted) string equally to a basic
    // (double-quoted) one; a line scanner that only strips double quotes
    // reads the quotes themselves back as part of the name.
    assert_eq!(
        package_name("[package]\nname = 'sim-shape'\n"),
        Some("sim-shape".to_owned())
    );
}

#[test]
fn package_name_ignores_an_inline_comment() {
    assert_eq!(
        package_name("[package]\nname = \"sim-shape\" # not \"ignored\"\n"),
        Some("sim-shape".to_owned())
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
fn workspace_members_is_never_taken_from_default_members() {
    // `default-members` ends in the same word; a substring search for
    // "members" starting from the front of the manifest would land
    // inside it (it comes first here) and read its array instead.
    let manifest = "[workspace]\ndefault-members = [\"crates/only-default\"]\n\
         members = [\"crates/a\", \"crates/b\"]\n";
    assert_eq!(
        workspace_members(manifest),
        vec!["crates/a".to_owned(), "crates/b".to_owned()]
    );
}

#[test]
fn workspace_members_is_never_taken_from_a_comment() {
    // A comment mentioning the word "members" before the real key, with
    // its own bracketed text later in the file, must never be read as
    // the array.
    let manifest = "[workspace]\n# members of the room, alphabetically: [\"a\", \"b\"]\n\
         members = [\"crates/a\"]\n";
    assert_eq!(workspace_members(manifest), vec!["crates/a".to_owned()]);
}

#[test]
fn segment_matches_glob_supports_a_wildcard_anywhere_in_one_segment() {
    // Cargo allows `*` anywhere in a workspace member's own trailing path
    // segment, not only as the whole segment.
    assert!(segment_matches_glob("app-service", "app-*"));
    assert!(segment_matches_glob("app-service", "*-service"));
    assert!(segment_matches_glob("app-service", "app-service"));
    assert!(segment_matches_glob("anything", "*"));
    assert!(!segment_matches_glob("app-service", "app-web"));
    assert!(!segment_matches_glob("app", "app-*"));
    assert!(segment_matches_glob("aXbYc", "a*b*c"));
    assert!(!segment_matches_glob("aXbYc", "a*Z*c"));
}

#[test]
fn expand_member_matches_a_partial_segment_glob() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "simdoc-expand-member-{}-{stamp}",
        std::process::id()
    ));
    for name in ["app-web", "app-api", "lib-shared"] {
        fs::create_dir_all(root.join("crates").join(name)).unwrap();
        fs::write(
            root.join("crates").join(name).join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\n"),
        )
        .unwrap();
    }

    let mut matched = expand_member(&root, "crates/app-*")
        .into_iter()
        .filter_map(|path| path.file_name()?.to_str().map(str::to_owned))
        .collect::<Vec<_>>();
    matched.sort();
    assert_eq!(matched, vec!["app-api".to_owned(), "app-web".to_owned()]);

    let _ = fs::remove_dir_all(&root);
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
