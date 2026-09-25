// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
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

fn sample_pin() -> EncoderPin {
    EncoderPin {
        source_sha256: "a".repeat(64),
        lock_sha256: "b".repeat(64),
        toolchain: "1.96.0".to_owned(),
        rustc_commit: "c".repeat(40),
        cargo_commit: "d".repeat(40),
        toolchain_sha256: BTreeMap::from([
            ("aarch64-apple-darwin".to_owned(), "e".repeat(64)),
            ("x86_64-unknown-linux-gnu".to_owned(), "f".repeat(64)),
        ]),
    }
}

#[test]
fn the_committed_identity_round_trips_through_the_manifest() {
    let pin = sample_pin();
    let manifest = "[package]\nname = \"xtask\"\n\n[workspace.metadata.sim.encoder]\n\
                    source_sha256 = \"old\"\nlock_sha256 = \"old\"\ntoolchain = \"0\"\n\n\
                    [workspace.metadata.sim.encoder.toolchain_sha256]\nold = \"old\"\n\n\
                    [dependencies]\nserde_json = \"1\"\n";
    let written = pin.write_into(manifest);

    assert_eq!(EncoderPin::from_manifest(&written).unwrap(), pin);
    assert!(written.contains("[dependencies]\nserde_json = \"1\""));
    assert_eq!(written.matches(PIN_TABLE).count(), 1);
    assert_eq!(written.matches(DIGEST_TABLE).count(), 1);
    assert!(!written.contains("old"));
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
    // A committed digest that is not the running toolchain's is stale.
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let digest = repinned.toolchain_sha256.values().next().unwrap();
    fs::write(
        root.join("Cargo.toml"),
        manifest.replace(digest.as_str(), &"0".repeat(64)),
    )
    .unwrap();
    assert!(
        run(&root, &["--check".to_owned()])
            .unwrap_err()
            .contains("stale")
    );
    fs::write(root.join("Cargo.toml"), manifest).unwrap();
    run(&root, &["--check".to_owned()]).unwrap();
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.97.1\"\n",
    )
    .unwrap();
    assert!(run(&root, &["--check".to_owned()]).is_err());
    fs::remove_dir_all(root).unwrap();
}

/// The toolchain that runs these tests, described the way a pin commits it.
fn running_pin() -> (EncoderPin, PathBuf) {
    let cargo = PathBuf::from(env::var_os("CARGO").expect("cargo runs the tests"));
    let root = cargo.parent().unwrap().parent().unwrap().to_path_buf();
    let (release, cargo_commit, host) = reported(&root.join("bin/cargo")).unwrap();
    let (_, rustc_commit, _) = reported(&root.join("bin/rustc")).unwrap();
    let mut pin = sample_pin();
    pin.toolchain = release;
    pin.rustc_commit = rustc_commit;
    pin.cargo_commit = cargo_commit;
    pin.toolchain_sha256 = BTreeMap::from([(host, toolchain_digest(&root).unwrap())]);
    (pin, root)
}

#[test]
fn a_toolchain_is_accepted_by_content_and_never_through_rustup() {
    let (pin, root) = running_pin();
    let toolchain = locate_toolchain(&pin, std::slice::from_ref(&root)).unwrap();
    assert_eq!(toolchain.cargo, root.join("bin/cargo"));
    assert_eq!(toolchain.rustdoc, root.join("bin/rustdoc"));
    assert!(pin.toolchain_sha256.contains_key(&toolchain.host));

    let mut wrong_release = pin.clone();
    wrong_release.toolchain = "1.97.1".to_owned();
    let err = locate_toolchain(&wrong_release, std::slice::from_ref(&root)).unwrap_err();
    assert!(err.contains("reports"), "{err}");

    let mut wrong_commit = pin.clone();
    wrong_commit.rustc_commit = "0".repeat(40);
    assert!(locate_toolchain(&wrong_commit, std::slice::from_ref(&root)).is_err());

    let mut wrong_digest = pin.clone();
    for digest in wrong_digest.toolchain_sha256.values_mut() {
        *digest = "0".repeat(64);
    }
    let err = locate_toolchain(&wrong_digest, std::slice::from_ref(&root)).unwrap_err();
    assert!(err.contains("is not committed for any host"), "{err}");
    assert!(
        locate_toolchain(&pin, &[])
            .unwrap_err()
            .contains("none found")
    );
}

