// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Build-time identity of the simdoc encoder.
//!
//! Embeds a SHA-256 content digest of the encoder's own manifest, lock, build
//! inputs, and `src` tree, the release and commit of the rustc and cargo that
//! built it, and a content digest of that toolchain's executables and
//! libraries. The build refuses any toolchain other than the one
//! sim-tooling's root manifest pins for this host (release, and content
//! digest), so a recorded identity can only come from the pinned compiler,
//! and the running encoder later requires the programs it launches to be
//! exactly this toolchain.

use std::{env, fs, path::PathBuf, process::Command};

mod build_identity;

use build_identity::{
    host_triple, pinned_channel, pinned_toolchain_digest, require_pinned, source_digest,
    tool_identity, toolchain_digest,
};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    for input in [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "build_identity.rs",
        "src",
        "../../Cargo.toml",
        "../../rust-toolchain.toml",
    ] {
        println!("cargo:rerun-if-changed={input}");
    }
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-env-changed=CARGO");

    let toolchain_path = manifest_dir.join("../../rust-toolchain.toml");
    let toolchain = fs::read_to_string(&toolchain_path).unwrap_or_else(|err| {
        panic!(
            "simdoc builds only inside its sim-tooling checkout ({}): {err}",
            toolchain_path.display()
        )
    });
    let pinned = pinned_channel(&toolchain)
        .unwrap_or_else(|| panic!("{} pins no channel", toolchain_path.display()));

    // The toolchain is the one directory holding the running cargo: its
    // `rustc` and `rustdoc` are the programs the encoder will launch.
    let cargo_path = PathBuf::from(env::var_os("CARGO").expect("cargo sets CARGO"));
    assert!(
        cargo_path.is_absolute(),
        "CARGO must be an absolute path, got {}",
        cargo_path.display()
    );
    let bin = cargo_path.parent().expect("cargo has a directory");
    let root = bin
        .parent()
        .expect("cargo sits in a toolchain bin directory");
    let sibling_rustc = bin.join("rustc");

    let rustc = identity(&env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned()));
    let sibling = identity(&sibling_rustc.to_string_lossy());
    let cargo = identity(&cargo_path.to_string_lossy());
    for (tool, (release, _, _)) in [("rustc", &rustc), ("cargo", &cargo)] {
        if let Err(err) = require_pinned(tool, release, &pinned) {
            panic!("{err}");
        }
    }
    assert!(
        sibling.0 == rustc.0 && sibling.1 == rustc.1 && sibling.2 == rustc.2,
        "the rustc building simdoc is not the rustc beside the running cargo"
    );

    let digest = toolchain_digest(root)
        .unwrap_or_else(|err| panic!("digest the toolchain at {}: {err}", root.display()));
    let root_manifest = fs::read_to_string(manifest_dir.join("../../Cargo.toml"))
        .unwrap_or_else(|err| panic!("read the sim-tooling root Cargo.toml: {err}"));
    let host = &sibling.2;
    match pinned_toolchain_digest(&root_manifest, host) {
        Some(expected) if expected == digest => {}
        Some(expected) => panic!(
            "the toolchain for {host} is not the pinned one: its digest is {digest}, the \
             root manifest pins {expected}"
        ),
        None => panic!(
            "the root manifest pins no toolchain digest for {host} (this toolchain's digest is \
             {digest}); review it and run `cargo run -p xtask -- simdoc-pin`"
        ),
    }

    let source = source_digest(&manifest_dir).unwrap_or_else(|err| panic!("{err}"));
    println!("cargo:rustc-env=SIMDOC_SOURCE_SHA256={source}");
    println!("cargo:rustc-env=SIMDOC_TOOLCHAIN_SHA256={digest}");
    println!("cargo:rustc-env=SIMDOC_RUSTC_RELEASE={}", rustc.0);
    println!("cargo:rustc-env=SIMDOC_RUSTC_COMMIT={}", rustc.1);
    println!("cargo:rustc-env=SIMDOC_CARGO_RELEASE={}", cargo.0);
    println!("cargo:rustc-env=SIMDOC_CARGO_COMMIT={}", cargo.1);
}

/// `(release, commit-hash, host)` of a tool.
fn identity(tool: &str) -> (String, String, String) {
    let output = Command::new(tool)
        .args(["--version", "--verbose"])
        .output()
        .unwrap_or_else(|err| panic!("run {tool} --version --verbose: {err}"));
    assert!(output.status.success(), "{tool} --version --verbose failed");
    let text = String::from_utf8(output.stdout).expect("tool version is UTF-8");
    let (release, commit) = tool_identity(&text)
        .unwrap_or_else(|| panic!("{tool} --version --verbose has no release or commit-hash"));
    let host =
        host_triple(&text).unwrap_or_else(|| panic!("{tool} --version --verbose has no host"));
    (release, commit, host)
}
