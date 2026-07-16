//! xtask: the SIM constellation build and documentation tool.
//!
//! xtask runs repository maintenance and documentation tasks over a SIM repo
//! checkout. Each task has a `--check` mode that fails when generated artifacts
//! are stale, so the same code both generates the artifacts and gates them.
//!
//! # Commands
//!
//! - `simdoc` -- build, or `--check`, the documentation lanes: API docs, agent
//!   cards, human docs, diagrams, and split contract files under `docs/`.
//! - `repo-contract` -- generate or check the per-repo contract files.
//! - `validation-matrix` -- generate or check the validation matrix.
//! - `crate-catalog` -- generate or check crate metadata, READMEs, and the
//!   crate catalog.
//! - `citizenize` -- rewrite a crate or path toward the citizen conventions.
//! - `atelier-site` -- generate or check the Atelier Studio Site graph cache.
//! - `atelier-cassette`, `atelier-capsule`, and `atelier-index` -- check caches.
//! - `atelier-radar` -- query ranked confidence hints over the Atelier index.
//! - `atelier-guard`, `atelier-tools`, and `atelier-shell` -- check agent
//!   maintenance surfaces.
//!
//! [`run`] dispatches an argument vector to the matching task. The library also
//! exposes each task's entry point and report type, including
//! [`repo_contract`], [`validation_matrix`], [`crate_catalog`],
//! [`citizenize_arg`], [`atelier_site`], and [`atelier_tools`].

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod atelier;

mod cardspine;
mod cardspine_state;
mod citizenize;
mod crate_catalog;
mod crate_catalog_manifest;
mod docencoder;
mod generator_options;
mod repo_contract;
mod repo_contract_cut;
mod repo_contract_render;
mod repo_contract_scan;
mod simdoc;
mod simdoc_rustdoc;
mod validation_matrix;

pub use atelier::{
    AtelierGuardOptions, AtelierGuardReport, AtelierLayer, AtelierNode, AtelierNodeKind,
    AtelierSite, AtelierSiteOptions, AtelierSiteReport, AtelierToolAction, AtelierToolDescriptor,
    AtelierToolsOptions, AtelierToolsReport, GuidelineFinding, GuidelineRule, GuidelineSeverity,
    atelier_guard, atelier_site, atelier_tools,
};
pub use cardspine::{CARD_CONTENT_ID_ALGORITHM, Card, CardSpine, card_content_id};
pub use citizenize::{CitizenizeReport, citizenize_arg, citizenize_path};
pub use crate_catalog::{CrateCatalogReport, crate_catalog};
pub use docencoder::{DocEncoder, DocPosition};
pub use repo_contract::{RepoContractReport, repo_contract};
pub use validation_matrix::{ValidationMatrixReport, validation_matrix};

/// Dispatches an xtask command-line argument vector to the matching task.
///
/// `args` is the full process argument vector. Remaining arguments select a task
/// such as `simdoc`, `repo-contract`, `atelier-capsule`, or `atelier-shell` and
/// its optional flags or argument.
/// Returns a usage error for an unrecognized command.
pub fn run(args: Vec<String>) -> Result<(), String> {
    if matches!(args.as_slice(), [_, command, ..] if command == "simdoc") {
        return simdoc::run(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-site") {
        return atelier::run(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-cassette") {
        return atelier::run_cassette(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-capsule") {
        return atelier::run_capsule(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-index") {
        return atelier::run_index(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-radar") {
        return atelier::run_radar(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-guard") {
        return atelier::run_guard(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-tools") {
        return atelier::run_tools(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-shell") {
        return atelier::run_shell(args);
    }

    match args.as_slice() {
        [_, command, ..] if command == "repo-contract" => {
            let options = generator_options::parse_repo_tool_args(&args, command)?;
            let report = repo_contract::repo_contract_for_repo(options.check, &options.repo)?;
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
        [_, command, ..] if command == "validation-matrix" => {
            let options = generator_options::parse_repo_tool_args(&args, command)?;
            let report = validation_matrix::validation_matrix_for_repo(options.check, &options.repo)?;
            if options.check {
                println!("validation-matrix: generated matrix is current");
                return Ok(());
            }
            println!(
                "validation-matrix: {} row(s), {} artifact(s) changed",
                report.rows, report.artifacts_changed
            );
            Ok(())
        }
        [_, command, ..] if command == "crate-catalog" => {
            let options = generator_options::parse_repo_tool_args(&args, command)?;
            let report = crate_catalog(options.check, Some(options.repo))?;
            if options.check {
                println!("crate-catalog: metadata and generated files are current");
            } else {
                println!(
                    "crate-catalog: {} package(s), {} manifest(s), {} readme(s), {} catalog file(s)",
                    report.packages,
                    report.manifests_changed,
                    report.readmes_changed,
                    report.catalogs_changed
                );
            }
            Ok(())
        }
        [_, command, target] if command == "citizenize" => {
            let report = citizenize_arg(target)?;
            println!(
                "citizenize: {} candidate(s), {} file(s) changed",
                report.candidates, report.files_changed
            );
            Ok(())
        }
        [program, ..] => Err(format!("usage: {program} <repo-contract [--check] [--repo <path>]|validation-matrix [--check] [--repo <path>]|crate-catalog [--check] [--repo <path>]|citizenize <crate-name-or-path>|simdoc [--check] [--rustdoc auto|skip|force]|atelier-site [--check]|atelier-cassette [--check]|atelier-capsule [--check]|atelier-index [--check]|atelier-radar <query>|atelier-guard [--check]|atelier-tools [--check]|atelier-shell [--check]>")),
        [] => Err("usage: xtask <repo-contract [--check] [--repo <path>]|validation-matrix [--check] [--repo <path>]|crate-catalog [--check] [--repo <path>]|citizenize <crate-name-or-path>|simdoc [--check] [--rustdoc auto|skip|force]|atelier-site [--check]|atelier-cassette [--check]|atelier-capsule [--check]|atelier-index [--check]|atelier-radar <query>|atelier-guard [--check]|atelier-tools [--check]|atelier-shell [--check]>".to_owned()),
    }
}
