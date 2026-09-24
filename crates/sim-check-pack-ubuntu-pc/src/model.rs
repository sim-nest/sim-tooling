//! Public values emitted by the closed corpus composition.

use std::fmt;

use sim_check_pack::PreparedOperationLocalCheck;
use sim_conformance_packs::{
    OperationLocalAuthorityError, OperationLocalEvidence, OperationLocalSupportDefinition,
};
use sim_kernel::ContentId;

/// Refusal while composing opaque platform evidence into checker inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UbuntuOperationLocalCorpusError {
    /// The platform corpus does not yet expose all typed roles required by C-OP.
    MissingNativeRoles(Vec<&'static str>),
    /// Two semantic roles resolve to the same acceptance identity.
    DuplicateNativeRole(&'static str),
    /// A role resolves to the wrong class or shares another role's binding.
    AmbiguousNativeRole(&'static str),
    /// Canonical evidence or support construction failed closed.
    InvalidComposition(String),
}

impl fmt::Display for UbuntuOperationLocalCorpusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingNativeRoles(roles) => {
                write!(
                    formatter,
                    "missing verified native roles: {}",
                    roles.join(", ")
                )
            }
            Self::DuplicateNativeRole(role) => {
                write!(formatter, "duplicate verified native role: {role}")
            }
            Self::AmbiguousNativeRole(role) => {
                write!(formatter, "ambiguous verified native role: {role}")
            }
            Self::InvalidComposition(detail) => detail.fmt(formatter),
        }
    }
}

impl std::error::Error for UbuntuOperationLocalCorpusError {}

/// Opaque-derived inputs for the exact neutral `operation/local` checker.
///
/// This type has no public constructor from facts or support identities. It is
/// produced only after [`crate::compose_operation_local`] accounts for every
/// fact from platform-owner-verified roles.
#[derive(Clone, Debug)]
pub struct UbuntuOperationLocalInputs {
    pub(crate) evidence: OperationLocalEvidence,
    pub(crate) support: OperationLocalSupportDefinition,
    pub(crate) corpus: ContentId,
    pub(crate) definition: ContentId,
}

impl UbuntuOperationLocalInputs {
    /// Canonical evidence derived from the complete verified corpus.
    #[must_use]
    pub const fn evidence(&self) -> &OperationLocalEvidence {
        &self.evidence
    }

    /// Canonical fact-to-native-witness support definition.
    #[must_use]
    pub const fn support_definition(&self) -> &OperationLocalSupportDefinition {
        &self.support
    }

    /// Identity of the exact opaque platform corpus consumed by this value.
    #[must_use]
    pub const fn corpus_identity(&self) -> &ContentId {
        &self.corpus
    }

    /// Identity of the immutable platform role/binding definition.
    #[must_use]
    pub const fn definition_identity(&self) -> &ContentId {
        &self.definition
    }

    /// Runs authority-free preparation through the neutral checker adapter.
    ///
    /// # Errors
    ///
    /// Refuses any checker build mismatch or semantic evidence failure.
    pub fn prepare(&self) -> Result<PreparedOperationLocalCheck<'_>, OperationLocalAuthorityError> {
        sim_check_pack::prepare_operation_local(&self.evidence, &self.support)
    }
}
