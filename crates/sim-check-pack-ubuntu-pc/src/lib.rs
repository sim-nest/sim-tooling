//! Ubuntu composition for opaque native `operation/local` evidence.
//!
//! This crate owns no execution or currentness authority. Its only production
//! input is the platform owner's opaque, exact-role corpus. It refuses before
//! constructing evidence if that corpus cannot account for all 22 checker
//! facts through independently verified native roles. The current baseline
//! soundly accounts for thirteen facts (eleven shared j/k facts plus successful
//! and failing test observation) and refuses the remaining nine atomically.

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
/// and failing test observation. Incomplete roles remain bound into the corpus
/// identity but are not promoted into unrelated assertions. Until typed native
/// roles cover the remaining facts, this function returns one atomic refusal
/// and constructs no evidence or support definition.
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
