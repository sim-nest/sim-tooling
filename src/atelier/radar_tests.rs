use std::{fs, path::PathBuf};

use serde_json::json;

use super::radar::{AtelierRadarOptions, RadarQuery, atelier_radar};

#[test]
fn radar_returns_known_operation_as_top_hint() {
    let fixture = RadarFixture::new("top");
    fixture.repo_file(
        "sim-alpha",
        "src/lib.rs",
        "//! Alpha\n\n/// Runs cargo validation command.\npub fn validate_workspace() {}\n",
    );
    fixture.repo_file(
        "sim-alpha",
        "README.md",
        "# Alpha\n\nbanana mango smoothie\n",
    );
    fixture.write_index(vec![
        chunk_with_rust(
            "validation",
            "sim-alpha",
            Some("sim-alpha"),
            "rust-fn",
            "src/lib.rs",
            3,
            "signature: fn validate_workspace\ndocs:\nRuns cargo validation command.",
            &["validation"],
            &[],
            json!({
                "ide_object_id": "ide://rust/sim-alpha/sim-alpha/crate/fn/src-lib-rs@3",
                "feature_gates": ["validation"],
                "crate_features": ["default"],
                "linked_tests": [{
                    "repo": "sim-alpha",
                    "file": "tests/validation.rs",
                    "line": 2
                }]
            }),
        ),
        chunk(
            "fruit",
            "sim-alpha",
            None,
            "readme",
            "README.md",
            1,
            "banana mango smoothie",
            &[],
            &[],
        ),
    ]);

    let report = fixture.radar(RadarQuery {
        text: "validation command".to_owned(),
        limit: 2,
        ..RadarQuery::default()
    });

    assert_eq!(report.hints[0].chunk_id, "validation");
    assert_eq!(
        report.hints[0].rust.as_ref().unwrap()["ide_object_id"],
        "ide://rust/sim-alpha/sim-alpha/crate/fn/src-lib-rs@3"
    );
    assert!((0.0..=1.0).contains(&report.hints[0].confidence));
    assert!(!report.stale_index);
}

#[test]
fn radar_capability_filter_excludes_non_matching_chunks() {
    let fixture = RadarFixture::new("capability");
    fixture.repo_file(
        "sim-alpha",
        "src/lib.rs",
        "pub fn validate_workspace() {}\n",
    );
    fixture.repo_file("sim-beta", "src/lib.rs", "pub fn encode_json() {}\n");
    fixture.write_index(vec![
        chunk(
            "validation",
            "sim-alpha",
            Some("sim-alpha"),
            "rust-fn",
            "src/lib.rs",
            1,
            "cargo validation command",
            &["validation"],
            &[],
        ),
        chunk(
            "codec",
            "sim-beta",
            Some("sim-beta"),
            "rust-fn",
            "src/lib.rs",
            1,
            "json codec encoder",
            &["codec"],
            &["json"],
        ),
    ]);

    let report = fixture.radar(RadarQuery {
        text: "command codec".to_owned(),
        capability: Some("validation".to_owned()),
        limit: 4,
        ..RadarQuery::default()
    });

    assert_eq!(report.hints.len(), 1);
    assert_eq!(report.hints[0].chunk_id, "validation");
}

#[test]
fn radar_drops_stale_spans_and_sets_stale_flag() {
    let fixture = RadarFixture::new("stale");
    fixture.repo_file("sim-alpha", "src/lib.rs", "pub fn alive() {}\n");
    fixture.write_index(vec![chunk(
        "stale",
        "sim-alpha",
        Some("sim-alpha"),
        "rust-fn",
        "src/missing.rs",
        1,
        "validation command",
        &["validation"],
        &[],
    )]);

    let report = fixture.radar(RadarQuery {
        text: "validation".to_owned(),
        limit: 4,
        ..RadarQuery::default()
    });

    assert!(report.hints.is_empty());
    assert!(report.stale_index);
    assert_eq!(report.stale_chunk_ids, vec!["stale"]);
}

