use super::*;
use std::{fs, os::unix::fs::PermissionsExt, time::SystemTime};

struct Temporary(PathBuf);

impl Temporary {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sim-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn selection(source: &Path, hash: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "sim.sealed-resource-selection/v1",
        "roots": [{
            "name": "toolchain",
            "guest_path": "/toolchain",
            "maximum_entries": 8,
            "maximum_bytes": 4096,
            "files": [{
                "source": source,
                "destination": "bin/tool",
                "sha256": hash,
                "executable": true
            }]
        }]
    }))
    .unwrap()
}

#[test]
fn exact_selection_materializes_regular_create_new_resource_and_report() {
    let root = Temporary::new("sealed-resource");
    let owner = root.0.join("owner");
    let output = root.0.join("output");
    fs::create_dir(&owner).unwrap();
    let source = owner.join("tool");
    fs::write(&source, b"selected").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    let bytes = selection(&source, &digest(b"selected"));
    let spec = root.0.join("selection.json");
    fs::write(&spec, &bytes).unwrap();
    let options = Options {
        selection: spec,
        expected_selection_sha256: digest(&bytes),
        destination: output.clone(),
        owner_roots: vec![owner],
    };
    let report = materialize(options).unwrap();
    assert_eq!(report.roots.len(), 1);
    assert_eq!(
        fs::read(output.join("toolchain/bin/tool")).unwrap(),
        b"selected"
    );
    assert!(output.join("materialization.json").is_file());
    assert!(
        materialize(Options {
            selection: root.0.join("selection.json"),
            expected_selection_sha256: digest(&bytes),
            destination: output,
            owner_roots: vec![root.0.join("owner")],
        })
        .is_err()
    );
}

#[test]
fn capture_freezes_owned_content_and_mode_before_materialization() {
    let root = Temporary::new("sealed-resource-capture");
    let owner = root.0.join("owner");
    fs::create_dir(&owner).unwrap();
    let source = owner.join("tool");
    fs::write(&source, b"selected").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    let plan = serde_json::to_vec(&serde_json::json!({
        "schema": "sim.sealed-resource-capture-plan/v1",
        "roots": [{
            "name": "toolchain",
            "guest_path": "/toolchain",
            "maximum_entries": 8,
            "maximum_bytes": 4096,
            "files": [{"source": source, "destination": "bin/tool"}]
        }]
    }))
    .unwrap();
    let plan_path = root.0.join("plan.json");
    let selection_path = root.0.join("selection.json");
    fs::write(&plan_path, &plan).unwrap();
    capture(CaptureOptions {
        plan: plan_path,
        expected_plan_sha256: digest(&plan),
        selection: selection_path.clone(),
        owner_roots: vec![owner],
    })
    .unwrap();
    let selection: Selection = serde_json::from_slice(&fs::read(selection_path).unwrap()).unwrap();
    assert_eq!(selection.roots[0].files[0].sha256, digest(b"selected"));
    assert!(selection.roots[0].files[0].executable);
}

#[test]
fn selection_refuses_substitution_escape_duplicate_and_mode_change() {
    let root = Temporary::new("sealed-resource-refusal");
    let owner = root.0.join("owner");
    fs::create_dir(&owner).unwrap();
    let source = owner.join("tool");
    fs::write(&source, b"selected").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    let hash = digest(b"selected");
    let bytes = selection(&source, &hash);
    let decoded: Selection = serde_json::from_slice(&bytes).unwrap();
    let owners = vec![owner.canonicalize().unwrap()];
    validate_selection(&decoded, &owners).unwrap();

    fs::write(&source, b"substituted").unwrap();
    let spec = root.0.join("selection.json");
    fs::write(&spec, &bytes).unwrap();
    assert!(
        materialize(Options {
            selection: spec,
            expected_selection_sha256: digest(&bytes),
            destination: root.0.join("substituted"),
            owner_roots: vec![owner.clone()],
        })
        .is_err()
    );

    let outside = root.0.join("outside");
    fs::write(&outside, b"outside").unwrap();
    let escaped = selection(&outside, &digest(b"outside"));
    let escaped: Selection = serde_json::from_slice(&escaped).unwrap();
    assert!(validate_selection(&escaped, &owners).is_err());

    let mut duplicate: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let file = duplicate["roots"][0]["files"][0].clone();
    duplicate["roots"][0]["files"] = serde_json::json!([file.clone(), file]);
    let duplicate: Selection = serde_json::from_value(duplicate).unwrap();
    assert!(validate_selection(&duplicate, &owners).is_err());
}

#[test]
fn capture_streams_large_selected_images_and_enforces_the_root_total() {
    use std::fs::File;

    let root = Temporary::new("sealed-resource-large-image");
    let owner = root.0.join("owner");
    fs::create_dir(&owner).unwrap();
    let source = owner.join("cargo");
    let length = 17 * 1024 * 1024;
    File::create(&source)
        .unwrap()
        .set_len(length as u64)
        .unwrap();
    let plan = |maximum_bytes| {
        serde_json::to_vec(&serde_json::json!({
            "schema": "sim.sealed-resource-capture-plan/v1",
            "roots": [{
                "name": "toolchain",
                "guest_path": "/toolchain",
                "maximum_entries": 8,
                "maximum_bytes": maximum_bytes,
                "files": [{"source": source, "destination": "bin/cargo"}]
            }]
        }))
        .unwrap()
    };

    let accepted = plan(18 * 1024 * 1024);
    let accepted_path = root.0.join("accepted-plan.json");
    fs::write(&accepted_path, &accepted).unwrap();
    capture(CaptureOptions {
        plan: accepted_path,
        expected_plan_sha256: digest(&accepted),
        selection: root.0.join("selection.json"),
        owner_roots: vec![owner.clone()],
    })
    .unwrap();

    let refused = plan(16 * 1024 * 1024);
    let refused_path = root.0.join("refused-plan.json");
    fs::write(&refused_path, &refused).unwrap();
    assert!(
        capture(CaptureOptions {
            plan: refused_path,
            expected_plan_sha256: digest(&refused),
            selection: root.0.join("refused-selection.json"),
            owner_roots: vec![owner],
        })
        .is_err()
    );
}
