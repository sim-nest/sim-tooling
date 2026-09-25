// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! End-to-end falsifiers for execution identity: the contract engine runs
//! only under the committed toolchain, found by content, and nothing on the
//! caller's `PATH` (a fake `rustup`, `cargo`, `rustc`, `rustdoc`, or `git`)
//! is ever executed, before or after the launch.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn tooling_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

/// A directory of counterfeit programs that record being run in `marker`.
fn counterfeits(dir: &Path, marker: &Path, names: &[&str]) {
    fs::create_dir_all(dir).unwrap();
    for name in names {
        let path = dir.join(name);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\necho {name} >> {}\necho 'release: 1.96.0'\n\
                 echo 'commit-hash: 0000000000000000000000000000000000000000'\nexit 1\n",
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn ran(marker: &Path) -> String {
    fs::read_to_string(marker).unwrap_or_default()
}

/// A committed one-package repository named `sim-fixture`.
fn fixture(label: &str) -> PathBuf {
    let root = scratch(label).join("sim-fixture");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
         description = \"Fixture application.\"\n\n[workspace]\n\n\
         [workspace.metadata.sim]\ndocs-command = \"cargo run -p xtask -- simdoc\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "//! Fixture application.\n").unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec![
            "remote",
            "add",
            "origin",
            "https://github.com/sim-nest/sim-fixture",
        ],
        vec!["add", "-A"],
        vec![
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--no-verify",
            "-mfixture",
        ],
    ] {
        let status = Command::new("git")
            .args(&args)
            .current_dir(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }
    root
}

/// Runs the built xtask on `repo` with a chosen environment.
fn emit(repo: &Path, out: &Path, environment: &[(&str, &Path)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    command
        .env_clear()
        .current_dir(repo)
        .args([
            "repo-contract",
            "--repo",
            ".",
            "--emit",
            "repo-contract.json",
            "--out-dir",
        ])
        .arg(out);
    for (name, value) in environment {
        command.env(name, value);
    }
    command.output().unwrap()
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME"))
}

/// The one `PATH` a hostile caller can choose: counterfeits first.
fn hostile_path(dir: &Path) -> PathBuf {
    PathBuf::from(format!("{}:/usr/bin:/bin", dir.display()))
}

#[test]
fn a_fake_rustup_and_fake_path_tools_are_never_run_before_or_after_launch() {
    let repo = fixture("isolation-path");
    let out = scratch("isolation-path-out");
    let dir = scratch("isolation-path-fakes");
    let marker = dir.join("ran");
    counterfeits(
        &dir.join("bin"),
        &marker,
        &["rustup", "cargo", "rustc", "rustdoc", "git"],
    );
    let real_cargo = PathBuf::from(env!("CARGO"));

    // A hostile compiler-flag variable must not reach the engine's build.
    let hostile_flags = PathBuf::from("-C link-arg=--no-such-linker-flag");
    let output = emit(
        &repo,
        &out,
        &[
            ("PATH", &hostile_path(&dir.join("bin"))),
            ("HOME", &home()),
            ("CARGO", &real_cargo),
            ("RUSTFLAGS", &hostile_flags),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(ran(&marker), "", "a counterfeit program was executed");
    assert!(out.join("repo-contract.json").is_file());

    // The built engine trusts only the toolchain it was built with, and takes
    // it from the cargo that started it, never from PATH.
    let engine = engine_binary();
    let engine_args = |command: &mut Command| {
        command
            .env_clear()
            .env("HOME", home())
            .env("PATH", hostile_path(&dir.join("bin")))
            .current_dir(&repo)
            .args([
                "repo-contract",
                "--repo",
                ".",
                "--emit",
                "repo-contract.json",
                "--out-dir",
            ])
            .arg(&out);
    };
    let mut command = Command::new(&engine);
    engine_args(&mut command);
    command.env("CARGO", &real_cargo);
    assert!(command.output().unwrap().status.success());

    // Nothing the caller exports steers the programs the engine launches: a
    // hostile Git or cargo environment is cleared, not inherited.
    let mut command = Command::new(&engine);
    engine_args(&mut command);
    command
        .env("CARGO", &real_cargo)
        .env("GIT_DIR", "/nonexistent-git-dir")
        .env("GIT_WORK_TREE", "/nonexistent-work-tree")
        .env("CARGO_BUILD_TARGET", "no-such-target")
        .env("CARGO_BUILD_RUSTC", dir.join("bin/rustc"));
    let hostile = command.output().unwrap();
    assert!(
        hostile.status.success(),
        "{}",
        String::from_utf8_lossy(&hostile.stderr)
    );
    assert_eq!(ran(&marker), "", "a counterfeit program was executed");

    let mut command = Command::new(&engine);
    engine_args(&mut command);
    let refused = command.output().unwrap();
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("CARGO is not set"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );

    let fake_toolchain = dir.join("toolchain");
    counterfeits(
        &fake_toolchain.join("bin"),
        &marker,
        &["cargo", "rustc", "rustdoc"],
    );
    fs::create_dir_all(fake_toolchain.join("lib")).unwrap();
    fs::write(fake_toolchain.join("lib/librustc_driver-x.so"), "").unwrap();
    let mut command = Command::new(&engine);
    engine_args(&mut command);
    command.env("CARGO", fake_toolchain.join("bin/cargo"));
    let refused = command.output().unwrap();
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("is not the one this simdoc was built with"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert_eq!(ran(&marker), "", "a counterfeit program was executed");
}

/// The engine executable the launcher just built.
fn engine_binary() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| tooling_root().join("crates/simdoc/target"));
    target.join("debug/simdoc")
}

#[test]
fn a_counterfeit_toolchain_is_refused_by_content_and_no_rustup_is_consulted() {
    let repo = fixture("isolation-toolchain");
    let out = scratch("isolation-toolchain-out");
    let dir = scratch("isolation-toolchain-fakes");
    let marker = dir.join("ran");
    counterfeits(&dir.join("bin"), &marker, &["rustup", "cargo", "rustc"]);
    // A rustup home whose pinned toolchain is counterfeit, reporting the
    // pinned release, and a cargo variable that names it.
    let toolchains = dir.join("rustup/toolchains/1.96.0-x86_64-unknown-linux-gnu");
    counterfeits(
        &toolchains.join("bin"),
        &marker,
        &["cargo", "rustc", "rustdoc"],
    );
    fs::create_dir_all(toolchains.join("lib")).unwrap();
    fs::write(toolchains.join("lib/librustc_driver-x.so"), "").unwrap();

    let output = emit(
        &repo,
        &out,
        &[
            ("PATH", &hostile_path(&dir.join("bin"))),
            ("HOME", &dir),
            ("RUSTUP_HOME", &dir.join("rustup")),
            ("CARGO", &toolchains.join("bin/cargo")),
        ],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no toolchain matches the committed identity"),
        "{stderr}"
    );
    assert!(!out.join("repo-contract.json").exists());
    assert_eq!(ran(&marker), "", "a counterfeit program was executed");
}
