// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Ubuntu composition for opaque native `operation/local` evidence.
//!
//! This crate owns no execution or currentness authority. Its only production
//! input is the platform owner's opaque, exact-role corpus. It refuses before
//! constructing evidence if that corpus cannot account for all 22 checker
//! facts through independently verified native roles. A corpus with admitted
//! stop, formatter, owner-command and refusal specimens accounts for all
//! twenty-two facts: eleven shared j/k facts, successful and failing test
//! observation, timeout and cancellation group termination, the bounded
//! single-kill stop, an observed formatter mutation, byte-exact manifest
//! scripts, exact validation and docs commands, reachable later owner gates,
//! and the refusal of model-interpolated commands. Any missing role is refused
//! atomically, naming every missing fact.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod composition;
mod contract;
mod model;

pub use contract::MISSING_PLATFORM_WITNESS_ROLES;
pub use model::{UbuntuOperationLocalCorpusError, UbuntuOperationLocalInputs};

use sim_platform_ubuntu_pc::VerifiedOperationLocalCorpus;

/// Composes all 22 C-OP facts solely from an opaque platform corpus.
///
/// Both completed roles establish eleven closed baseline facts plus successful
/// and failing test observation; admitted stop specimens establish timeout and
/// cancellation group termination and the bounded single-kill stop; an
/// admitted formatter checkout specimen establishes the formatter mutation;
/// admitted owner-command specimens establish the four manifest facts; an
/// admitted refusal specimen establishes model-interpolation refusal.
/// Incomplete roles remain bound into the corpus identity but are not promoted
/// into unrelated assertions. A corpus lacking any admission gets one atomic
/// refusal naming every missing role, and no evidence or support definition.
///
/// # Errors
///
/// Returns every missing native role, or a canonical construction error after
/// all required roles are present.
pub fn compose_operation_local(
    corpus: &VerifiedOperationLocalCorpus,
) -> Result<UbuntuOperationLocalInputs, UbuntuOperationLocalCorpusError> {
    composition::compose(corpus)
}

/// Exact build-input identity compiled into this Ubuntu producer.
#[must_use]
pub const fn build_inputs_sha256() -> &'static str {
    env!("SIM_CHECK_PACK_UBUNTU_BUILD_INPUTS_SHA256")
}

/// Exact resolver-lock identity compiled into this Ubuntu producer.
#[must_use]
pub const fn dependency_lock_sha256() -> &'static str {
    env!("SIM_CHECK_PACK_UBUNTU_DEPENDENCY_LOCK_SHA256")
}

#[cfg(test)]
mod tests;