#[test]
fn radar_returns_index_graph_rows_for_bridge_packet_guidance() {
    let fixture = RadarFixture::new("graph");
    fixture.repo_file("sim-alpha", "docs/index/index.sx", "graph\n");
    fixture.write_index(vec![
        graph_chunk(
            "feature/sim-codecs/bridge-packet-codec",
            "feature",
            "Bridge packet codec grammar already exists and carries a runnable specimen.",
            &["already-exists", "run-this-example"],
            &["grammar/bridge-packet", "recipe/sim-codecs/bridge-packet"],
        ),
        graph_chunk(
            "grammar/bridge-packet",
            "grammar",
            "Grammar for bridge packets with closed round-trip evidence.",
            &["already-exists"],
            &["feature/sim-codecs/bridge-packet-codec"],
        ),
        graph_chunk(
            "recipe/sim-codecs/bridge-packet",
            "specimen",
            "Run this example for bridge packet grammar.",
            &["run-this-example"],
            &["feature/sim-codecs/bridge-packet-codec"],
        ),
        graph_chunk(
            "route/add-model-facing-packet-workflow",
            "route",
            "Reuse route for bridge packet model workflows.",
            &["reuse-route"],
            &["feature/sim-codecs/bridge-packet-codec"],
        ),
    ]);

    let report = fixture.radar(RadarQuery {
        text: "grammar for bridge packets".to_owned(),
        limit: 4,
        ..RadarQuery::default()
    });
    let graph_ids = report
        .hints
        .iter()
        .filter_map(|hint| hint.graph_id.as_deref())
        .collect::<Vec<_>>();

    assert!(graph_ids.contains(&"feature/sim-codecs/bridge-packet-codec"));
    assert!(graph_ids.contains(&"grammar/bridge-packet"));
    assert!(graph_ids.contains(&"recipe/sim-codecs/bridge-packet"));
    assert!(graph_ids.contains(&"route/add-model-facing-packet-workflow"));
    assert!(
        report
            .hints
            .iter()
            .any(|hint| hint.panels.contains(&"reuse-route".to_owned()))
    );
    assert!(
        report
            .hints
            .iter()
            .any(|hint| hint.panels.contains(&"run-this-example".to_owned()))
    );
}

// Test fixture builder: many fields map 1:1 to chunk metadata; grouping them into
// a struct would obscure the per-field test inputs.
#[allow(clippy::too_many_arguments)]
fn chunk(
    id: &str,
    repo: &str,
    crate_name: Option<&str>,
    kind: &str,
    file: &str,
    line: usize,
    text: &str,
    capabilities: &[&str],
    codecs: &[&str],
) -> serde_json::Value {
    json!({
        "id": id,
        "repo": repo,
        "crate": crate_name,
        "kind": kind,
        "span": {
            "file": file,
            "line": line,
        },
        "text": text,
        "heading_path": [],
        "source_unit": format!("{repo}/{kind}/{id}"),
        "capabilities": capabilities,
        "codecs": codecs,
        "pin": "aaaa",
        "chunker": "sim-codec-doc/doc/chunk-recursive",
    })
}

fn graph_chunk(
    graph_id: &str,
    kind: &str,
    text: &str,
    panels: &[&str],
    related_ids: &[&str],
) -> serde_json::Value {
    let mut value = chunk(
        graph_id,
        "sim-alpha",
        None,
        kind,
        "docs/index/index.sx",
        1,
        text,
        &[],
        &[],
    );
    value["graph_id"] = json!(graph_id);
    value["graph_kind"] = json!(kind);
    value["panels"] = json!(panels);
    value["related_ids"] = json!(related_ids);
    value
}

#[allow(clippy::too_many_arguments)]
fn chunk_with_rust(
    id: &str,
    repo: &str,
    crate_name: Option<&str>,
    kind: &str,
    file: &str,
    line: usize,
    text: &str,
    capabilities: &[&str],
    codecs: &[&str],
    rust: serde_json::Value,
) -> serde_json::Value {
    let mut value = chunk(
        id,
        repo,
        crate_name,
        kind,
        file,
        line,
        text,
        capabilities,
        codecs,
    );
    value["rust"] = rust;
    value
}

struct RadarFixture {
    root: PathBuf,
}

impl RadarFixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sim-tooling-radar-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn repo_file(&self, repo: &str, path: &str, text: &str) {
        let path = self.root.join(repo).join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn write_index(&self, chunks: Vec<serde_json::Value>) {
        let repos = ["sim-alpha", "sim-beta"]
            .into_iter()
            .map(|name| {
                json!({
                    "name": name,
                    "kind": "code",
                    "local_path": name,
                    "contains_code": true,
                    "crates": [name],
                    "source_paths": ["."],
                    "validation_command": "cargo test",
                    "docs_command": "cargo run -p xtask -- simdoc --check",
                    "pin": "aaaa",
                    "status": "clean",
                })
            })
            .collect::<Vec<_>>();
        fs::write(
            self.index_file(),
            serde_json::to_string_pretty(&json!({
                "schema": "sim.atelier.constellation-index.v1",
                "repos": repos,
                "chunks": chunks,
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn radar(&self, query: RadarQuery) -> super::radar::RadarReport {
        atelier_radar(AtelierRadarOptions {
            control_root: self.root.clone(),
            index_file: Some(self.index_file()),
            query,
            json: false,
        })
        .unwrap()
    }

    fn index_file(&self) -> PathBuf {
        self.root.join("index.json")
    }
}

impl Drop for RadarFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
