use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};
use sim_codec_index::{IndexCodec, IndexForm};
use sim_index_core::{
    AnchorId, DiscoveredAnchor, DiscoveredSpecimen, DiscoveredSurface, FeatureId, FeatureRecord,
    IndexDoc, RouteId, RouteRecord, RouteStep, SpecimenId, SubjectId, SubjectRecord, SurfaceId,
    key::CanonicalFeatureKey,
};
use sim_kernel::EncodePosition;

use super::index::{AtelierIndexOptions, atelier_index};

#[test]
fn index_reports_missing_dirty_and_missing_cargo_repos() {
    let fixture = IndexFixture::new("diagnostics");
    fixture.write_manifest(&[
        repo_row(
            "clean",
            "code",
            "clean",
            true,
            &["sim-clean"],
            &["."],
            "aaaa",
        ),
        repo_row(
            "missing",
            "code",
            "missing",
            true,
            &["sim-missing"],
            &["."],
            "bbbb",
        ),
        repo_row(
            "no-cargo",
            "code",
            "no-cargo",
            true,
            &["sim-no-cargo"],
            &["."],
            "cccc",
        ),
        repo_row(
            "dirty",
            "code",
            "dirty",
            true,
            &["sim-dirty"],
            &["."],
            "dddd",
        ),
    ]);
    fixture
        .repo("clean")
        .cargo("sim-clean")
        .readme("clean repo")
        .git_clean();
    fs::create_dir_all(fixture.repo("no-cargo").path()).unwrap();
    fixture
        .repo("dirty")
        .cargo("sim-dirty")
        .readme("dirty repo")
        .git_clean();
    fs::write(fixture.repo("dirty").path().join("dirty.txt"), "changed").unwrap();

    let index = fixture.index();
    let diagnostics = index["diagnostics"].as_array().unwrap();

    assert!(has_diagnostic(diagnostics, "repo-missing", "missing"));
    assert!(has_diagnostic(
        diagnostics,
        "cargo-toml-missing",
        "no-cargo"
    ));
    assert!(has_diagnostic(diagnostics, "repo-dirty", "dirty"));
}

#[test]
fn index_excludes_meta_workspace_chunk_paths() {
    let fixture = IndexFixture::new("meta");
    fixture.write_manifest(&[repo_row(
        "clean",
        "code",
        "clean",
        true,
        &["sim-clean"],
        &[".", ".meta-workspace/packages/sim-clean"],
        "aaaa",
    )]);
    fixture
        .repo("clean")
        .cargo("sim-clean")
        .readme("clean repo")
        .git_clean();
    fs::create_dir_all(
        fixture
            .repo("clean")
            .path()
            .join(".meta-workspace/packages/sim-clean/src"),
    )
    .unwrap();
    fs::write(
        fixture
            .repo("clean")
            .path()
            .join(".meta-workspace/packages/sim-clean/src/lib.rs"),
        "pub fn generated() {}\n",
    )
    .unwrap();

    let index = fixture.index();
    assert_eq!(
        index["source_policy"]["chunks_include_meta_workspace"],
        Value::Bool(false)
    );
    for chunk in index["chunks"].as_array().unwrap() {
        let file = chunk["span"]["file"].as_str().unwrap();
        assert!(!file.contains(".meta-workspace"));
    }
}

#[test]
fn index_chunk_ids_are_stable_for_unchanged_sources() {
    let fixture = IndexFixture::new("stable");
    fixture.write_manifest(&[repo_row(
        "clean",
        "code",
        "clean",
        true,
        &["sim-clean"],
        &["."],
        "aaaa",
    )]);
    fixture
        .repo("clean")
        .cargo("sim-clean")
        .readme("# Clean\n\nThe clean repo has docs.")
        .rust_lib(
            "sim-clean",
            "//! Crate docs.\n\n/// Alpha docs.\npub fn alpha() {}\n",
        )
        .git_clean();

    let first = chunk_ids(&fixture.index());
    let second = chunk_ids(&fixture.index());

    assert_eq!(first, second);
    assert!(first.iter().any(|id| id.contains("readme")));
    assert!(first.iter().any(|id| id.contains("alpha")));
}

