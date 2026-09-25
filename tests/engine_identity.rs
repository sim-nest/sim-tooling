// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! End-to-end identity falsifiers for the contract engine.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn tooling_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn pinned_channel() -> String {
    fs::read_to_string(tooling_root().join("rust-toolchain.toml"))
        .unwrap()
        .lines()
        .find_map(|line| {
            let value = line
                .trim()
                .strip_prefix("channel")?
                .trim()
                .strip_prefix('=')?;
            Some(value.trim().trim_matches('"').to_owned())
        })
        .unwrap()
}

/// The directory of the toolchain running these tests (no `rustup`, no PATH).
fn toolchain_bin() -> PathBuf {
    PathBuf::from(env!("CARGO")).parent().unwrap().to_path_buf()
}

fn scratch(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The engine's build refuses a compiler that is not the pinned release:
/// removing the check from `build.rs` makes this build succeed.
#[test]
fn a_substituted_compiler_cannot_build_the_engine() {
    let dir = scratch("engine-substituted-rustc");
    let wrapper = dir.join("rustc");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nreal={}\nif [ \"$1\" = --version ] && [ \"$2\" = --verbose ]; then\n  \
             \"$real\" \"$@\" | sed 's/^release: .*/release: 0.0.1-substituted/'\n  exit 0\nfi\n\
             exec \"$real\" \"$@\"\n",
            toolchain_bin().join("rustc").display()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();

    let output = Command::new(toolchain_bin().join("cargo"))
        .current_dir(tooling_root())
        .env("RUSTC", &wrapper)
        .env_remove("RUSTC_WRAPPER")
        .args(["build", "--quiet", "--locked", "--manifest-path"])
        .arg(tooling_root().join("crates/simdoc/Cargo.toml"))
        .arg("--target-dir")
        .arg(dir.join("target"))
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "a substituted compiler built the engine"
    );
    assert!(
        stderr.contains(&format!(
            "simdoc must be built with the pinned toolchain {}; rustc is 0.0.1-substituted",
            pinned_channel()
        )),
        "{stderr}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

/// The committed identity is the one on disk. (Every xtask contract route
/// then accepts the engine only after the built executable reports exactly
/// it; the cross-route tests exercise that path.)
#[test]
fn the_committed_engine_identity_is_current() {
    xtask::run(vec![
        "xtask".to_owned(),
        "simdoc-pin".to_owned(),
        "--check".to_owned(),
    ])
    .expect("the committed engine identity is current");
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "target" {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Builds a copy of the engine whose root manifest pins `digest_line`
/// (rewritten from the committed one), and returns cargo's output.
fn build_with_pin(label: &str, rewrite: impl Fn(&str) -> String) -> std::process::Output {
    let dir = scratch(label);
    let root = dir.join("tooling");
    fs::create_dir_all(&root).unwrap();
    for file in ["Cargo.toml", "rust-toolchain.toml"] {
        fs::copy(tooling_root().join(file), root.join(file)).unwrap();
    }
    copy_tree(
        &tooling_root().join("crates/simdoc"),
        &root.join("crates/simdoc"),
    );
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    fs::write(root.join("Cargo.toml"), rewrite(&manifest)).unwrap();
    let output = Command::new(toolchain_bin().join("cargo"))
        .current_dir(&root)
        .env("RUSTC", toolchain_bin().join("rustc"))
        .env_remove("RUSTC_WRAPPER")
        .args(["build", "--quiet", "--locked", "--manifest-path"])
        .arg(root.join("crates/simdoc/Cargo.toml"))
        .arg("--target-dir")
        .arg(tooling_root().join("crates/simdoc/target"))
        .output()
        .unwrap();
    fs::remove_dir_all(&dir).unwrap();
    output
}

/// The engine's build refuses a toolchain whose content is not the pinned
/// one, even though it reports the pinned release: removing the digest
/// comparison from `build.rs` makes these builds succeed.
#[test]
fn the_engine_build_refuses_a_toolchain_that_is_not_the_pinned_content() {
    let digest_line = |manifest: &str| {
        manifest
            .lines()
            .find(|line| line.starts_with("x86_64-unknown-linux-gnu = "))
            .map(str::to_owned)
    };
    if digest_line(&fs::read_to_string(tooling_root().join("Cargo.toml")).unwrap()).is_none() {
        return; // the committed pin is for another host
    }
    let wrong = build_with_pin("engine-wrong-digest", |manifest| {
        let line = digest_line(manifest).unwrap();
        manifest.replace(
            &line,
            &format!("x86_64-unknown-linux-gnu = \"{}\"", "0".repeat(64)),
        )
    });
    let stderr = String::from_utf8_lossy(&wrong.stderr);
    assert!(
        !wrong.status.success(),
        "a toolchain with the wrong digest built the engine"
    );
    assert!(stderr.contains("is not the pinned one"), "{stderr}");

    let missing = build_with_pin("engine-missing-digest", |manifest| {
        let line = digest_line(manifest).unwrap();
        manifest.replace(&line, "")
    });
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(
        !missing.status.success(),
        "a toolchain with no pinned digest built the engine"
    );
    assert!(stderr.contains("pins no toolchain digest"), "{stderr}");
}
