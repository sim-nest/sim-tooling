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

fn rustup_which(tool: &str) -> PathBuf {
    let output = Command::new("rustup")
        .args(["which", "--toolchain", &pinned_channel(), tool])
        .output()
        .unwrap();
    assert!(output.status.success());
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
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
            rustup_which("rustc").display()
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();

    let output = Command::new(rustup_which("cargo"))
        .current_dir(tooling_root())
        .env("RUSTUP_TOOLCHAIN", pinned_channel())
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