#[test]
fn index_attaches_rust_facts_to_rust_chunks() {
    let fixture = IndexFixture::new("rust-facts");
    fixture.write_manifest(&[repo_row(
        "clean",
        "code",
        "clean",
        true,
        &["sim-clean"],
        &["."],
        "aaaa",
    )]);
    fixture
        .repo("clean")
        .cargo("sim-clean")
        .readme("clean repo")
        .rust_lib(
            "sim-clean",
            "//! Crate docs.\n\n/// Alpha source docs.\npub fn alpha() {}\n",
        )
        .rust_test("tests/alpha.rs", "#[test]\nfn alpha_links() { alpha(); }\n")
        .rustdoc_json(
            "sim_clean",
            json!({
                "index": {
                    "0:0": {
                        "name": "alpha",
                        "attrs": ["#[cfg(feature = \"fast\")]"],
                        "docs": "Alpha rustdoc docs.",
                        "span": {
                            "filename": "src/lib.rs",
                            "begin": [4, 1]
                        },
                        "inner": {
                            "function": {}
                        }
                    }
                }
            }),
        )
        .git_clean();

    let index = fixture.index();
    let chunk = index["chunks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|chunk| {
            chunk["kind"] == "rust-fn" && chunk["text"].as_str().unwrap().contains("alpha")
        })
        .unwrap();
    let rust = &chunk["rust"];

    assert_eq!(rust["repo"], "clean");
    assert_eq!(rust["crate"], "sim-clean");
    assert_eq!(rust["source"]["file"], "src/lib.rs");
    assert_eq!(rust["docs_summary"], "Alpha rustdoc docs.");
    assert_eq!(rust["feature_gates"], json!(["fast"]));
    assert_eq!(rust["linked_tests"][0]["file"], "tests/alpha.rs");
    assert_eq!(rust["browse"]["kind"], "sim-browse-object");
    assert_eq!(index["rust"]["items"].as_array().unwrap().len(), 1);
}

#[test]
fn index_imports_sim_index_graph_units() {
    let fixture = IndexFixture::new("sim-index-graph");
    fixture.write_manifest(&[repo_row(
        "sim-say",
        "frontpage",
        "sim-say",
        false,
        &[],
        &[],
        "aaaa",
    )]);
    fixture.repo("sim-say").index_sx(&fixture_doc()).git_clean();

    let index = fixture.index();
    let units = index["units"].as_array().unwrap();
    assert!(graph_unit(units, "feature/demo/parser", "feature").is_some());
    assert!(graph_unit(units, "crate/demo", "package").is_some());
    assert!(graph_unit(units, "surface/demo/parser", "surface").is_some());
    assert!(graph_unit(units, "grammar/demo-parser", "grammar").is_some());
    assert!(graph_unit(units, "recipe/demo/parser", "specimen").is_some());
    assert!(graph_unit(units, "route/demo/parser", "route").is_some());

    let chunks = index["chunks"].as_array().unwrap();
    let route = chunks
        .iter()
        .find(|chunk| chunk["graph_id"] == "route/demo/parser")
        .expect("route chunk");
    assert!(
        string_array(&route["panels"]).contains(&"reuse-route".to_owned()),
        "route chunks advertise the reuse route panel"
    );
}

fn has_diagnostic(diagnostics: &[Value], kind: &str, repo: &str) -> bool {
    diagnostics
        .iter()
        .any(|item| item["kind"] == kind && item["repo"] == repo)
}

fn graph_unit<'a>(units: &'a [Value], id: &str, kind: &str) -> Option<&'a Value> {
    units
        .iter()
        .find(|unit| unit["graph_id"] == id && unit["kind"] == kind)
}

fn fixture_doc() -> IndexDoc {
    let mut doc = IndexDoc::public("atelier-index-test");
    doc.subjects.push(SubjectRecord {
        id: SubjectId::new("crate/demo"),
        kind: "crate".to_owned(),
        title: "demo".to_owned(),
    });
    doc.anchors.push(DiscoveredAnchor {
        id: AnchorId::new("anchor/demo/parser-decoder"),
        subject: SubjectId::new("crate/demo"),
        kind: "decoder".to_owned(),
    });
    doc.surfaces.push(DiscoveredSurface {
        id: SurfaceId::new("surface/demo/parser"),
        subject: SubjectId::new("crate/demo"),
        kind: "syntax".to_owned(),
    });
    doc.specimens.push(DiscoveredSpecimen {
        id: SpecimenId::new("recipe/demo/parser"),
        subject: SubjectId::new("crate/demo"),
        kind: "recipe".to_owned(),
        path: "recipes/parser/recipe.toml".to_owned(),
        language: Some("toml".to_owned()),
        runnable: true,
        checked: true,
        checked_by: Some("xtask check-recipes".to_owned()),
        doc_anchor: None,
    });
    doc.features.push(FeatureRecord {
        id: FeatureId::new("feature/demo/parser"),
        key: CanonicalFeatureKey::new("crate/demo/feature-demo-parser"),
        subject: SubjectId::new("crate/demo"),
        title: "Demo parser".to_owned(),
        summary: "Parse a demo language.".to_owned(),
        anchors: vec![AnchorId::new("anchor/demo/parser-decoder")],
        surfaces: vec![SurfaceId::new("surface/demo/parser")],
        specimens: vec![SpecimenId::new("recipe/demo/parser")],
        grammar_contracts: vec![sim_index_core::GrammarContract {
            id: "grammar/demo-parser".to_owned(),
            decoder: Some(AnchorId::new("anchor/demo/parser-decoder")),
            encoder: None,
            surface: Some(SurfaceId::new("surface/demo/parser")),
            round_trip: true,
        }],
        doc_anchor: None,
    });
    doc.routes.push(RouteRecord {
        id: RouteId::new("route/demo/parser"),
        title: "Parse a demo language".to_owned(),
        audiences: vec!["code".to_owned(), "framework".to_owned()],
        steps: vec![
            RouteStep::Feature {
                id: FeatureId::new("feature/demo/parser"),
                why: "The parser feature is already present.".to_owned(),
            },
            RouteStep::Specimen {
                id: SpecimenId::new("recipe/demo/parser"),
                why: "Run the checked parser recipe.".to_owned(),
            },
        ],
        doc_anchor: None,
    });
    doc
}

