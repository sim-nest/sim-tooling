// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use std::os::unix::fs::symlink;
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sim-build-inputs-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("fixture root");
        Self { root }
    }

    fn directory(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(&path).expect("fixture directory");
        path
    }

    fn file(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("fixture parent");
        }
        fs::write(&path, bytes).expect("fixture file");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("remove fixture");
    }
}

#[test]
fn usage_exposes_the_selected_view_owner() {
    let text = usage("xtask");
    assert!(text.contains("build-inputs <select --metadata"));
    assert!(text.contains("--expected-cargo-version <version>"));
    assert!(text.contains("--max-view-entries <n> --max-view-bytes <n>"));
}

#[test]
fn local_patch_catalog_deduplicates_only_identical_owner_aliases() {
    let fixture = Fixture::new("patch-aliases");
    let workspace = fixture.directory("workspace");
    fixture.file(
        "workspace/packages/shared/Cargo.toml",
        b"[package]\nname = \"shared\"\nversion = \"0.1.0\"\n",
    );
    fixture.file(
        "workspace/packages/shared/src/lib.rs",
        b"pub fn shared() {}\n",
    );
    let identical = br#"[workspace]
members = ["packages/shared"]

[patch.crates-io]
shared = { path = "packages/shared" }

[patch."https://example.invalid/shared"]
shared = { path = "packages/shared" }
"#;
    let packages = graph::local_patch_packages(&workspace, identical)
        .expect("identical patch aliases have one owner");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "shared");

    fixture.file(
        "workspace/packages/substitute/Cargo.toml",
        b"[package]\nname = \"shared\"\nversion = \"0.1.0\"\n",
    );
    fixture.file(
        "workspace/packages/substitute/src/lib.rs",
        b"pub fn substituted() {}\n",
    );
    let divergent = br#"[workspace]
members = ["packages/shared"]

[patch.crates-io]
shared = { path = "packages/shared" }

[patch."https://example.invalid/shared"]
shared = { path = "packages/substitute" }
"#;
    let error = match graph::local_patch_packages(&workspace, divergent) {
        Ok(_) => panic!("same-name divergent patch owners must be refused"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        "local patch catalog maps one package name to divergent owners"
    );
}