#[test]
fn a_counterfeit_toolchain_that_reports_the_pinned_identity_is_refused() {
    let (pin, _) = running_pin();
    let dir = temp_root("pin-counterfeit");
    fs::create_dir_all(dir.join("bin")).unwrap();
    fs::create_dir_all(dir.join("lib")).unwrap();
    let host = pin.toolchain_sha256.keys().next().unwrap();
    // Each tool reports exactly the committed release, commit, and host.
    for (tool, commit) in [
        ("cargo", &pin.cargo_commit),
        ("rustc", &pin.rustc_commit),
        ("rustdoc", &pin.rustc_commit),
    ] {
        let report = format!(
            "echo 'release: {}'; echo 'commit-hash: {commit}'; echo 'host: {host}'",
            pin.toolchain
        );
        script(&dir.join("bin").join(tool), &report);
    }
    fs::write(dir.join("lib/librustc_driver-x.so"), "").unwrap();
    let err = locate_toolchain(&pin, std::slice::from_ref(&dir)).unwrap_err();
    assert!(err.contains("is not committed for any host"), "{err}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn every_launched_command_runs_under_a_cleared_environment() {
    let toolchain = Toolchain {
        cargo: PathBuf::from("/toolchains/1.96.0/bin/cargo"),
        rustc: PathBuf::from("/toolchains/1.96.0/bin/rustc"),
        rustdoc: PathBuf::from("/toolchains/1.96.0/bin/rustdoc"),
        channel: "1.96.0".to_owned(),
        host: "x86_64-unknown-linux-gnu".to_owned(),
    };
    let command = toolchain.command(&toolchain.cargo);
    assert_eq!(command.get_program(), "/toolchains/1.96.0/bin/cargo");
    let envs = command
        .get_envs()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.map(ToOwned::to_owned),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        envs["PATH"],
        Some(OsString::from("/toolchains/1.96.0/bin:/usr/bin:/bin"))
    );
    assert_eq!(
        envs["RUSTC"],
        Some(OsString::from("/toolchains/1.96.0/bin/rustc"))
    );
    assert_eq!(
        envs["RUSTDOC"],
        Some(OsString::from("/toolchains/1.96.0/bin/rustdoc"))
    );
    for wrapper in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"] {
        assert_eq!(envs[wrapper], Some(OsString::new()));
    }
    for name in envs.keys() {
        assert!(
            PASSED_ENVIRONMENT.contains(&name.as_str())
                || [
                    "PATH",
                    "RUSTC",
                    "RUSTDOC",
                    "RUSTC_WRAPPER",
                    "RUSTC_WORKSPACE_WRAPPER"
                ]
                .contains(&name.as_str()),
            "{name} leaks into a launched command"
        );
    }
}

#[test]
fn the_built_identity_must_match_every_committed_field() {
    let pin = sample_pin();
    let toolchain = Toolchain {
        cargo: PathBuf::from("/t/bin/cargo"),
        rustc: PathBuf::from("/t/bin/rustc"),
        rustdoc: PathBuf::from("/t/bin/rustdoc"),
        channel: "1.96.0".to_owned(),
        host: "x86_64-unknown-linux-gnu".to_owned(),
    };
    let reported = |field: &str, value: &str| {
        let mut fields = BTreeMap::from([
            ("source_sha256", "a".repeat(64)),
            ("lock_sha256", "b".repeat(64)),
            ("rustc_release", "1.96.0".to_owned()),
            ("cargo_release", "1.96.0".to_owned()),
            ("rustc_commit", "c".repeat(40)),
            ("cargo_commit", "d".repeat(40)),
            ("toolchain_sha256", "f".repeat(64)),
        ]);
        fields.insert(
            fields
                .keys()
                .find(|key| **key == field)
                .copied()
                .unwrap_or("package"),
            value.to_owned(),
        );
        fields
            .iter()
            .map(|(key, value)| format!("{key}={value}\n"))
            .collect::<String>()
    };
    verify_identity(&reported("package", "simdoc"), &pin, &toolchain).unwrap();
    for field in [
        "source_sha256",
        "lock_sha256",
        "rustc_release",
        "cargo_release",
        "rustc_commit",
        "cargo_commit",
        "toolchain_sha256",
    ] {
        let err = verify_identity(&reported(field, "wrong"), &pin, &toolchain).unwrap_err();
        assert!(err.contains(field), "{field}: {err}");
    }
    assert!(verify_identity("", &pin, &toolchain).is_err());
}
