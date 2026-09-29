// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Discovery of runnable recipe and conformance specimens for SIM Index fragments.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use sim_index_core::{DiscoveredSpecimen, SpecimenId, SubjectId};

use crate::{
    index_fragment::{rel_path, repo_name, slug_path, subject_id},
    repo_contract::PackageContract,
    repo_contract_scan::recipe_books,
};

/// Discovers recipe and conformance specimens from a repo checkout.
pub(crate) fn discovered(repo: &Path, packages: &[PackageContract]) -> Vec<DiscoveredSpecimen> {
    let package_groups = packages
        .iter()
        .map(|package| (package.name.clone(), package.group.clone()))
        .collect::<BTreeMap<_, _>>();
    let package_names = packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut specimens = BTreeMap::new();
    let mut checked_recipe_paths = BTreeSet::new();
    let recipe_consumer = crate::recipe_evidence::declared_consumer(repo);

    for book in recipe_books(repo, &package_groups) {
        let Some(recipes) = book["recipes"].as_array() else {
            continue;
        };
        for recipe in recipes {
            let Some(path) = recipe["recipe_toml"].as_str() else {
                continue;
            };
            checked_recipe_paths.insert(path.to_owned());
            let package = recipe["package"]
                .as_str()
                .or_else(|| book["package"].as_str())
                .unwrap_or_default();
            let language = recipe["codec"].as_str().and_then(language_value);
            insert_specimen(
                &mut specimens,
                DiscoveredSpecimen {
                    id: recipe_specimen_id(repo, path),
                    subject: owner_for_package(repo, package, &package_names),
                    kind: "recipe".to_owned(),
                    path: path.to_owned(),
                    language,
                    // A declared consumer is not a validation: the specimen
                    // is neither runnable nor checked, and says who is
                    // declared to run it.
                    runnable: false,
                    checked: false,
                    checked_by: recipe_consumer
                        .as_deref()
                        .map(crate::recipe_evidence::declared_consumer_evidence),
                    doc_anchor: None,
                },
            );
        }
    }

    for path in recipe_toml_files(repo) {
        let rel = rel_path(repo, &path);
        if checked_recipe_paths.contains(&rel) {
            continue;
        }
        let language = crate::owned::read_to_string(&path)
            .ok()
            .and_then(|text| toml_string_value(&text, "codec"))
            .and_then(|value| language_value(&value));
        insert_specimen(
            &mut specimens,
            DiscoveredSpecimen {
                id: recipe_specimen_id(repo, &rel),
                subject: owner_for_path(repo, packages, &path),
                kind: "recipe".to_owned(),
                path: rel,
                language,
                runnable: false,
                checked: false,
                checked_by: None,
                doc_anchor: None,
            },
        );
    }

    for path in conformance_rust_files(repo) {
        let rel = rel_path(repo, &path);
        let text = crate::owned::read_to_string(&path).unwrap_or_default();
        let language = conformance_language(&rel, &text);
        insert_specimen(
            &mut specimens,
            DiscoveredSpecimen {
                id: spec_test_id(repo, &rel),
                subject: owner_for_path(repo, packages, &path),
                kind: "spec-test".to_owned(),
                path: rel,
                language,
                runnable: true,
                checked: true,
                checked_by: Some("cargo test".to_owned()),
                doc_anchor: None,
            },
        );
    }

    specimens.into_values().collect()
}

fn insert_specimen(
    specimens: &mut BTreeMap<String, DiscoveredSpecimen>,
    specimen: DiscoveredSpecimen,
) {
    specimens.entry(specimen.id.to_string()).or_insert(specimen);
}

fn recipe_specimen_id(repo: &Path, rel: &str) -> SpecimenId {
    let mut tail = rel.strip_prefix("recipes/").unwrap_or(rel).to_owned();
    tail = tail.replace("/recipes/", "/");
    if let Some(stripped) = tail.strip_suffix("/recipe.toml") {
        tail = stripped.to_owned();
    }
    SpecimenId::new(format!(
        "recipe/{}/{}",
        slug_path(&repo_name(repo)),
        slug_path(&tail)
    ))
}

fn spec_test_id(repo: &Path, rel: &str) -> SpecimenId {
    let tail = rel.strip_suffix(".rs").unwrap_or(rel);
    SpecimenId::new(format!(
        "spec-test/{}/{}",
        slug_path(&repo_name(repo)),
        slug_path(tail)
    ))
}

fn owner_for_package(repo: &Path, package: &str, package_names: &BTreeSet<&str>) -> SubjectId {
    if package_names.contains(package) {
        subject_id("crate", package)
    } else {
        subject_id("repo", &repo_name(repo))
    }
}