#[test]
fn materialization_separates_selected_sources_from_the_resolver_catalog() {
    let fixture = Fixture::new("complete");
    let workspace = fixture.directory("workspace");
    let owner = fixture.directory("owner");
    let vendor_root = fixture.directory("registry");
    fixture.file("owner/src/lib.rs", b"pub fn selected() {}\n");
    fixture.file("owner/secret.rs", b"must not be copied\n");
    fixture.file(
        "workspace/Cargo.toml",
        br#"[workspace]
resolver = "2"
members = ["packages/local", "packages/not-selected"]

[workspace.dependencies]
local = { path = "packages/local" }
not-selected = { path = "packages/not-selected" }
serde = "1"

[patch.crates-io]
local = { path = "packages/local" }
not-selected = { path = "packages/not-selected" }
optional-local = { path = "packages/optional-local" }
"#,
    );
    fixture.file(
        "workspace/packages/local/Cargo.toml",
        b"[package]\nname = \"local\"\nversion = \"0.1.0\"\n",
    );
    symlink(owner.join("src"), workspace.join("packages/local/src")).expect("source symlink");
    fixture.file(
        "workspace/packages/not-selected/Cargo.toml",
        b"[package]\nname = \"not-selected\"\nversion = \"0.1.0\"\n",
    );
    fixture.file(
        "workspace/packages/not-selected/src/lib.rs",
        b"unselected\n",
    );
    fixture.file(
        "workspace/packages/optional-local/Cargo.toml",
        b"[package]\nname = \"optional-local\"\nversion = \"0.2.0\"\n",
    );
    fixture.file(
        "workspace/packages/optional-local/src/lib.rs",
        b"pub fn optional() {}\n",
    );
    fixture.file(
        "workspace/packages/optional-local/tests/not-a-production-target.rs",
        b"must not enter the resolver input\n",
    );
    // Flat, matching how locked_resolver_packages locates every registry
    // package directly at vendor_root.join("<name>-<version>"):
    // verify_archive_checksum derives the sibling cache/<name>-<version>.crate
    // from that same shape.
    let registry_manifest = fixture.file(
        "registry/dep-1.2.3/Cargo.toml",
        b"[package]\nname = \"dep\"\nversion = \"1.2.3\"\n",
    );
    fixture.file(
        "registry/dep-1.2.3/.cargo-checksum.json",
        b"{\"files\":{},\"package\":null}\n",
    );
    fixture.file("registry/dep-1.2.3/src/lib.rs", b"pub fn dependency() {}\n");
    // A real published crate can ship a zero-length file (an empty `build.rs`
    // companion module, a marker file); the whole-package copy and its fresh
    // checksum manifest must both still materialize it, not refuse the
    // package.
    fixture.file("registry/dep-1.2.3/build.rs", b"");
    fixture.file("cache/dep-1.2.3.crate", b"dep-archive-bytes\n");
    let resolver_manifest = fixture.file(
        "registry/resolver-3.0.0/Cargo.toml",
        b"[package]\nname = \"resolver\"\nversion = \"3.0.0\"\n",
    );
    fixture.file(
        "registry/resolver-3.0.0/.cargo-checksum.json",
        b"{\"files\":{},\"package\":null}\n",
    );
    fixture.file(
        "registry/resolver-3.0.0/src/lib.rs",
        b"pub fn resolver() {}\n",
    );
    fixture.file("cache/resolver-3.0.0.crate", b"resolver-archive-bytes\n");
    fixture.file(
        "registry/unused-9.0.0/Cargo.toml",
        b"[package]\nname = \"unused\"\nversion = \"9.0.0\"\n",
    );
    fixture.file("registry/unused-9.0.0/src/lib.rs", b"pub fn unused() {}\n");
    fixture.file(
        "registry/patch-dep-8.0.0/Cargo.toml",
        b"[package]\nname = \"patch-dep\"\nversion = \"8.0.0\"\n",
    );
    fixture.file(
        "registry/patch-dep-8.0.0/src/lib.rs",
        b"pub fn patch_dependency() {}\n",
    );
    fixture.file("cache/patch-dep-8.0.0.crate", b"patch-dep-archive-bytes\n");
    let local_id = "path+file:///fixture/local#0.1.0";
    let resolver_local_id = "path+file:///fixture/not-selected#0.1.0";
    let registry_id = "registry+https://example.invalid/index#dep@1.2.3";
    let resolver_id = "registry+https://example.invalid/index#resolver@3.0.0";
    let metadata = serde_json::json!({
        "packages": [
            {
                "id": local_id,
                "name": "local",
                "version": "0.1.0",
                "manifest_path": workspace.join("packages/local/Cargo.toml"),
                "source": null
            },
            {
                "id": registry_id,
                "name": "dep",
                "version": "1.2.3",
                "manifest_path": registry_manifest,
                "source": "registry+https://example.invalid/index"
            },
            {
                "id": "path+file:///fixture/not-selected#0.1.0",
                "name": "not-selected",
                "version": "0.1.0",
                "manifest_path": workspace.join("packages/not-selected/Cargo.toml"),
                "source": null,
                "targets": [{
                    "kind": ["lib"],
                    "src_path": workspace.join("packages/not-selected/src/lib.rs")
                }]
            },
            {
                "id": "registry+https://example.invalid/index#resolver@3.0.0",
                "name": "resolver",
                "version": "3.0.0",
                "manifest_path": resolver_manifest,
                "source": "registry+https://example.invalid/index",
                "targets": [{
                    "kind": ["lib"],
                    "src_path": fixture.root.join("registry/resolver-3.0.0/src/lib.rs")
                }]
            }
        ],
        "resolve": {
            "nodes": [
                {"id": local_id, "dependencies": [registry_id, resolver_id, resolver_local_id]},
                {"id": registry_id, "dependencies": []},
                {"id": resolver_id, "dependencies": []},
                {
                    "id": resolver_local_id,
                    "dependencies": []
                }
            ]
        }
    });
    let graph = serde_json::json!({
        "version": 1,
        "units": [
            {
                "pkg_id": local_id,
                "target": {
                    "kind": ["lib"],
                    "src_path": workspace.join("packages/local/src/lib.rs")
                }
            },
            {
                "pkg_id": registry_id,
                "target": {
                    "kind": ["lib"],
                    "src_path": fixture.root.join("registry/dep-1.2.3/src/lib.rs")
                }
            }
        ]
    });
    let metadata_path = fixture.file(
        "metadata.json",
        &serde_json::to_vec(&metadata).expect("metadata"),
    );
    let graph_path = fixture.file("graph.json", &serde_json::to_vec(&graph).expect("graph"));
    let lock = fixture.file(
        "Cargo.lock",
        b"version = 3\n\n[[package]]\nname = \"dep\"\nversion = \"1.2.3\"\nsource = \"registry+https://example.invalid/index\"\nchecksum = \"aedff91a6006659dd23e9c657f2e3f5d5d189e64f8cd7cf63a571b4870f5305c\"\n\n[[package]]\nname = \"not-selected\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"optional-local\"\nversion = \"0.2.0\"\ndependencies = [\"patch-dep\"]\n\n[[package]]\nname = \"patch-dep\"\nversion = \"8.0.0\"\nsource = \"registry+https://example.invalid/index\"\nchecksum = \"2e43da6fbedf42d0cf9652768e274f7d1a16b7cdf258f48ab1db3bcab829df69\"\n\n[[package]]\nname = \"resolver\"\nversion = \"3.0.0\"\nsource = \"registry+https://example.invalid/index\"\nchecksum = \"e22eebe0823c18486897a87550d2ef08f5fb3b25b1dd3df341ef9a2f9577fc1f\"\n\n[[package]]\nname = \"unused\"\nversion = \"9.0.0\"\nsource = \"registry+https://example.invalid/index\"\nchecksum = \"9999999999999999999999999999999999999999999999999999999999999999\"\n",
    );
    let config = fixture.file(
        "config.toml",
        b"[source.crates-io]\nreplace-with = \"vendored\"\n",
    );
    let manifest = workspace.join("Cargo.toml");
    let expected = BTreeMap::from([
        (
            "metadata",
            digest(&bounded_read(&metadata_path).expect("metadata input")),
        ),
        (
            "unit-graph",
            digest(&bounded_read(&graph_path).expect("graph input")),
        ),
        (
            "workspace-manifest",
            digest(&bounded_read(&manifest).expect("manifest input")),
        ),
        ("lock", digest(&bounded_read(&lock).expect("lock input"))),
        (
            "config",
            digest(&bounded_read(&config).expect("config input")),
        ),
    ]);
    let destination = fixture.root.join("materialized");
    let report = materialize(Options {
        metadata: metadata_path,
        unit_graph: graph_path,
        workspace,
        vendor_root,
        lock,
        config,
        destination: destination.clone(),
        owner_roots: vec![owner],
        expected,
        source_limit: TreeLimit {
            entries: 64,
            bytes: 64 * 1024,
        },
        vendor_limit: TreeLimit {
            entries: 64,
            bytes: 64 * 1024,
        },
        input_limit: TreeLimit {
            entries: 128,
            bytes: 128 * 1024,
        },
    })
    .expect("materialization");

    let copied = destination.join("source/packages/local/src/lib.rs");
    assert_eq!(
        fs::read(&copied).expect("copied source"),
        b"pub fn selected() {}\n"
    );
    assert!(
        !fs::symlink_metadata(copied)
            .expect("copied metadata")
            .file_type()
            .is_symlink()
    );
    assert!(
        destination
            .join("source/packages/not-selected/src/lib.rs")
            .is_file()
    );
    assert!(!destination.join("source/secret.rs").exists());
    assert!(destination.join("vendor/dep-1.2.3/src/lib.rs").is_file());
    let empty_copy = destination.join("vendor/dep-1.2.3/build.rs");
    assert!(
        empty_copy.is_file(),
        "a zero-length package file must still be materialized"
    );
    assert!(fs::read(&empty_copy).expect("copied empty file").is_empty());
    assert!(
        destination
            .join("vendor/resolver-3.0.0/src/lib.rs")
            .is_file()
    );
    assert!(
        !destination.join("vendor/unused-9.0.0").exists(),
        "an unrelated row retained by Cargo.lock must not become a resolver input"
    );
    assert!(
        destination
            .join("vendor/patch-dep-8.0.0/src/lib.rs")
            .is_file(),
        "a retained local patch's locked production dependency must enter the resolver input"
    );
    assert_eq!(report.local_packages, [local_id]);
    assert_eq!(
        report
            .resolver_only_local_packages
            .iter()
            .map(|id| id.rsplit('/').next().unwrap())
            .collect::<Vec<_>>(),
        ["not-selected#0.1.0", "optional-local#0.2.0"]
    );
    assert_eq!(report.registry_packages, [registry_id]);
    assert_eq!(
        report.resolver_only_registry_packages,
        [
            "registry+https://example.invalid/index#patch-dep@8.0.0",
            "registry+https://example.invalid/index#resolver@3.0.0",
        ]
    );
    assert!(destination.join("materialization.json").is_file());
    assert!(
        destination
            .join("source/packages/optional-local/src/lib.rs")
            .is_file(),
        "unused local patch is still a Cargo resolver input"
    );
    assert!(
        !destination
            .join("source/packages/optional-local/tests")
            .exists(),
        "non-production resolver files must not consume the sealed input budget"
    );

    let narrowed =
        fs::read_to_string(destination.join("source/Cargo.toml")).expect("narrow manifest");
    let narrowed: toml::Value = toml::from_str(&narrowed).expect("narrowed manifest");
    assert_eq!(
        narrowed["workspace"]["members"].as_array().unwrap(),
        &[toml::Value::String("packages/local".into())]
    );
    assert!(
        narrowed["workspace"]["exclude"]
            .as_array()
            .unwrap()
            .contains(&toml::Value::String("packages/not-selected".into()))
    );
    assert_eq!(
        narrowed["workspace"]["dependencies"]["serde"].as_str(),
        Some("1")
    );
}

