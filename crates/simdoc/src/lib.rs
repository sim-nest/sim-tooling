// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Standalone source and resolver boundary for SIM generated documentation.

#![deny(unsafe_code)]
#![deny(missing_docs)]

mod bounded_process;
mod cargo_home;
// Shared with the build script; the toolchain rules are used at run time too.
#[allow(dead_code)]
#[path = "../build_identity.rs"]
mod build_identity;
mod cardspine;
mod cardspine_state;
mod content_digest;
mod crate_catalog;
mod crate_catalog_manifest;
mod docencoder;
mod generator_options;
mod index_anchor_scan;
mod index_author;
mod index_composition;
mod index_fragment;
mod index_specimen_scan;
mod index_surface_scan;
mod json_render;
mod owned;
mod publication;
mod recipe_evidence;
mod recipe_gate;
mod repo_contract;
mod repo_contract_cli;
mod repo_contract_cut;
mod repo_contract_render;
mod repo_contract_scan;
#[cfg(test)]
mod resolver_boundary_tests;
#[cfg(test)]
mod resolver_farm_tests;
mod resolver_input;
mod simdoc;
mod simdoc_index;
mod simdoc_rustdoc;
#[cfg(test)]
mod test_fixture;
mod tools;
mod validation_matrix;
mod worktree;

pub use cardspine::{CARD_CONTENT_ID_ALGORITHM, Card, CardSpine, card_content_id};
pub use docencoder::{DocEncoder, DocPosition};
pub use repo_contract::{RepoContractReport, repo_contract};

#[cfg(test)]
pub(crate) fn tooling_checkout_root() -> std::path::PathBuf {
    let manifest_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_root
        .parent()
        .and_then(|dir| dir.parent())
        .expect("simdoc package remains under its sim-tooling checkout")
        .to_path_buf();
    assert!(root.join(".git").exists());
    assert!(root.join("Cargo.toml").is_file());
    root
}

/// Runs one simdoc command: `simdoc` (the documentation lanes),
/// `repo-contract` (write, check, or emit the repo contract artifacts),
/// `crate-catalog`, `validation-matrix`, or `identity` (the build-time
/// identity launchers verify before trusting this executable).
pub fn run(args: Vec<String>) -> Result<(), String> {
    match args.get(1).map(String::as_str) {
        Some("identity") => {
            print!("{}", repo_contract::identity_lines());
            Ok(())
        }
        Some("repo-contract") => repo_contract_cli::run_repo_contract(&args),
        Some("crate-catalog") => crate_catalog::run(&args),
        Some("validation-matrix") => validation_matrix::run(&args),
        _ => simdoc::run(args),
    }
}