fn owner_for_path(repo: &Path, packages: &[PackageContract], path: &Path) -> SubjectId {
    let rel = rel_path(repo, path);
    let repo_name = repo_name(repo);
    let mut best = None::<&PackageContract>;
    for package in packages {
        if package.root.is_empty() {
            continue;
        }
        if rel == package.root || rel.starts_with(&format!("{}/", package.root)) {
            best = match best {
                Some(current) if current.root.len() >= package.root.len() => Some(current),
                _ => Some(package),
            };
        }
    }
    if let Some(package) = best {
        return subject_id("crate", &package.name);
    }
    if let Some(package) = packages.iter().find(|package| package.root.is_empty()) {
        return subject_id("crate", &package.name);
    }
    if let Some(package) = packages.iter().find(|package| package.name == repo_name) {
        return subject_id("crate", &package.name);
    }
    subject_id("repo", &repo_name)
}

fn recipe_toml_files(repo: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_named_files(repo, "recipe.toml", &mut files);
    files.sort();
    files
}

fn conformance_rust_files(repo: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_ext_files(&repo.join("src"), "rs", &mut files);
    collect_ext_files(&repo.join("tests"), "rs", &mut files);
    collect_ext_files(&repo.join("crates"), "rs", &mut files);
    files.retain(|path| {
        let rel = rel_path(repo, path);
        crate::owned::read_to_string(path)
            .map(|text| is_conformance_source(&rel, &text))
            .unwrap_or(false)
    });
    files.sort();
    files
}

fn is_conformance_source(rel: &str, text: &str) -> bool {
    let code = without_quoted_strings(text);
    rel.contains("conformance")
        || code.contains("run_registered_conformance")
        || code.contains(" conformance:")
}

fn conformance_language(rel: &str, text: &str) -> Option<String> {
    if rel.contains("sim-codec-doc") {
        Some("doc".to_owned())
    } else if rel.contains("sim-codec-lisp") || text.contains("codec/lisp") {
        Some("lisp".to_owned())
    } else if rel.contains("sim-codec-json") || text.contains("codec/json") {
        Some("json".to_owned())
    } else if rel.contains("sim-codec") || text.contains("codec/") {
        Some("codec".to_owned())
    } else if rel.contains("shape") || text.contains("Shape") || text.contains("shape") {
        Some("shape".to_owned())
    } else {
        None
    }
}

fn language_value(value: &str) -> Option<String> {
    let value = slug_path(value.trim());
    (!value.is_empty()).then_some(value)
}

fn toml_string_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix(key)?.trim_start();
        quoted_value(rest.strip_prefix('=')?.trim_start())
    })
}

fn quoted_value(text: &str) -> Option<String> {
    let after_start = text.split_once('"')?.1;
    let end = after_start.find('"')?;
    Some(after_start[..end].to_owned())
}

fn without_quoted_strings(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    for ch in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            out.push(' ');
        } else if ch == '"' {
            in_string = true;
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

fn collect_named_files(dir: &Path, name: &str, out: &mut Vec<PathBuf>) {
    out.extend(
        crate::owned::files_under(dir)
            .into_iter()
            .filter(|path| descends(dir, path))
            .filter(|path| path.file_name().and_then(|file| file.to_str()) == Some(name)),
    );
}

fn collect_ext_files(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    out.extend(
        crate::owned::files_under(dir)
            .into_iter()
            .filter(|path| descends(dir, path))
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some(extension)),
    );
}

/// Whether every directory between `dir` and `path` is one a walk descends.
fn descends(dir: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(dir) else {
        return false;
    };
    let parent = relative.parent().unwrap_or(Path::new(""));
    parent
        .components()
        .all(|component| should_descend(Path::new(component.as_os_str())))
}

