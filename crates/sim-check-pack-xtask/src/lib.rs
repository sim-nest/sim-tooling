// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Isolated checker command and in-process qualification adapter.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

#[path = "../../../src/check_pack.rs"]
pub mod check_pack;
#[cfg(test)]
#[path = "../../../src/check_pack_test_support.rs"]
mod check_pack_test_support;

pub use check_pack::{CheckPackOptions, execute_with_owner_currentness};
pub use cold_stage::admit_cold_stage;
pub use consumers::admit_consumers;
pub use facet_transaction::admit_facet_transaction;
pub use git_release::admit_git_release;
pub use negatives::admit_negatives;
pub use neutral_input::admit_neutral_input;
pub use owner_receipt::{OwnerReceiptRefusal, admit_executed_journal};
pub use portable_seams::admit_portable_seams;
pub use release_obligations::admit_release_obligations;
pub use roadmap_authority::admit_roadmap_authority;
pub use sealed_convergence::admit_sealed_convergence;
pub use stage_one::admit_stage_one;

mod cold_stage;
mod consumers;
mod facet_transaction;
mod git_release;
mod negatives;
mod neutral_input;
mod owner_receipt;
mod portable_seams;
mod release_obligations;
mod roadmap_authority;
mod sealed_convergence;
mod stage_one;

/// Dispatches the isolated checker command.
pub fn run(args: Vec<String>) -> Result<(), String> {
    check_pack::run(args)
}
