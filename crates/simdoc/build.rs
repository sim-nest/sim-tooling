// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Build-time identity of the simdoc encoder.
//!
//! Embeds a SHA-256 content digest of the encoder's own manifest, lock, build
//! inputs, and `src` tree, and the release and commit of the rustc and cargo
//! that built it. The build refuses any toolchain other than the channel
//! pinned in sim-tooling's `rust-toolchain.toml`, so a recorded identity can
//! only come from the pinned compiler.

use std::{env, fs, path::PathBuf, process::Command};

mod build_identity;

use build_identity::{pinned_channel, require_pinned, source_digest, tool_identity};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    for input in [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "build_identity.rs",
        "src",
        "../../rust-toolchain.toml",
    ] {
        println!("cargo:rerun-if-changed={input}");
    }
    println!("cargo:rerun-if-env-changed=RUSTC");

    let toolchain_path = manifest_dir.join("../../rust-toolchain.toml");
    let toolchain = fs::read_to_string(&toolchain_path).unwrap_or_else(|err| {
        panic!(
            "simdoc builds only inside its sim-tooling checkout ({}): {err}",
            toolchain_path.display()
        )
    });
    let pinned = pinned_channel(&toolchain)
        .unwrap_or_else(|| panic!("{} pins no channel", toolchain_path.display()));
    let rustc = identity(&env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned()));
    let cargo = identity(&env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()));
    for (tool, (release, _)) in [("rustc", &rustc), ("cargo", &cargo)] {
        if let Err(err) = require_pinned(tool, release, &pinned) {
            panic!("{err}");
        }
    }

    let digest = source_digest(&manifest_dir).unwrap_or_else(|err| panic!("{err}"));
    println!("cargo:rustc-env=SIMDOC_SOURCE_SHA256={digest}");
    println!("cargo:rustc-env=SIMDOC_RUSTC_RELEASE={}", rustc.0);
    println!("cargo:rustc-env=SIMDOC_RUSTC_COMMIT={}", rustc.1);
    println!("cargo:rustc-env=SIMDOC_CARGO_RELEASE={}", cargo.0);
    println!("cargo:rustc-env=SIMDOC_CARGO_COMMIT={}", cargo.1);
}

fn identity(tool: &str) -> (String, String) {
    let output = Command::new(tool)
        .args(["--version", "--verbose"])
        .output()
        .unwrap_or_else(|err| panic!("run {tool} --version --verbose: {err}"));
    assert!(output.status.success(), "{tool} --version --verbose failed");
    let text = String::from_utf8(output.stdout).expect("tool version is UTF-8");
    tool_identity(&text)
        .unwrap_or_else(|| panic!("{tool} --version --verbose has no release or commit-hash"))
}