#[test]
fn copied_tree_refuses_symlink_outside_selected_owners() {
    let fixture = Fixture::new("escape");
    let source = fixture.directory("source");
    let allowed = fixture.directory("allowed");
    let outside = fixture.file("outside/secret", b"secret\n");
    symlink(outside, source.join("escape")).expect("escape symlink");
    let mut writer = TreeWriter::create(
        fixture.root.join("output"),
        TreeLimit {
            entries: 16,
            bytes: 1024,
        },
    )
    .expect("writer");
    let error = writer
        .copy_tree(&source, Path::new("package"), &[allowed])
        .expect_err("escape must be refused");
    assert!(error.contains("escaped selected owners"));
}

#[test]
fn a_zero_length_file_in_a_copied_tree_is_not_refused() {
    // A real published crate can legitimately ship an empty file (an empty
    // `build.rs` companion module, a marker file); whole-package copies (see
    // `build_inputs/registry.rs::copy_selected`) must materialize it, not
    // refuse the whole package.
    let fixture = Fixture::new("empty-file");
    let source = fixture.directory("source");
    fixture.file("source/build.rs", b"");
    fixture.file("source/src/lib.rs", b"pub fn present() {}\n");
    let mut writer = TreeWriter::create(
        fixture.root.join("output"),
        TreeLimit {
            entries: 16,
            bytes: 1024,
        },
    )
    .expect("writer");
    writer
        .copy_tree(&source, Path::new("package"), &[])
        .expect("a zero-length file must copy, not refuse the package");
    let copied = fs::read(fixture.root.join("output/package/build.rs")).expect("copied file");
    assert!(copied.is_empty());
}

