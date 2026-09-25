// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Routes to the one repo-contract engine.
//!
//! The contract engine lives in `crates/simdoc`, its own resolver root with a
//! committed lock. xtask never compiles any of it: every xtask route that needs
//! contract output (`repo-contract`, including the `--emit` wire interface,
//! `index-check`, `crate-catalog`, and `validation-matrix`) runs that locked
//! executable, so one dependency identity produces every contract byte.

use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

/// Manifest of the locked contract engine inside this sim-tooling checkout.
pub(crate) fn simdoc_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/simdoc/Cargo.toml")
}

/// Runs one simdoc subcommand with `args`, printing its standard output and
/// returning its diagnostic text as the error when it refuses.
pub(crate) fn run_forwarded(subcommand: &str, args: &[String]) -> Result<(), String> {
    let stdout = run_captured(subcommand, args)?;
    print!("{stdout}");
    Ok(())
}

/// Returns the exact bytes of one repo-contract artifact for `repo`, emitted
/// by the locked engine into a private scratch directory.
pub(crate) fn emitted_artifact(repo: &Path, name: &str) -> Result<String, String> {
    let scratch = scratch_directory()?;
    let result = (|| {
        run_captured(
            "repo-contract",
            &[
                "--repo".to_owned(),
                path_arg(repo)?,
                "--emit".to_owned(),
                name.to_owned(),
                "--out-dir".to_owned(),
                path_arg(&scratch)?,
            ],
        )?;
        fs::read_to_string(scratch.join(name)).map_err(|err| format!("read emitted {name}: {err}"))
    })();
    let _ = fs::remove_dir_all(&scratch);
    result
}

/// Returns the contract's package set for `repo`: `metadata` (the merged
/// `cargo metadata` of every covered workspace), `group_order`, and `groups`.
pub(crate) fn repo_packages(repo: &Path) -> Result<Value, String> {
    let stdout = run_captured("repo-packages", &["--repo".to_owned(), path_arg(repo)?])?;
    let document: Value = serde_json::from_str(&stdout)
        .map_err(|err| format!("parse simdoc repo-packages output: {err}"))?;
    if document["schema"] != "sim.repo-packages.v1" {
        return Err("simdoc repo-packages returned an unknown schema".to_owned());
    }
    Ok(document)
}

fn run_captured(subcommand: &str, args: &[String]) -> Result<String, String> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let output = Command::new(cargo)
        .args(["run", "--quiet", "--locked", "--manifest-path"])
        .arg(simdoc_manifest())
        .arg("--")
        .arg(subcommand)
        .args(args)
        .output()
        .map_err(|err| format!("start simdoc {subcommand}: {err}"))?;
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| format!("simdoc {subcommand} printed non-UTF-8 output"))?;
    if output.status.success() {
        return Ok(stdout);
    }
    let diagnostic = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(if diagnostic.is_empty() {
        format!("simdoc {subcommand} failed with {}", output.status)
    } else {
        diagnostic
    })
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
