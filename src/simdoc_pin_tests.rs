// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    env,
    os::unix::fs::{PermissionsExt, symlink},
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

// conformance: the engine runs only when its source, lock, toolchain, and
// built identity are exactly the committed ones.

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}

/// A tooling checkout holding a minimal engine tree.
fn tooling(label: &str) -> PathBuf {
    let root = temp_root(label);
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.96.0\"\n",
    )
    .unwrap();
    let simdoc = root.join("crates/simdoc");
    fs::create_dir_all(simdoc.join("src/nested")).unwrap();
    for file in ["Cargo.toml", "Cargo.lock", "build.rs", "build_identity.rs"] {
        fs::write(simdoc.join(file), file).unwrap();
    }
    fs::write(simdoc.join("src/lib.rs"), "lib").unwrap();
    fs::write(simdoc.join("src/nested/mod.rs"), "nested").unwrap();
    root
}

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn the_committed_identity_round_trips_through_the_manifest() {
    let pin = EncoderPin {
        source_sha256: "a".repeat(64),
        lock_sha256: "b".repeat(64),
        toolchain: "1.96.0".to_owned(),
    };
    let manifest = "[package]\nname = \"xtask\"\n\n[workspace.metadata.sim.encoder]\n\
                    source_sha256 = \"old\"\nlock_sha256 = \"old\"\ntoolchain = \"0\"\n\n\
                    [dependencies]\nserde_json = \"1\"\n";
    let written = pin.write_into(manifest);

    assert_eq!(EncoderPin::from_manifest(&written).unwrap(), pin);
    assert!(written.contains("[dependencies]\nserde_json = \"1\""));
    assert_eq!(written.matches(PIN_TABLE).count(), 1);
    assert!(
        EncoderPin::from_manifest("[package]\nname = \"x\"\n")
            .unwrap_err()
            .contains("simdoc-pin")
    );
}

#[test]
fn the_source_digest_binds_every_engine_file_and_refuses_symlinks() {
    let root = tooling("pin-digest");
    let simdoc = root.join("crates/simdoc");
    let first = source_digest(&simdoc).unwrap();
    assert_eq!(first, source_digest(&simdoc).unwrap());

    fs::write(simdoc.join("src/nested/mod.rs"), "changed").unwrap();
    let second = source_digest(&simdoc).unwrap();
    assert_ne!(first, second);
    fs::write(simdoc.join("build_identity.rs"), "changed").unwrap();
    assert_ne!(second, source_digest(&simdoc).unwrap());

    symlink(
        root.join("rust-toolchain.toml"),
        simdoc.join("src/extra.rs"),
    )
    .unwrap();
    assert!(
        source_digest(&simdoc)
            .unwrap_err()
            .contains("not an ordinary file")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_modified_engine_is_refused_until_it_is_pinned_again() {
    let root = tooling("pin-verify");
    let pin = EncoderPin::measure(&root, &root.join("crates/simdoc")).unwrap();
    verify_source(&root, &pin).unwrap();

    fs::write(root.join("crates/simdoc/src/lib.rs"), "modified").unwrap();
    let err = verify_source(&root, &pin).unwrap_err();
    assert!(err.contains("not the committed one"), "{err}");

    fs::write(root.join("Cargo.toml"), "[package]\nname = \"xtask\"\n").unwrap();
    run(&root, &[]).unwrap();
    run(&root, &["--check".to_owned()]).unwrap();
    let repinned =
        EncoderPin::from_manifest(&fs::read_to_string(root.join("Cargo.toml")).unwrap()).unwrap();
    verify_source(&root, &repinned).unwrap();
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.97.1\"\n",
    )
    .unwrap();
    assert!(
        run(&root, &["--check".to_owned()])
            .unwrap_err()
            .contains("stale")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn toolchain_binaries_come_from_rustup_and_must_report_the_pin() {
    let root = temp_root("pin-toolchain");
    let good = root.join("good");
    let bad = root.join("bad");
    script(&good, "echo 'rustc 1.96.0'; echo 'release: 1.96.0'");
    script(&bad, "echo 'rustc 1.97.1'; echo 'release: 1.97.1'");
    let rustup = root.join("rustup");

    script(&rustup, &format!("echo {}", good.display()));
    let toolchain = resolve_toolchain(rustup.as_os_str(), "1.96.0").unwrap();
    assert_eq!(toolchain.cargo, good);
    assert_eq!(toolchain.rustc, good);

    script(&rustup, &format!("echo {}", bad.display()));
    let err = resolve_toolchain(rustup.as_os_str(), "1.96.0").unwrap_err();
    assert!(
        err.contains("reports release 1.97.1, not the pinned 1.96.0"),
        "{err}"
    );

    script(&rustup, "echo relative/cargo");
    let err = resolve_toolchain(rustup.as_os_str(), "1.96.0").unwrap_err();
    assert!(err.contains("relative path"), "{err}");

    script(&rustup, "echo 'toolchain not installed' >&2; exit 1");
    let err = resolve_toolchain(rustup.as_os_str(), "1.96.0").unwrap_err();
    assert!(err.contains("rustup cannot resolve cargo"), "{err}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_built_identity_must_match_every_committed_field() {
    let pin = EncoderPin {
        source_sha256: "a".repeat(64),
        lock_sha256: "b".repeat(64),
        toolchain: "1.96.0".to_owned(),
    };
    let reported = |source: &str, lock: &str, rustc: &str, cargo: &str| {
        format!(
            "package=simdoc\nsource_sha256={source}\nlock_sha256={lock}\n\
             rustc_release={rustc}\ncargo_release={cargo}\n"
        )
    };
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    verify_identity(&reported(&a, &b, "1.96.0", "1.96.0"), &pin).unwrap();
    for (text, field) in [
        (reported(&b, &b, "1.96.0", "1.96.0"), "source_sha256"),
        (reported(&a, &a, "1.96.0", "1.96.0"), "lock_sha256"),
        (reported(&a, &b, "1.97.1", "1.96.0"), "rustc_release"),
        (reported(&a, &b, "1.96.0", "1.97.1"), "cargo_release"),
        (String::new(), "source_sha256"),
    ] {
        let err = verify_identity(&text, &pin).unwrap_err();
        assert!(err.contains(field), "{field}: {err}");
    }
}