#[test]
fn resolver_manifest_uses_explicit_workspace_without_becoming_a_member() {
    let rendered = attach_resolver_package_to_workspace(
        b"[package]\nname = \"support\"\nversion.workspace = true\nedition.workspace = true\n",
    )
    .expect("resolver manifest");
    let manifest: toml::Value =
        toml::from_str(std::str::from_utf8(&rendered).expect("UTF-8 rendered manifest"))
            .expect("parse rendered manifest");
    assert_eq!(manifest["package"]["workspace"].as_str(), Some("../.."));
    assert_eq!(
        manifest["package"]["version"]["workspace"].as_bool(),
        Some(true)
    );
    assert_eq!(
        manifest["package"]["edition"]["workspace"].as_bool(),
        Some(true)
    );
}

#[test]
fn resolver_target_selection_keeps_concrete_library_crate_types() {
    let package = serde_json::json!({
        "targets": [
            {"kind": ["rlib", "cdylib"], "src_path": "/owner/src/lib.rs"},
            {"kind": ["custom-build"], "src_path": "/owner/build.rs"},
            {"kind": ["test"], "src_path": "/owner/tests/integration.rs"}
        ]
    });
    assert_eq!(
        super::graph::target_entries(&package).expect("target selection"),
        [
            PathBuf::from("/owner/build.rs"),
            PathBuf::from("/owner/src/lib.rs")
        ]
    );
}

#[test]
fn exact_digest_mismatch_fails_before_creating_staging() {
    let fixture = Fixture::new("digest");
    let workspace = fixture.directory("workspace");
    fixture.file("workspace/Cargo.toml", b"[workspace]\nmembers = []\n");
    let metadata = fixture.file(
        "metadata.json",
        b"{\"packages\":[],\"resolve\":{\"nodes\":[]}}",
    );
    let graph = fixture.file("graph.json", b"{\"version\":1,\"units\":[]}");
    let lock = fixture.file("Cargo.lock", b"version = 3\n");
    let config = fixture.file("config.toml", b"[net]\noffline = true\n");
    let destination = fixture.root.join("materialized");
    let result = materialize(Options {
        metadata,
        unit_graph: graph,
        workspace: workspace.clone(),
        vendor_root: fixture.directory("registry"),
        lock,
        config,
        destination: destination.clone(),
        owner_roots: vec![workspace],
        expected: BTreeMap::from([
            ("metadata", "0".repeat(64)),
            ("unit-graph", "0".repeat(64)),
            ("workspace-manifest", "0".repeat(64)),
            ("lock", "0".repeat(64)),
            ("config", "0".repeat(64)),
        ]),
        source_limit: TreeLimit {
            entries: 16,
            bytes: 1024,
        },
        vendor_limit: TreeLimit {
            entries: 16,
            bytes: 1024,
        },
        input_limit: TreeLimit {
            entries: 32,
            bytes: 2048,
        },
    });
    let error = match result {
        Ok(_) => panic!("digest substitution must be refused"),
        Err(error) => error,
    };
    assert!(error.contains("exact input digests differ"));
    assert!(!destination.exists());
    assert_eq!(
        fs::read_dir(&fixture.root)
            .expect("fixture members")
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains("materializing"))
            .count(),
        0
    );
}