fn should_descend(path: &Path) -> bool {
    !matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | "target" | ".meta-workspace" | ".sim")
    )
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::json;

    use super::*;

    #[test]
    fn index_specimen_scan_finds_checked_and_unchecked_rows() {
        let parent = temp_root("sim-tooling-specimens-parent");
        let root = parent.join("sim-demo-repo");
        let checked_dir = root.join("recipes/01-basics/checked");
        let loose_dir = root.join("crates/sim-loose/recipes/01-basics/not-run");
        let conformance_dir = root.join("crates/sim-codec-doc/tests");
        let root_conformance_dir = root.join("tests");
        let xtask_dir = root.join("xtask/src");
        fs::create_dir_all(&checked_dir).unwrap();
        fs::create_dir_all(&loose_dir).unwrap();
        fs::create_dir_all(&conformance_dir).unwrap();
        fs::create_dir_all(&root_conformance_dir).unwrap();
        fs::create_dir_all(&xtask_dir).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"sim-demo\"\n\n[workspace.metadata.sim]\n\
             validation-commands = [\"cargo run -p xtask -- check-recipes\"]\n",
        )
        .unwrap();
        fs::write(root.join("recipes/book.toml"), "book = \"sim-demo\"\n").unwrap();
        fs::write(
            xtask_dir.join("main.rs"),
            "fn main() { match std::env::args().nth(1).as_deref() { Some(\"check-recipes\") => run(), _ => {} } }\nfn run() {}\n",
        )
        .unwrap();
        fs::write(
            checked_dir.join("recipe.toml"),
            "id = \"checked\"\ntitle = \"Checked\"\ncodec = \"lisp\"\n",
        )
        .unwrap();
        fs::write(
            loose_dir.join("recipe.toml"),
            "id = \"not-run\"\ntitle = \"Not run\"\ncodec = \"cli transcript\"\n",
        )
        .unwrap();
        fs::write(
            conformance_dir.join("conformance.rs"),
            "#[test]\nfn all_implemented_backends_roundtrip_simple_fixture() {}\n",
        )
        .unwrap();
        fs::write(
            root_conformance_dir.join("behavior.rs"),
            "// conformance: the root package preserves its public behavior.\n#[test]\nfn public_behavior_is_stable() {}\n",
        )
        .unwrap();
        let packages = vec![
            package("sim-demo", ""),
            package("sim-codec-doc", "crates/sim-codec-doc"),
            package("sim-loose", "crates/sim-loose"),
        ];

        let specimens = discovered(&root, &packages);
        let ids = specimens
            .iter()
            .map(|specimen| specimen.id.as_str().to_owned())
            .collect::<Vec<_>>();
        let checked = find(&specimens, "recipe/sim-demo-repo/01-basics/checked");
        let loose = find(
            &specimens,
            "recipe/sim-demo-repo/crates/sim-loose/01-basics/not-run",
        );
        let spec_test = find(
            &specimens,
            "spec-test/sim-demo-repo/crates/sim-codec-doc/tests/conformance",
        );
        let root_spec_test = find(&specimens, "spec-test/sim-demo-repo/tests/behavior");

        assert_eq!(ids, sorted(ids.clone()));
        assert_eq!(checked.subject.as_str(), "crate/sim-demo");
        assert_eq!(checked.path, "recipes/01-basics/checked/recipe.toml");
        assert_eq!(checked.language.as_deref(), Some("lisp"));
        assert!(!checked.runnable);
        assert!(!checked.checked);
        assert_eq!(
            checked.checked_by.as_deref(),
            Some("declared consumer, not validated by simdoc: xtask check-recipes")
        );
        assert!(checked.doc_anchor.is_none());
        assert_eq!(loose.subject.as_str(), "crate/sim-loose");
        assert_eq!(loose.language.as_deref(), Some("cli-transcript"));
        assert!(!loose.runnable);
        assert!(!loose.checked);
        assert!(loose.checked_by.is_none());
        assert_eq!(spec_test.kind, "spec-test");
        assert_eq!(spec_test.subject.as_str(), "crate/sim-codec-doc");
        assert_eq!(spec_test.language.as_deref(), Some("doc"));
        assert_eq!(spec_test.checked_by.as_deref(), Some("cargo test"));
        assert_eq!(root_spec_test.subject.as_str(), "crate/sim-demo");
        assert_eq!(root_spec_test.checked_by.as_deref(), Some("cargo test"));
        assert_eq!(
            ids,
            discovered(&root, &packages)
                .iter()
                .map(|row| row.id.to_string())
                .collect::<Vec<_>>()
        );

        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn index_specimen_scan_does_not_claim_unchecked_recipe_book() {
        let parent = temp_root("sim-tooling-unchecked-specimens-parent");
        let root = parent.join("sim-demo-repo");
        let recipe_dir = root.join("recipes/01-basics/not-run");
        fs::create_dir_all(&recipe_dir).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"sim-demo\"\n").unwrap();
        fs::write(root.join("recipes/book.toml"), "book = \"sim-demo\"\n").unwrap();
        fs::write(
            recipe_dir.join("recipe.toml"),
            "id = \"not-run\"\ntitle = \"Not run\"\ncodec = \"lisp\"\n",
        )
        .unwrap();

        let specimens = discovered(&root, &[package("sim-demo", "")]);
        let recipe = find(&specimens, "recipe/sim-demo-repo/01-basics/not-run");

        assert_eq!(recipe.kind, "recipe");
        assert!(!recipe.runnable);
        assert!(!recipe.checked);
        assert!(recipe.checked_by.is_none());

        fs::remove_dir_all(parent).unwrap();
    }

    fn find<'a>(specimens: &'a [DiscoveredSpecimen], id: &str) -> &'a DiscoveredSpecimen {
        specimens
            .iter()
            .find(|specimen| specimen.id.as_str() == id)
            .unwrap()
    }

    fn sorted(mut values: Vec<String>) -> Vec<String> {
        values.sort();
        values
    }

    fn package(name: &str, root: &str) -> PackageContract {
        PackageContract {
            name: name.to_owned(),
            crate_name: name.replace('-', "_"),
            manifest: if root.is_empty() {
                "Cargo.toml".to_owned()
            } else {
                format!("{root}/Cargo.toml")
            },
            root: root.to_owned(),
            group: "workspace".to_owned(),
            publish: "false".to_owned(),
            description: format!("{name} package"),
            target_kinds: vec!["lib".to_owned()],
            targets: vec![json!({
                "name": name,
                "kind": ["lib"],
                "crate_types": ["lib"],
                "src": if root.is_empty() { "src/lib.rs".to_owned() } else { format!("{root}/src/lib.rs") },
            })],
            dependencies: Vec::new(),
            source_dependencies: Vec::new(),
            features: Vec::new(),
            rustdoc_summary: format!("{name} docs"),
        }
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
}
