// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    collections::BTreeMap,
    env, fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

#[test]
fn input_files_cover_rust_sources_without_local_lockfile() {
    let root = temp_root("sim-tooling-input-files");
    fs::create_dir_all(root.join("src/nested")).unwrap();
    fs::create_dir_all(root.join("crates/sim-fixture/src")).unwrap();
    fs::create_dir_all(root.join(".sim/local-guard/src")).unwrap();
    fs::create_dir_all(root.join("sim-tooling/src")).unwrap();
    fs::create_dir_all(root.join("ignored-helper")).unwrap();
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n").unwrap();
    fs::write(root.join("Cargo.lock"), "# local ignored lockfile\n").unwrap();
    fs::write(root.join(".gitignore"), "/ignored-helper/\n").unwrap();
    fs::write(root.join("features.toml"), "schema = \"sim.features\"\n").unwrap();
    fs::write(root.join("src/lib.rs"), "").unwrap();
    fs::write(root.join("src/nested/tool.rs"), "").unwrap();
    fs::write(root.join("crates/sim-fixture/src/lib.rs"), "").unwrap();
    fs::write(
        root.join(".sim/local-guard/Cargo.toml"),
        "[package]\nname = \"local-guard\"\n",
    )
    .unwrap();
    fs::write(root.join(".sim/local-guard/src/lib.rs"), "").unwrap();
    fs::write(
        root.join("sim-tooling/Cargo.toml"),
        "[package]\nname = \"sim-tooling\"\n",
    )
    .unwrap();
    fs::write(root.join("sim-tooling/src/lib.rs"), "").unwrap();
    fs::write(
        root.join("ignored-helper/Cargo.toml"),
        "[package]\nname = \"ignored-helper\"\n",
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );

    assert!(
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    let scope = crate::owned::enter(&root).unwrap();
    let paths = input_files(&root.canonicalize().unwrap(), &json!({}))
        .into_iter()
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/")
        })
        .collect::<Vec<_>>();

    assert!(paths.contains(&"Cargo.toml".to_owned()));
    assert!(paths.contains(&"features.toml".to_owned()));
    assert!(paths.contains(&"src/lib.rs".to_owned()));
    assert!(paths.contains(&"src/nested/tool.rs".to_owned()));
    assert!(paths.contains(&"crates/sim-fixture/src/lib.rs".to_owned()));
    assert!(!paths.contains(&"Cargo.lock".to_owned()));
    assert!(!paths.contains(&".sim/local-guard/Cargo.toml".to_owned()));
    assert!(!paths.contains(&".sim/local-guard/src/lib.rs".to_owned()));
    assert!(!paths.contains(&"sim-tooling/Cargo.toml".to_owned()));
    assert!(!paths.contains(&"sim-tooling/src/lib.rs".to_owned()));
    assert!(!paths.contains(&"ignored-helper/Cargo.toml".to_owned()));
    scope.finish().unwrap();

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn input_files_cover_what_recipe_evidence_and_exclusion_witnesses_actually_read() {
    // recipe_evidence::workflow_commands reads every direct entry of
    // .github/workflows; exclusion_witness reads the root package's own
    // tests/*.rs and tests/*/main.rs. Neither was previously in the
    // workspace-hash input set, so a change to either could move a
    // generated claim without moving workspace_hash, defeating
    // ensure_inputs_unchanged's staleness check.
    let root = temp_root("sim-tooling-input-files-workflows-tests");
    fs::create_dir_all(root.join(".github/workflows/sub")).unwrap();
    fs::create_dir_all(root.join("tests/harness")).unwrap();
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n").unwrap();
    fs::write(root.join(".github/workflows/ci.yml"), "name: ci\n").unwrap();
    fs::write(root.join(".github/workflows/other.yaml"), "name: other\n").unwrap();
    fs::write(root.join(".github/workflows/readme.md"), "not a workflow\n").unwrap();
    // Nested one level deeper: recipe_evidence only reads direct entries.
    fs::write(
        root.join(".github/workflows/sub/nested.yml"),
        "name: nested\n",
    )
    .unwrap();
    fs::write(root.join("tests/dispatch.rs"), "").unwrap();
    fs::write(root.join("tests/harness/main.rs"), "").unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    let scope = crate::owned::enter(&root).unwrap();
    let paths = input_files(&root.canonicalize().unwrap(), &json!({}))
        .into_iter()
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/")
        })
        .collect::<Vec<_>>();

    assert!(paths.contains(&".github/workflows/ci.yml".to_owned()));
    assert!(paths.contains(&".github/workflows/other.yaml".to_owned()));
    assert!(paths.contains(&"tests/dispatch.rs".to_owned()));
    assert!(paths.contains(&"tests/harness/main.rs".to_owned()));
    assert!(!paths.contains(&".github/workflows/readme.md".to_owned()));
    assert!(!paths.contains(&".github/workflows/sub/nested.yml".to_owned()));
    scope.finish().unwrap();

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn input_files_cover_a_target_at_a_non_conventional_path_named_by_metadata() {
    // [lib] path = "code/lib.rs" is invisible to is_contract_input's own
    // src/crates-prefixed patterns; only cargo metadata's own src_path
    // for the target names it.
    let root = temp_root("sim-tooling-input-files-metadata-target");
    fs::create_dir_all(root.join("code")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\n\n[lib]\npath = \"code/lib.rs\"\n",
    )
    .unwrap();
    fs::write(root.join("code/lib.rs"), "").unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    let canonical = root.canonicalize().unwrap();
    let metadata = json!({
        "packages": [{
            "name": "fixture",
            "targets": [{
                "name": "fixture",
                "src_path": canonical.join("code/lib.rs").to_string_lossy(),
            }]
        }]
    });
    let scope = crate::owned::enter(&canonical).unwrap();
    let paths = input_files(&canonical, &metadata)
        .into_iter()
        .map(|path| {
            path.strip_prefix(&canonical)
                .unwrap()
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/")
        })
        .collect::<Vec<_>>();
    assert!(paths.contains(&"code/lib.rs".to_owned()), "{paths:?}");
    scope.finish().unwrap();

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn recipe_books_include_root_layout_book() {
    let root = temp_root("sim-tooling-root-recipes");
    let recipe_dir = root
        .join("recipes")
        .join("01-basics")
        .join("exact-bool-shape");
    fs::create_dir_all(&recipe_dir).unwrap();
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"sim-root\"\n").unwrap();
    fs::write(
        root.join("recipes/book.toml"),
        "book = \"sim-root\"\ntitle = \"Root recipes\"\nsummary = \"Root cookbook.\"\n",
    )
    .unwrap();
    fs::write(
        recipe_dir.join("recipe.toml"),
        "id = \"exact-bool-shape\"\ntitle = \"Exact boolean shape\"\ncodec = \"rust\"\n",
    )
    .unwrap();

    let groups = BTreeMap::from([("sim-root".to_owned(), "workspace".to_owned())]);
    let books = recipe_books(&root, &groups);

    assert_eq!(books.len(), 1);
    assert_eq!(books[0]["package"], "sim-root");
    assert_eq!(books[0]["group"], "workspace");
    assert_eq!(books[0]["book_toml"], "recipes/book.toml");
    assert_eq!(books[0]["recipe_count"], 1);
    assert_eq!(
        books[0]["recipes"][0]["recipe_toml"],
        "recipes/01-basics/exact-bool-shape/recipe.toml"
    );
    assert_eq!(
        books[0]["recipes"][0]["card_id"],
        "sim-root/01-basics/exact-bool-shape"
    );

    fs::remove_dir_all(root).unwrap();
}

fn temp_root(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("{name}-{}-{stamp}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}
