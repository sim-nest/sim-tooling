// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    env, fs,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;

use super::*;

// conformance: provenance records what executed, and only regeneration and
// validation commands the repository declares or the encoder can prove.

/// The sim-tooling checkout that contains this encoder.
fn tooling_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

#[test]
fn declared_commands_are_recorded_verbatim() {
    let metadata = json!({"metadata": {"sim": {
        "docs-command": "cargo run -p xtask -- simdoc",
        "validation-commands": ["cargo test", "cargo run -p xtask -- simdoc --check"],
    }}});

    let recorded = regeneration(&tooling_root(), &metadata).unwrap();

    assert_eq!(
        recorded.regeneration_command,
        "cargo run -p xtask -- simdoc"
    );
    assert_eq!(
        recorded.validation_commands,
        ["cargo test", "cargo run -p xtask -- simdoc --check"]
    );
}

#[test]
fn undeclared_commands_fall_back_to_the_direct_encoder_check() {
    let recorded = regeneration(&tooling_root(), &json!({})).unwrap();

    assert_eq!(
        recorded.regeneration_command,
        "cargo run --locked --manifest-path crates/simdoc/Cargo.toml -- simdoc"
    );
    assert_eq!(
        recorded.validation_commands,
        ["cargo run --locked --manifest-path crates/simdoc/Cargo.toml -- simdoc --check"]
    );

    let declared_launcher = json!({"metadata": {"sim": {"docs-command": "just docs"}}});
    let recorded = regeneration(&tooling_root(), &declared_launcher).unwrap();
    assert_eq!(recorded.validation_commands, ["just docs --check"]);
}

#[test]
fn direct_encoder_command_never_names_a_path_outside_the_repository() {
    let root = temp_root("regeneration-layout");
    let repo = root.join("sim-platform");
    let manifest = root.join("sim-tooling/crates/simdoc/Cargo.toml");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(&manifest, "").unwrap();

    let err = direct_encoder_command(&repo, &manifest).unwrap_err();
    assert!(
        err.contains("declare workspace.metadata.sim.docs-command"),
        "{err}"
    );
    let err = regeneration(&repo, &json!({})).unwrap_err();
    assert!(err.contains("outside this repository"), "{err}");

    let inside = root.join("sim-tooling");
    assert_eq!(
        direct_encoder_command(&inside, &manifest).unwrap(),
        "cargo run --locked --manifest-path crates/simdoc/Cargo.toml -- simdoc"
    );

    let spaced = inside.join("with space/Cargo.toml");
    fs::create_dir_all(spaced.parent().unwrap()).unwrap();
    fs::write(&spaced, "").unwrap();
    assert!(
        direct_encoder_command(&inside, &spaced)
            .unwrap_err()
            .contains("cannot be recorded")
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_command_declarations_are_refused() {
    let repo = tooling_root();
    for (declared, expected) in [
        (json!({"docs-command": 1}), "must be strings"),
        (json!({"docs-command": ""}), "printable ASCII"),
        (json!({"docs-command": " cargo"}), "printable ASCII"),
        (json!({"docs-command": "cargo\ntest"}), "printable ASCII"),
        (
            json!({"docs-command": "cargo t\u{e9}st"}),
            "printable ASCII",
        ),
        (
            json!({"validation-commands": "cargo test"}),
            "must be an array",
        ),
        (json!({"validation-commands": []}), "must not be empty"),
        (json!({"validation-commands": [1]}), "must be strings"),
        (
            json!({"validation-commands": ["cargo test", "cargo test"]}),
            "declared twice",
        ),
    ] {
        let metadata = json!({"metadata": {"sim": declared}});
        let err = regeneration(&repo, &metadata).unwrap_err();
        assert!(err.contains(expected), "{declared}: {err}");
    }
}

#[test]
fn executable_identity_binds_version_source_and_built_lock() {
    let lock = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).unwrap();
    let identity = executable_identity();

    assert_eq!(identity["package"], "simdoc");
    assert_eq!(identity["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(identity["lock_sha256"], content_digest(&lock));
    let source = identity["source_sha256"].as_str().unwrap();
    assert_eq!(source.len(), 64);
    assert!(source.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_ne!(source, content_digest(&lock));
}

#[test]
fn toolchain_identity_is_the_pinned_channel() {
    let pin = fs::read_to_string(tooling_root().join("rust-toolchain.toml")).unwrap();
    let channel = pin
        .lines()
        .find_map(|line| line.trim().strip_prefix("channel = "))
        .unwrap()
        .trim_matches('"');
    let toolchain = toolchain_identity();

    for tool in ["rustc", "cargo"] {
        assert_eq!(toolchain[tool]["release"], channel, "{tool}");
        assert_eq!(
            toolchain[tool]["commit_hash"].as_str().unwrap().len(),
            40,
            "{tool}"
        );
    }
}

#[test]
fn execution_records_identities_and_no_path_for_an_outside_resolver() {
    let recorded = execution_with(None);
    assert_eq!(recorded["operation"], CONTRACT_OPERATION);
    assert_eq!(recorded["executable"], executable_identity());
    assert_eq!(recorded["toolchain"], toolchain_identity());
    assert!(recorded["resolver"].is_null());

    let root = temp_root("regeneration-resolver");
    let meta = root.join("sim-private/.meta-workspace");
    fs::create_dir_all(&meta).unwrap();
    fs::write(meta.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
    fs::write(meta.join("Cargo.lock"), "version = 4\n").unwrap();
    let resolver = crate::resolver_input::validate(&meta.join("Cargo.toml")).unwrap();

    let recorded = execution_with(Some(&resolver));
    assert_eq!(recorded["resolver"]["kind"], "shared-resolver");
    let text = recorded.to_string();
    for leak in [
        root.to_string_lossy().as_ref(),
        "sim-private",
        "meta-workspace",
        "Cargo.toml",
        "../",
        env!("CARGO_MANIFEST_DIR"),
    ] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }
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
