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

/// The engine digest as the launcher frames it, computed here independently of
/// the launcher: `Cargo.toml`, `Cargo.lock`, `build.rs`, `build_identity.rs`
/// and every file under `src`, sorted by path COMPONENTS (so `src/a/b.rs`
/// sorts before `src/a.rs`, as `Path` ordering does), each as the
/// slash-separated path, a NUL, the u64 little-endian length, and the bytes.
fn independent_source_digest(engine: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    fn walk(root: &std::path::Path, relative: &std::path::Path, out: &mut Vec<Vec<String>>) {
        for entry in fs::read_dir(root.join(relative)).unwrap() {
            let entry = entry.unwrap();
            let path = relative.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, out);
            } else {
                out.push(
                    path.components()
                        .map(|part| part.as_os_str().to_str().unwrap().to_owned())
                        .collect(),
                );
            }
        }
    }
    let mut files: Vec<Vec<String>> = ["Cargo.toml", "Cargo.lock", "build.rs", "build_identity.rs"]
        .iter()
        .map(|name| vec![(*name).to_owned()])
        .collect();
    walk(engine, std::path::Path::new("src"), &mut files);
    files.sort();
    let mut hasher = Sha256::new();
    for parts in files {
        let name = parts.join("/");
        let bytes = fs::read(engine.join(&name)).unwrap();
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The pin committed at `HEAD` is the digest of the engine committed at
/// `HEAD`, recomputed from `git archive HEAD` (a clean export: no working
/// tree, no ignored or untracked file, no editor state) by an implementation
/// independent of the launcher. A stale pin (an engine or lock changed after
/// `simdoc-pin`, or a covered file regenerated) fails here, in CI and in the
/// local gate. Before the commit that carries the change the test judges the
/// previous commit; run it again after committing.
#[test]
fn the_pin_at_head_is_the_digest_of_the_engine_at_head() {
    let head = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .current_dir(tooling_root())
        .output();
    if !head.is_ok_and(|output| output.status.success()) {
        return; // not a git checkout: nothing to export
    }
    let archive = Command::new("git")
        .args([
            "archive",
            "--format=tar",
            "HEAD",
            "Cargo.toml",
            "crates/simdoc",
        ])
        .current_dir(tooling_root())
        .output()
        .unwrap();
    assert!(archive.status.success(), "git archive HEAD");
    let dir = scratch("pin-at-head");
    let mut untar = Command::new("tar")
        .args(["xf", "-", "-C"])
        .arg(&dir)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut untar.stdin.take().unwrap(), &archive.stdout).unwrap();
    assert!(untar.wait().unwrap().success(), "extract the archive");
    let manifest = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    let committed = |key: &str| {
        manifest
            .lines()
            .find_map(|line| {
                line.strip_prefix(key)?
                    .trim_start()
                    .strip_prefix('=')
                    .map(|value| value.trim().trim_matches('"').to_owned())
            })
            .unwrap_or_else(|| panic!("no {key} in the committed manifest"))
    };
    let engine = dir.join("crates/simdoc");
    assert_eq!(
        independent_source_digest(&engine),
        committed("source_sha256"),
        "the committed engine source pin is stale at HEAD; run `cargo run -p xtask -- simdoc-pin`, then commit"
    );
    let lock = fs::read(engine.join("Cargo.lock")).unwrap();
    let lock_digest: String = {
        use sha2::{Digest, Sha256};
        Sha256::digest(&lock)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    };
    assert_eq!(
        lock_digest,
        committed("lock_sha256"),
        "lock pin is stale at HEAD"
    );
    fs::remove_dir_all(dir).unwrap();
}
