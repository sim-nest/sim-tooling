//! Exact native invocation of SIM's public conformance packs.
//!
//! This crate owns the small typed adapter named by the native checker binding.
//! It has no process, filesystem, proof-store, or revocation authority. The
//! caller must hold the weak operation-local handle issued by the live SDK
//! owner, and the returned value remains bound to that owner generation.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use sim_conformance_packs::{
    OperationLocalAuthority, OperationLocalAuthorityError, OperationLocalEvidence,
    OperationLocalInvoker, OperationLocalSupportDefinition, QualifiedOperationLocalCheck,
};

mod prepared;
pub use prepared::{PreparedOperationLocalCheck, prepare_operation_local};

/// Code-only implementation selected by the protected SIM boot.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeOperationLocalInvoker;

/// Returns the exact build-input identity compiled into this native adapter.
#[must_use]
pub const fn build_inputs_sha256() -> &'static str {
    env!("SIM_CHECK_PACK_BUILD_INPUTS_SHA256")
}

/// Returns the exact resolver lock selected for this adapter build.
#[must_use]
pub const fn dependency_lock_sha256() -> &'static str {
    env!("SIM_CHECK_PACK_DEPENDENCY_LOCK_SHA256")
}

/// Executes C-OP `operation/local` through its exact native call binding.
///
/// Subject and input-closure identities are derived from the frozen evidence;
/// callers cannot pair the bytes with substitute identities. Every fact must
/// also have one independently identified member in `support_definition`.
/// Typed support and provenance bind that complete definition, this adapter's
/// measured build graph, the checker's measured build graph and the exact
/// native invocation. The grade is deliberately bootstrap-scoped because this
/// entrypoint closes NV12.05; it cannot mint later release evidence.
/// Currentness and revocation come only from `authority`, and the opaque result
/// must still be admitted and freshly rechecked by the owning proof catalog.
pub fn invoke_operation_local(
    evidence: &OperationLocalEvidence,
    support_definition: &OperationLocalSupportDefinition,
    authority: &OperationLocalAuthority,
) -> Result<QualifiedOperationLocalCheck, OperationLocalAuthorityError> {
    prepare_operation_local(evidence, support_definition)?.issue(authority)
}

impl OperationLocalInvoker for NativeOperationLocalInvoker {
    fn invoke(
        &self,
        evidence: &OperationLocalEvidence,
        support: &OperationLocalSupportDefinition,
        authority: &OperationLocalAuthority,
    ) -> Result<QualifiedOperationLocalCheck, OperationLocalAuthorityError> {
        invoke_operation_local(evidence, support, authority)
    }
}

#[cfg(test)]
mod tests;
