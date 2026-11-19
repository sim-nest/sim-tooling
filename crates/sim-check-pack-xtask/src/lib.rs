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
pub use git_release::admit_git_release;
pub use owner_receipt::{OwnerReceiptRefusal, admit_executed_journal};

mod git_release;
mod owner_receipt;

/// Dispatches the isolated checker command.
pub fn run(args: Vec<String>) -> Result<(), String> {
    check_pack::run(args)
}
