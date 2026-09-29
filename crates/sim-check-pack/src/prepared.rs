use sim_conformance_core::{
    CheckInvocation, CheckScopeId, CheckerBinding, CheckerResultId, EvidenceGrade,
    EvidenceProvenanceId, EvidenceSetId, PolicyId, content_id_datum,
};
use sim_conformance_packs::{
    CheckObservation, OperationLocalAuthority, OperationLocalAuthorityError,
    OperationLocalEvidence, OperationLocalSupportDefinition, PackRequest, PackVerdict,
    QualifiedOperationLocalCheck, find_pack, operation_local_policy_id, packs,
};
use sim_kernel::{ContentId, Datum, Symbol};

use crate::{build_inputs_sha256, dependency_lock_sha256};

/// Deterministic, authority-free preparation of one exact native check.
///
/// This value proves only that the pure pack accepted the supplied immutable
/// evidence and support under this adapter's measured build graph. It contains
/// no SDK-owner handle, currentness decision, revocation observation, or
/// receipt, and therefore cannot be mistaken for qualified evidence. Live
/// issuance consumes the same prepared binding and invocation through
/// [`Self::issue`].
#[derive(Clone, Debug)]
pub struct PreparedOperationLocalCheck<'a> {
    binding: CheckerBinding,
    invocation: CheckInvocation,
    evidence: &'a OperationLocalEvidence,
    support_definition: &'a OperationLocalSupportDefinition,
    expected_result: CheckerResultId,
    observations: Vec<CheckObservation>,
    grade: EvidenceGrade,
    provenance: EvidenceProvenanceId,
    policy: PolicyId,
    support: EvidenceSetId,
}

impl PreparedOperationLocalCheck<'_> {
    /// Returns the exact native checker binding selected by the pack owner.
    pub const fn binding(&self) -> &CheckerBinding {
        &self.binding
    }

    /// Returns the exact subject/scope/input/native-call invocation.
    pub const fn invocation(&self) -> &CheckInvocation {
        &self.invocation
    }

    /// Returns the result of pure deterministic pack evaluation.
    pub const fn expected_result(&self) -> &CheckerResultId {
        &self.expected_result
    }

    /// Returns the observations covered by the expected result.
    pub fn observations(&self) -> &[CheckObservation] {
        &self.observations
    }

    /// Returns the bootstrap-only grade selected by this adapter.
    pub const fn grade(&self) -> EvidenceGrade {
        self.grade
    }

    /// Returns the exact construction provenance.
    pub const fn provenance(&self) -> &EvidenceProvenanceId {
        &self.provenance
    }

    /// Returns the current operation/local policy identity.
    pub const fn policy(&self) -> &PolicyId {
        &self.policy
    }

    /// Returns the complete supporting-evidence set identity.
    pub const fn support(&self) -> &EvidenceSetId {
        &self.support
    }

    /// Consumes this authority-free preparation and requests a live receipt
    /// from the actual SDK owner.
    pub fn issue(
        self,
        authority: &OperationLocalAuthority,
    ) -> Result<QualifiedOperationLocalCheck, OperationLocalAuthorityError> {
        let expected_binding = self.binding.id().clone();
        let expected_invocation = self.invocation.id().clone();
        let expected_result = self.expected_result.clone();
        let expected_observations = self.observations.clone();
        let expected_grade = self.grade;
        let expected_provenance = self.provenance.clone();
        let expected_policy = self.policy.clone();
        let expected_support = self.support.clone();
        let qualified = authority.check_and_issue(
            self.binding,
            self.invocation,
            self.evidence,
            self.support_definition,
            self.provenance,
            self.support,
        )?;
        if qualified.binding().id() != &expected_binding
            || qualified.invocation().id() != &expected_invocation
            || qualified.receipt().result() != &expected_result
            || qualified.receipt().grade() != expected_grade
            || qualified.receipt().provenance() != &expected_provenance
            || qualified.receipt().policy() != &expected_policy
            || qualified.receipt().support() != &expected_support
            || qualified.observations() != expected_observations
        {
            return Err(OperationLocalAuthorityError::Unsupported);
        }
        Ok(qualified)
    }
}

