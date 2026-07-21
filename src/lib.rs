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
//! - `index doctor` -- scan generated index fragments for unclaimed discoveries.
//! - `index seed` -- extract private migration seed rows from legacy markdown.
//! - `index merge`, `index render`, `index find`, `index overlap`, and
//!   `index snapshot` -- build, query, and stage the public constellation index.
//! - `index-check` -- gate generated index fragment freshness and coverage.
//! - `check-file-sizes` -- gate Rust source files against repository hard limits.
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
mod dispatch;
mod docencoder;
mod file_size_gate;
mod generator_options;
mod index_anchor_scan;
mod index_author;
mod index_check;
mod index_doctor;
mod index_find;
mod index_fragment;
mod index_merge;
mod index_overlap;
mod index_render;
mod index_render_features;
mod index_rules;
mod index_seed;
mod index_snapshot;
mod index_source;
mod index_specimen_scan;
mod index_surface_scan;
mod repo_contract;
mod repo_contract_cut;
mod repo_contract_render;
mod repo_contract_scan;
mod simdoc;
mod simdoc_index;
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
    dispatch::dispatch(args)
}
