// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Command routes over the repo-contract engine.
//!
//! - `repo-contract [--check] [--repo <path>] [--emit <artifact>... --out-dir <path>]`
//!   writes, checks, or emits the per-repo contract artifacts. The emission
//!   form is the `xtask-repo-contract-v1` wire interface: it writes only the
//!   named artifacts into a preopened directory and never touches the
//!   repository.
//! - `repo-packages [--repo <path>]` prints the covered package metadata and
//!   the package grouping cut as one JSON document, for tools that need the
//!   contract's package set without its artifacts.
//!
//! The xtask routes run this executable, built from its own locked resolver
//! root, so every contract byte comes from one dependency identity.

use std::{
    collections::BTreeSet,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde_json::json;

use crate::{
    generator_options::find_repo_root,
    repo_contract::{
        RepoContractReport, cargo_metadata, contract_artifacts, repo_contract_for_repo,
        workspace_package_names,
    },
    repo_contract_cut::load_or_derive_split_cut,
};

const USAGE: &str =
    "usage: repo-contract [--check] [--repo <path>] [--emit <artifact>... --out-dir <path>]";

const CONTRACT_ARTIFACT_NAMES: [&str; 11] = [
    "card-index.json",
    "card-index.md",
    "feature-map.json",
    "feature-map.md",
    "provenance.json",
    "repo-contract.json",
    "repo-contract.md",
    "rustdoc-index.json",
    "rustdoc-index.md",
    "sim-index-fragment.claims.sx",
    "sim-index-fragment.sx",
];

#[derive(Debug)]
struct RepoContractOptions {
    repo: PathBuf,
    check: bool,
    emission: Option<EmissionOptions>,
}

#[derive(Debug)]
struct EmissionOptions {
    names: Vec<String>,
    out_dir: PathBuf,
}

/// Runs `repo-contract`; `args[1]` is the subcommand name.
pub(crate) fn run_repo_contract(args: &[String]) -> Result<(), String> {
    let options = parse_options(args)?;
    if let Some(emission) = options.emission {
        let report = emit_contract_artifacts(&options.repo, &emission.names, &emission.out_dir)?;
        println!(
            "repo-contract: {} package(s), {} artifact(s) emitted",
            report.packages, report.artifacts_changed
        );
        return Ok(());
    }
    let report = repo_contract_for_repo(options.check, &options.repo)?;
    if options.check {
        println!("repo-contract: generated contract files are current");
        return Ok(());
    }
    println!(
        "repo-contract: {} package(s), {} artifact(s) changed",
        report.packages, report.artifacts_changed
    );
    Ok(())
}

/// Runs `repo-packages`; `args[1]` is the subcommand name.
pub(crate) fn run_repo_packages(args: &[String]) -> Result<(), String> {
    let repo = match args.get(2..).unwrap_or_default() {
        [] => find_repo_root(&env::current_dir().map_err(display_io)?)?,
        [flag, path] if flag == "--repo" => {
            find_repo_root(&PathBuf::from(path).canonicalize().map_err(display_io)?)?
        }
        _ => return Err("usage: repo-packages [--repo <path>]".to_owned()),
    };
    let repo = repo.canonicalize().map_err(display_io)?;
    let metadata = cargo_metadata(&repo)?;
    let cut = load_or_derive_split_cut(&repo, &workspace_package_names(&metadata)?)?;
    let document = json!({
        "schema": "sim.repo-packages.v1",
        "metadata": metadata,
        "group_order": cut.group_order,
        "groups": cut.groups,
    });
    let text = serde_json::to_string(&document).map_err(|err| err.to_string())?;
    println!("{text}");
    Ok(())
}

fn parse_options(args: &[String]) -> Result<RepoContractOptions, String> {
    let mut repo = None;
    let mut check = false;
    let mut names = Vec::new();
    let mut out_dir = None;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--check" => check = true,
            "--repo" | "--emit" | "--out-dir" => {
                let flag = args[index].as_str();
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("{flag} requires a value\n{USAGE}"))?;
                match flag {
                    "--repo" => repo = Some(PathBuf::from(value)),
                    "--emit" => names.push(value.clone()),
                    _ => out_dir = Some(PathBuf::from(value)),
                }
            }
            "-h" | "--help" => return Err(USAGE.to_owned()),
            other => return Err(format!("unknown repo-contract argument `{other}`\n{USAGE}")),
        }
        index += 1;
    }
    if check && (!names.is_empty() || out_dir.is_some()) {
        return Err(format!(
            "--check and --emit are mutually exclusive\n{USAGE}"
        ));
    }
    if names.is_empty() && out_dir.is_some() {
        return Err(format!("--out-dir requires --emit\n{USAGE}"));
    }
    let emission = if names.is_empty() {
        None
    } else {
        let out_dir = out_dir.ok_or_else(|| format!("--emit requires --out-dir\n{USAGE}"))?;
        let mut unique = BTreeSet::new();
        for name in &names {
            if !unique.insert(name.clone()) {
                return Err(format!("duplicate repo-contract artifact `{name}`"));
            }
            if !CONTRACT_ARTIFACT_NAMES.contains(&name.as_str()) {
                return Err(format!("unknown repo-contract artifact `{name}`"));
            }
        }
        Some(EmissionOptions { names, out_dir })
    };
    let start = match repo {
        Some(path) => path.canonicalize().map_err(display_io)?,
        None => env::current_dir().map_err(display_io)?,
    };
    Ok(RepoContractOptions {
        repo: find_repo_root(&start)?,
        check,
        emission,
    })
}

fn emit_contract_artifacts(
    repo: &Path,
    names: &[String],
    out_dir: &Path,
) -> Result<RepoContractReport, String> {
    validate_preopened_directory(out_dir)?;
    let out_dir = out_dir.canonicalize().map_err(display_io)?;
    let artifacts = contract_artifacts(repo)?;
    for name in names {
        let content = artifacts
            .files
            .get(name.as_str())
            .ok_or_else(|| format!("unknown repo-contract artifact `{name}`"))?;
        atomic_write(&out_dir, name, content.as_bytes())?;
    }
    Ok(RepoContractReport {
        packages: artifacts.package_count,
        artifacts_changed: names.len(),
    })
}

fn validate_preopened_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(display_io)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "output directory must be a preopened real directory: {}",
            path.display()
        ));
    }
    Ok(())
}

fn atomic_write(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    let target = dir.join(name);
    if let Ok(metadata) = fs::symlink_metadata(&target)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(format!(
            "refusing non-file output target: {}",
            target.display()
        ));
    }
    let stage = dir.join(format!(".{name}.sim-stage-{}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stage)
        .map_err(|err| format!("create atomic output stage {}: {err}", stage.display()))?;
    let result = (|| {
        file.write_all(bytes).map_err(display_io)?;
        file.sync_all().map_err(display_io)?;
        fs::rename(&stage, &target).map_err(display_io)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&stage);
    }
    result
}

fn display_io(err: std::io::Error) -> String {
    err.to_string()
}

#[cfg(test)]
#[path = "repo_contract_cli_tests.rs"]
mod tests;
