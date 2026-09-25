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
//! The executable is built and run from the sim-tooling root with the
//! environment's toolchain selectors (`RUSTUP_TOOLCHAIN`, `RUSTC`) removed, so
//! rustup resolves the channel pinned in sim-tooling's `rust-toolchain.toml`;
//! simdoc's build refuses any other compiler and records the one it used. The
//! caller's `CARGO` variable is deliberately ignored: it names the cargo of
//! whatever toolchain launched xtask, which is not the pinned identity.
//! Output is never captured: the child's standard streams pass straight
//! through, so nothing unbounded is buffered here.

use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

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
    let status = engine_command(subcommand, &args)
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
        let status = engine_command("repo-contract", &args)
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

/// The engine invocation: `cargo run --locked` on the simdoc manifest, from
/// the sim-tooling root, with the toolchain selectors removed.
pub(crate) fn engine_command(subcommand: &str, args: &[String]) -> Command {
    let root = tooling_root();
    let mut command = Command::new("cargo");
    command
        .current_dir(&root)
        .env_remove("RUSTUP_TOOLCHAIN")
        .env_remove("RUSTC")
        .args(["run", "--quiet", "--locked", "--manifest-path"])
        .arg(root.join("crates/simdoc/Cargo.toml"))
        .arg("--")
        .arg(subcommand)
        .args(args);
    command
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
    fn the_engine_runs_locked_from_the_pinned_tooling_root() {
        let command = engine_command("repo-contract", &["--check".to_owned()]);
        assert_eq!(command.get_program(), "cargo");
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
        let removed = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_owned())
            .collect::<Vec<OsString>>();
        assert!(removed.contains(&OsString::from("RUSTUP_TOOLCHAIN")));
        assert!(removed.contains(&OsString::from("RUSTC")));
        assert!(
            command
                .get_envs()
                .all(|(name, value)| name != "CARGO" || value.is_none())
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
