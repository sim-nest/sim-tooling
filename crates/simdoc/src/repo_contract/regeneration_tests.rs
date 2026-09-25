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

// conformance: provenance records the encoder that ran and only commands the
// repository declares or the encoder can prove.

#[test]
fn declared_commands_are_recorded_verbatim() {
    let metadata = json!({"metadata": {"sim": {
        "docs-command": "cargo run -p xtask -- simdoc",
        "validation-commands": ["cargo test", "cargo run -p xtask -- simdoc --check"],
    }}});

    let recorded = regeneration(&crate::tooling_checkout_root(), &metadata).unwrap();

    assert_eq!(recorded.docs_command, "cargo run -p xtask -- simdoc");
    assert_eq!(
        recorded.validation_commands,
        ["cargo test", "cargo run -p xtask -- simdoc --check"]
    );
}

#[test]
fn undeclared_commands_fall_back_to_the_direct_encoder_check() {
    let recorded = regeneration(&crate::tooling_checkout_root(), &json!({})).unwrap();

    assert_eq!(
        recorded.docs_command,
        "cargo run --locked --manifest-path crates/simdoc/Cargo.toml -- simdoc"
    );
    assert_eq!(
        recorded.validation_commands,
        ["cargo run --locked --manifest-path crates/simdoc/Cargo.toml -- simdoc --check"]
    );

    let declared_launcher = json!({"metadata": {"sim": {"docs-command": "just docs"}}});
    let recorded = regeneration(&crate::tooling_checkout_root(), &declared_launcher).unwrap();
    assert_eq!(recorded.validation_commands, ["just docs --check"]);
}

#[test]
fn direct_encoder_command_names_the_encoder_from_the_repository_root() {
    let root = temp_root("regeneration-layout");
    let repo = root.join("sim-platform");
    let manifest = root.join("sim-tooling/crates/simdoc/Cargo.toml");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(&manifest, "").unwrap();

    assert_eq!(
        direct_encoder_command(&repo, &manifest).unwrap(),
        "cargo run --locked --manifest-path ../sim-tooling/crates/simdoc/Cargo.toml -- simdoc"
    );

    let spaced = root.join("with space/Cargo.toml");
    fs::create_dir_all(spaced.parent().unwrap()).unwrap();
    fs::write(&spaced, "").unwrap();
    assert!(
        direct_encoder_command(&repo, &spaced)
            .unwrap_err()
            .contains("cannot be recorded")
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_command_declarations_are_refused() {
    let repo = crate::tooling_checkout_root();
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
fn encoder_identity_binds_version_and_built_lock() {
    let lock = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).unwrap();

    assert_eq!(
        encoder_identity(),
        json!({
            "package": "simdoc",
            "version": env!("CARGO_PKG_VERSION"),
            "lock_sha256": content_digest(&lock),
        })
    );
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