/// Evaluates the exact native pack without manufacturing live owner authority.
///
/// This is the operator-bootstrap producer seam. It binds evidence, support,
/// adapter source/build inputs, checker source/build inputs, invocation,
/// bootstrap grade and policy. It deliberately cannot issue a receipt or claim
/// currentness. Missing support, a mismatched resolver graph, or any false C-OP
/// fact refuses the whole preparation.
pub fn prepare_operation_local<'a>(
    evidence: &'a OperationLocalEvidence,
    support_definition: &'a OperationLocalSupportDefinition,
) -> Result<PreparedOperationLocalCheck<'a>, OperationLocalAuthorityError> {
    let spec = find_pack("checker/c-op")
        .ok_or(sim_conformance_core::ConformanceError::UnresolvedBinding)?;
    ensure_compatible_build_graph(spec.dependency_lock_sha256(), dependency_lock_sha256())?;
    let binding = spec.native_operation_local_binding()?;
    let invocation = binding.instantiate(
        spec.checker_code_id()?,
        spec.pack_id()?,
        evidence.subject_id()?,
        CheckScopeId::from_text("operation/local")?,
        evidence.input_closure_id()?,
    )?;
    support_definition.verify_evidence(evidence)?;
    let support = support_identity(&invocation, support_definition, spec)?;
    let provenance = provenance_identity(&invocation, support_definition, &support, spec)?;
    let binding_text = render(binding.id().content_id());
    let subject_text = render(invocation.subject().content_id());
    let request = PackRequest {
        checker: "checker/c-op",
        binding: &binding_text,
        subject: &subject_text,
        scope: "operation/local",
        evidence,
    };
    let (expected_result, observations) = match packs::operation::check(&request) {
        PackVerdict::Pass {
            result,
            observations,
        } => (result, observations),
        PackVerdict::Refused(failure) => {
            return Err(OperationLocalAuthorityError::Refused(failure));
        }
        PackVerdict::UnimplementedPack { .. } => {
            return Err(OperationLocalAuthorityError::Unsupported);
        }
    };
    Ok(PreparedOperationLocalCheck {
        binding,
        invocation,
        evidence,
        support_definition,
        expected_result,
        observations,
        grade: EvidenceGrade::Bootstrap,
        provenance,
        policy: operation_local_policy_id()?,
        support,
    })
}

fn support_identity(
    invocation: &CheckInvocation,
    definition: &OperationLocalSupportDefinition,
    spec: &sim_conformance_packs::PackSpec,
) -> Result<EvidenceSetId, OperationLocalAuthorityError> {
    Ok(EvidenceSetId::from_fields(vec![
        (
            Symbol::qualified("conformance", "support-definition"),
            content_id_datum(definition.identity()),
        ),
        (
            Symbol::qualified("conformance", "input-closure"),
            invocation.input_closure().to_datum(),
        ),
        (
            Symbol::qualified("conformance", "adapter-build-inputs-sha256"),
            Datum::String(build_inputs_sha256().into()),
        ),
        (
            Symbol::qualified("conformance", "adapter-dependency-lock-sha256"),
            Datum::String(dependency_lock_sha256().into()),
        ),
        (
            Symbol::qualified("conformance", "checker-build-inputs-sha256"),
            Datum::String(spec.build_inputs_sha256().into()),
        ),
        (
            Symbol::qualified("conformance", "checker-dependency-lock-sha256"),
            Datum::String(spec.dependency_lock_sha256().into()),
        ),
        (
            Symbol::qualified("conformance", "execution"),
            invocation.execution().id().to_datum(),
        ),
    ])?)
}

fn provenance_identity(
    invocation: &CheckInvocation,
    definition: &OperationLocalSupportDefinition,
    support: &EvidenceSetId,
    spec: &sim_conformance_packs::PackSpec,
) -> Result<EvidenceProvenanceId, OperationLocalAuthorityError> {
    Ok(EvidenceProvenanceId::from_fields(vec![
        (
            Symbol::qualified("conformance", "adapter"),
            Datum::String("sim-check-pack/native-operation-local-v1".into()),
        ),
        (
            Symbol::qualified("conformance", "adapter-build-inputs-sha256"),
            Datum::String(build_inputs_sha256().into()),
        ),
        (
            Symbol::qualified("conformance", "adapter-dependency-lock-sha256"),
            Datum::String(dependency_lock_sha256().into()),
        ),
        (
            Symbol::qualified("conformance", "checker-build-inputs-sha256"),
            Datum::String(spec.build_inputs_sha256().into()),
        ),
        (
            Symbol::qualified("conformance", "checker-dependency-lock-sha256"),
            Datum::String(spec.dependency_lock_sha256().into()),
        ),
        (
            Symbol::qualified("conformance", "execution"),
            invocation.execution().id().to_datum(),
        ),
        (
            Symbol::qualified("conformance", "support-definition"),
            content_id_datum(definition.identity()),
        ),
        (
            Symbol::qualified("conformance", "support"),
            support.to_datum(),
        ),
    ])?)
}

fn ensure_compatible_build_graph(
    pack_lock: &str,
    adapter_lock: &str,
) -> Result<(), OperationLocalAuthorityError> {
    if pack_lock == adapter_lock {
        Ok(())
    } else {
        Err(OperationLocalAuthorityError::IncompatibleBuildGraph)
    }
}

fn render(id: &ContentId) -> String {
    let digest = id
        .bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{}:{digest}", id.algorithm.as_qualified_str())
}

#[cfg(test)]
pub(super) fn incompatible_build_graph_for_test() -> Result<(), OperationLocalAuthorityError> {
    ensure_compatible_build_graph(&"a".repeat(64), &"b".repeat(64))
}
