// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Routes to the one contract engine.
//!
//! The contract engine lives in `crates/simdoc`, its own resolver root with a
//! committed lock. xtask never compiles any of it: every xtask route that
//! produces contract output (`repo-contract` including the `--emit` wire
//! interface, `crate-catalog`, `validation-matrix`, and `index-check`'s
//! freshness comparison) runs that locked executable.
//!
//! Before anything runs, [`crate::simdoc_pin`] verifies the engine against
//! the identity this repository commits: its source and lock digests, the
//! pinned toolchain, and a toolchain directory whose `cargo`, `rustc`,
//! `rustdoc`, and libraries hash to the committed digest and report the
//! committed commits. Nothing is looked up on `PATH` and `rustup` is never
//! run. The engine is built and run with exactly those binaries from the
//! sim-tooling root under a cleared environment (compiler wrappers and flag
//! variables cannot reach it), and its embedded identity must match before
//! any real command runs. Output is never captured: the child's standard
//! streams pass straight through, so nothing unbounded is buffered here.

use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::simdoc_pin::{self, EncoderPin, Toolchain};

/// Ceiling for one emitted artifact read back by xtask.
const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
/// Flags whose value is a path, resolved against the caller's directory
/// before the engine runs from the sim-tooling root.
const PATH_FLAGS: [&str; 3] = ["--repo", "--repo-root", "--out-dir"];

/// The sim-tooling checkout this xtask was built from.
pub(crate) fn tooling_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Runs one simdoc subcommand with `args` for the repository at the
/// caller's current directory unless `args` names one.
pub(crate) fn run_forwarded(subcommand: &str, args: &[String]) -> Result<(), String> {
    let caller = env::current_dir().map_err(|err| format!("current dir: {err}"))?;
    let args = anchored_args(&caller, args);
    let status = engine_command(verified_engine()?, subcommand, &args)?
        .status()
        .map_err(|err| format!("start simdoc {subcommand}: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("simdoc {subcommand} refused (status {status})"))
    }
}

/// Returns the exact bytes of one repo-contract artifact for `repo`, emitted
/// by the locked engine into a private scratch directory.
pub(crate) fn emitted_artifact(repo: &Path, name: &str) -> Result<String, String> {
    let scratch = scratch_directory()?;
    let result = (|| {
        let args = [
            "--repo".to_owned(),
            path_arg(repo)?,
            "--emit".to_owned(),
            name.to_owned(),
            "--out-dir".to_owned(),
            path_arg(&scratch)?,
        ];
        let status = engine_command(verified_engine()?, "repo-contract", &args)?
            .stdout(Stdio::null())
            .status()
            .map_err(|err| format!("start simdoc repo-contract: {err}"))?;
        if !status.success() {
            return Err(format!("simdoc repo-contract refused (status {status})"));
        }
        read_bounded(&scratch.join(name))
    })();
    let _ = fs::remove_dir_all(&scratch);
    result
}

/// The engine verified against this repository's committed identity, once
/// per process.
pub(crate) fn verified_engine() -> Result<&'static Toolchain, String> {
    static ENGINE: OnceLock<Result<Toolchain, String>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let root = tooling_root();
            let manifest = fs::read_to_string(root.join("Cargo.toml"))
                .map_err(|err| format!("read the sim-tooling Cargo.toml: {err}"))?;
            let pin = EncoderPin::from_manifest(&manifest)?;
            simdoc_pin::verify_source(&root, &pin)?;
            let toolchain = simdoc_pin::locate_toolchain(&pin, &simdoc_pin::candidate_roots(&pin))?;
            let output = engine_command(&toolchain, "identity", &[])?
                .stderr(Stdio::inherit())
                .output()
                .map_err(|err| format!("build the simdoc engine: {err}"))?;
            if !output.status.success() || output.stdout.len() > 4096 {
                return Err(format!(
                    "the simdoc engine did not report its identity ({})",
                    output.status
                ));
            }
            simdoc_pin::verify_identity(
                &String::from_utf8_lossy(&output.stdout),
                &pin,
                &toolchain,
            )?;
            Ok(toolchain)
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The engine invocation: the accepted toolchain's `cargo run --locked` on the
/// simdoc manifest, from the sim-tooling root, compiling with that toolchain's
/// `rustc` under a cleared environment and no compiler wrapper.
pub(crate) fn engine_command(
    toolchain: &Toolchain,
    subcommand: &str,
    args: &[String],
) -> Result<Command, String> {
    let root = tooling_root();
    let manifest = root.join("crates/simdoc/Cargo.toml");
    let (home, target, real_home) = private_dirs(&root)?;
    let mut command = toolchain.command_in(&toolchain.cargo, &root, &[manifest.as_path()])?;
    command
        .env("CARGO_HOME", home)
        .env("HOME", home)
        .env("CARGO_TARGET_DIR", target)
        .env("SIMDOC_REAL_CARGO_HOME", real_home)
        .args(["run", "--quiet", "--locked", "--manifest-path"])
        .arg(&manifest)
        .arg("--")
        .arg(subcommand)
        .args(args);
    Ok(command)
}

/// The private Cargo home (holding exactly the engine's locked, checksum-
/// verified registry crates), the private target directory (so no built
/// artifact or fingerprint of anyone else's is ever reused), and the real
/// Cargo home the engine draws its own private homes from. Created once per
/// process.
fn private_dirs(root: &Path) -> Result<&'static (PathBuf, PathBuf, PathBuf), String> {
    static DIRS: OnceLock<Result<(PathBuf, PathBuf, PathBuf), String>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let real = crate::cargo_home::real_cargo_home(
            None,
            env::var_os("CARGO_HOME"),
            env::var_os("HOME"),
        )?;
        let lock = fs::read_to_string(root.join("crates/simdoc/Cargo.lock"))
            .map_err(|err| format!("read the engine's Cargo.lock: {err}"))?;
        let home = crate::cargo_home::hermetic_home(&real, Some(&lock), "engine-home")?;
        let target = crate::cargo_home::private_dir("engine-target")?;
        Ok((home, target, real))
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// Resolves every path-valued flag against `caller`, and names `caller` as
/// the repository when no repository flag is given.
pub(crate) fn anchored_args(caller: &Path, args: &[String]) -> Vec<String> {
    let mut anchored = Vec::with_capacity(args.len() + 2);
    let mut names_repo = false;
    let mut index = 0;
    while index < args.len() {
        let flag = &args[index];
        anchored.push(flag.clone());
        if PATH_FLAGS.contains(&flag.as_str())
            && let Some(value) = args.get(index + 1)
        {
            names_repo |= flag != "--out-dir";
            anchored.push(caller.join(value).to_string_lossy().into_owned());
            index += 2;
            continue;
        }
        index += 1;
    }
    if !names_repo {
        anchored.push("--repo".to_owned());
        anchored.push(caller.to_string_lossy().into_owned());
    }
    anchored
}

fn read_bounded(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(MAX_ARTIFACT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("read {}: {err}", path.display()))?;
    if bytes.len() as u64 > MAX_ARTIFACT_BYTES {
        return Err(format!(
            "{} exceeds {MAX_ARTIFACT_BYTES} bytes",
            path.display()
        ));
    }
    String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8", path.display()))
}

fn path_arg(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("path is not UTF-8: {}", path.display()))
}

