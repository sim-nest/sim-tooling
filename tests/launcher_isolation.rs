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

/// The engine executable, built once per test process into a private target
/// directory by the toolchain running these tests. (The launcher itself builds
/// into its own private directory, which it removes with its process.)
fn engine_binary() -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| {
            let target = scratch("isolation-engine-target");
            let status = Command::new(env!("CARGO"))
                .current_dir(tooling_root())
                .args(["build", "--quiet", "--locked", "--manifest-path"])
                .arg(tooling_root().join("crates/simdoc/Cargo.toml"))
                .arg("--target-dir")
                .arg(&target)
                .status()
                .unwrap();
            assert!(status.success(), "build the engine");
            target.join("debug/simdoc")
        })
        .clone()
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

/// Cargo configuration can replace the engine's dependencies (`paths`, source
/// replacement) while the engine still reports its expected identity. The
/// caller's Cargo home and target directory never reach a cargo run, and a
/// configuration in an ancestor of the repository refuses the engine's run.
#[test]
fn cargo_configuration_never_reaches_the_engine() {
    let real_cargo = PathBuf::from(env!("CARGO"));
    let repo = fixture("isolation-config");
    let out = scratch("isolation-config-out");

    // The caller's Cargo home never reaches the build: a hostile configuration
    // there (which would break or redirect any cargo that read it) has no
    // effect, and neither does a caller-chosen target directory.
    let cargo_home = scratch("isolation-config-home");
    fs::write(
        cargo_home.join("config.toml"),
        "paths = [\"/nonexistent-override\"]\n",
    )
    .unwrap();
    // The crate cache is the caller's; only its checksummed archives are used.
    std::os::unix::fs::symlink(home().join(".cargo/registry"), cargo_home.join("registry"))
        .unwrap();
    let target = scratch("isolation-config-target");
    let real_home = home();
    let output = emit(
        &repo,
        &out,
        &[
            ("HOME", &real_home),
            ("CARGO", &real_cargo),
            ("CARGO_HOME", &cargo_home),
            ("CARGO_TARGET_DIR", &target),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_dir(&target).unwrap().count(),
        0,
        "the caller's target was used"
    );

    // The engine: a `.cargo/config.toml` above the repository refuses it.
    let ok = emit(&repo, &out, &[("HOME", &home()), ("CARGO", &real_cargo)]);
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let parent = repo.parent().unwrap();
    fs::create_dir_all(parent.join(".cargo")).unwrap();
    fs::write(
        parent.join(".cargo/config.toml"),
        "[build]\nrustflags = [\"--cfg=evil\"]\n[env]\nEVIL = \"1\"\n",
    )
    .unwrap();
    let engine = engine_binary();
    let mut command = Command::new(&engine);
    command
        .env_clear()
        .env("HOME", home())
        .env("CARGO", &real_cargo)
        .current_dir(&repo)
        .args([
            "repo-contract",
            "--repo",
            ".",
            "--emit",
            "repo-contract.json",
            "--out-dir",
        ])
        .arg(scratch("isolation-config-out2"));
    let refused = command.output().unwrap();
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("Cargo configuration") && stderr.contains("would apply"),
        "{stderr}"
    );
}

/// The shared resolver is a symlink farm beside the repository. Its generated
/// manifest may name any source path, so the engine, run as a person or CI
/// runs it with `SIMDOC_CARGO_MANIFEST_PATH`, refuses every target path
/// outside the package's bound closure before anything is built or read.
#[test]
fn a_farm_manifest_cannot_point_cargo_outside_the_bound_closure() {
    let real_cargo = PathBuf::from(env!("CARGO"));
    let repo = fixture("isolation-farm");
    let farm = repo.parent().unwrap().join("sim-private/.meta-workspace");
    let package = farm.join("packages/app");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        farm.join("Cargo.toml"),
        "[workspace]\nresolver = \"3\"\nmembers = [\"packages/app\"]\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(repo.join("src"), package.join("src")).unwrap();
    let evil = repo.parent().unwrap().join("evil.rs");
    fs::write(&evil, "fn main() {}\n").unwrap();
    let manifest = |extra: &str| {
        fs::write(
            package.join("Cargo.toml"),
            format!(
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n{extra}"
            ),
        )
        .unwrap();
        let status = Command::new(&real_cargo)
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(farm.join("Cargo.toml"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    };
    let run = || {
        Command::new(engine_binary())
            .env_clear()
            .env("HOME", home())
            .env("CARGO", &real_cargo)
            .env("SIMDOC_CARGO_MANIFEST_PATH", farm.join("Cargo.toml"))
            .current_dir(&repo)
            .args(["simdoc", "--repo-root", ".", "--rustdoc", "skip"])
            .output()
            .unwrap()
    };
    manifest("");
    let honest = run();
    assert!(
        honest.status.success(),
        "{}",
        String::from_utf8_lossy(&honest.stderr)
    );
    for extra in [
        format!("[lib]\npath = \"{}\"\n", evil.display()),
        format!("build = \"{}\"\n", evil.display()),
        format!("[[bin]]\nname = \"x\"\npath = \"{}\"\n", evil.display()),
    ] {
        manifest(&extra);
        let refused = run();
        assert!(!refused.status.success(), "{extra}");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(
            stderr.contains("not part of the package's bound source closure"),
            "{extra}: {stderr}"
        );
    }
}

fn copy_tooling(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "target" || name == ".git" {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_tooling(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The launcher refuses an engine whose source or lock is not the committed
/// one. A copy of the tree is built into its own xtask (which bakes its own
/// root), then the copy's engine source and the copy's engine lock are
/// changed one at a time after the pin: each is refused with the pin's own
/// message before any engine runs. Removing `verify_source` from the launcher
/// makes this test fail.
#[test]
fn a_launcher_refuses_an_engine_whose_source_or_lock_changed_after_the_pin() {
    let dir = scratch("isolation-mutated-engine");
    let root = dir.join("tooling");
    copy_tooling(&tooling_root(), &root);
    let build = Command::new(env!("CARGO"))
        .current_dir(&root)
        .args([
            "build",
            "--quiet",
            "--locked",
            "--offline",
            "--bin",
            "xtask",
        ])
        .arg("--target-dir")
        .arg(dir.join("target"))
        .status()
        .unwrap();
    assert!(build.success(), "build the copied xtask");
    let xtask = dir.join("target/debug/xtask");
    let repo = fixture("isolation-mutated-engine-repo");
    let refused = |what: &str| {
        let out = scratch("isolation-mutated-engine-out");
        let output = Command::new(&xtask)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &dir)
            .current_dir(&repo)
            .args([
                "repo-contract",
                "--repo",
                ".",
                "--emit",
                "repo-contract.json",
                "--out-dir",
            ])
            .arg(&out)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{what}: launched");
        assert!(
            stderr.contains("is not the committed one"),
            "{what}: {stderr}"
        );
        assert!(!out.join("repo-contract.json").exists(), "{what}");
    };
    let source = root.join("crates/simdoc/src/lib.rs");
    let original = fs::read(&source).unwrap();
    let mut changed = original.clone();
    changed.extend_from_slice(b"\n// changed after the pin\n");
    fs::write(&source, changed).unwrap();
    refused("source");
    fs::write(&source, original).unwrap();

    let lock = root.join("crates/simdoc/Cargo.lock");
    let original = fs::read(&lock).unwrap();
    let mut changed = original.clone();
    changed.extend_from_slice(b"\n# changed after the pin\n");
    fs::write(&lock, changed).unwrap();
    refused("lock");
    fs::write(&lock, original).unwrap();
    fs::remove_dir_all(dir).unwrap();
}