fn chunk_ids(index: &Value) -> Vec<String> {
    index["chunks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|chunk| chunk["id"].as_str().unwrap().to_owned())
        .collect()
}

fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn repo_row(
    name: &str,
    kind: &str,
    local_path: &str,
    contains_code: bool,
    crates: &[&str],
    sources: &[&str],
    commit: &str,
) -> String {
    format!(
        r#"
[[repo]]
name = "{name}"
kind = "{kind}"
local_path = "{local_path}"
contains_code = {contains_code}
crate_names = [{crates}]
source_paths = [{sources}]
validation_command = "cargo test"
docs_command = "cargo run -p xtask -- simdoc --check"
commit = "{commit}"
"#,
        crates = quoted_list(crates),
        sources = quoted_list(sources),
    )
}

fn quoted_list(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

struct IndexFixture {
    root: PathBuf,
}

impl IndexFixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sim-tooling-index-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write_manifest(&self, rows: &[String]) {
        fs::write(self.root.join("repos.toml"), rows.join("\n")).unwrap();
    }

    fn repo(&self, name: &str) -> RepoFixture {
        RepoFixture {
            root: self.root.join(name),
        }
    }

    fn index(&self) -> Value {
        atelier_index(AtelierIndexOptions {
            control_root: self.root.clone(),
            repos_manifest: Some(self.root.join("repos.toml")),
            cache_dir: Some(self.root.join(".sim/atelier/index")),
            check: false,
            max_chunk_bytes: 64,
        })
        .unwrap()
        .index
    }
}

impl Drop for IndexFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Clone)]
struct RepoFixture {
    root: PathBuf,
}

impl RepoFixture {
    fn path(&self) -> &Path {
        &self.root
    }

    fn cargo(&self, name: &str) -> Self {
        fs::create_dir_all(&self.root).unwrap();
        fs::write(
            self.root.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        )
        .unwrap();
        self.clone()
    }

    fn readme(&self, text: &str) -> Self {
        fs::create_dir_all(&self.root).unwrap();
        fs::write(self.root.join("README.md"), text).unwrap();
        self.clone()
    }

    fn rust_lib(&self, _name: &str, text: &str) -> Self {
        fs::create_dir_all(self.root.join("src")).unwrap();
        fs::write(self.root.join("src/lib.rs"), text).unwrap();
        self.clone()
    }

    fn rust_test(&self, path: &str, text: &str) -> Self {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        self.clone()
    }

    fn index_sx(&self, doc: &IndexDoc) -> Self {
        let path = self.root.join("docs/index/index.sx");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = IndexCodec
            .encode(doc, EncodePosition::Data, IndexForm::Json)
            .unwrap();
        fs::write(path, source).unwrap();
        self.clone()
    }

    fn rustdoc_json(&self, crate_file: &str, value: serde_json::Value) -> Self {
        let doc_dir = self.root.join("target/doc");
        fs::create_dir_all(&doc_dir).unwrap();
        fs::write(
            doc_dir.join(format!("{crate_file}.json")),
            serde_json::to_string(&value).unwrap(),
        )
        .unwrap();
        self.clone()
    }

    fn git_clean(&self) -> Self {
        run_git(&self.root, &["init"]);
        run_git(&self.root, &["config", "user.email", "noreply@example.com"]);
        run_git(&self.root, &["config", "user.name", "Atelier Test"]);
        run_git(&self.root, &["add", "."]);
        run_git(&self.root, &["commit", "-m", "init"]);
        self.clone()
    }
}

fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