fn scratch_directory() -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_nanos();
    let path = env::temp_dir().join(format!("xtask-simdoc-{}-{stamp}", std::process::id()));
    fs::create_dir(&path).map_err(|err| format!("create {}: {err}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    #[test]
    fn the_engine_runs_locked_with_the_resolved_pinned_binaries() {
        let toolchain = Toolchain {
            cargo: PathBuf::from("/toolchains/1.96.0/bin/cargo"),
            rustc: PathBuf::from("/toolchains/1.96.0/bin/rustc"),
            rustdoc: PathBuf::from("/toolchains/1.96.0/bin/rustdoc"),
            channel: "1.96.0".to_owned(),
            host: "x86_64-unknown-linux-gnu".to_owned(),
        };
        let command = engine_command(&toolchain, "repo-contract", &["--check".to_owned()]).unwrap();
        assert_eq!(command.get_program(), "/toolchains/1.96.0/bin/cargo");
        assert_eq!(command.get_current_dir(), Some(tooling_root().as_path()));
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            [
                "run".to_owned(),
                "--quiet".to_owned(),
                "--locked".to_owned(),
                "--manifest-path".to_owned(),
                tooling_root()
                    .join("crates/simdoc/Cargo.toml")
                    .to_string_lossy()
                    .into_owned(),
                "--".to_owned(),
                "repo-contract".to_owned(),
                "--check".to_owned(),
            ]
        );
        let envs = command
            .get_envs()
            .map(|(name, value)| (name.to_owned(), value.map(ToOwned::to_owned)))
            .collect::<Vec<(OsString, Option<OsString>)>>();
        for (name, value) in [
            ("PATH", Some("/toolchains/1.96.0/bin:/usr/bin:/bin")),
            ("RUSTC", Some("/toolchains/1.96.0/bin/rustc")),
            ("RUSTDOC", Some("/toolchains/1.96.0/bin/rustdoc")),
            ("RUSTC_WRAPPER", Some("")),
            ("RUSTC_WORKSPACE_WRAPPER", Some("")),
        ] {
            assert!(
                envs.contains(&(OsString::from(name), value.map(OsString::from))),
                "{name}"
            );
        }
        // Every cargo run has a private home and target directory: fresh
        // directories of this process, never the caller's or the tree's own
        // `target`, whose artifacts and fingerprints anyone could have planted.
        let value = |name: &str| {
            envs.iter()
                .find(|(key, _)| key == name)
                .and_then(|(_, value)| value.clone())
                .map(PathBuf::from)
                .unwrap_or_else(|| panic!("{name} is not set"))
        };
        let temp = env::temp_dir();
        for name in ["CARGO_HOME", "CARGO_TARGET_DIR"] {
            assert!(value(name).starts_with(&temp), "{name}");
        }
        assert_eq!(value("HOME"), value("CARGO_HOME"));
        assert_ne!(
            value("CARGO_TARGET_DIR"),
            tooling_root().join("crates/simdoc/target")
        );
        assert!(value("SIMDOC_REAL_CARGO_HOME").is_absolute());
        assert!(
            envs.iter()
                .all(|(name, _)| name != "CARGO" && name != "RUSTUP_TOOLCHAIN"),
            "the launcher names no cargo variable and no rustup toolchain"
        );
    }

    #[test]
    fn path_arguments_are_anchored_at_the_caller() {
        let caller = Path::new("/work/sim-platform");
        assert_eq!(
            anchored_args(caller, &["--check".to_owned()]),
            ["--check", "--repo", "/work/sim-platform"]
        );
        assert_eq!(
            anchored_args(
                caller,
                &[
                    "--repo".to_owned(),
                    ".".to_owned(),
                    "--emit".to_owned(),
                    "provenance.json".to_owned(),
                    "--out-dir".to_owned(),
                    "out".to_owned(),
                ]
            ),
            [
                "--repo",
                "/work/sim-platform/.",
                "--emit",
                "provenance.json",
                "--out-dir",
                "/work/sim-platform/out",
            ]
        );
        assert_eq!(
            anchored_args(caller, &["--repo".to_owned(), "/abs/repo".to_owned()]),
            ["--repo", "/abs/repo"]
        );
    }
}
