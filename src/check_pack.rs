//! Exact conformance-pack command adapter.

use serde_json::{Value, json};
use sim_conformance_core::{EvidenceGrade, RevocationStatus};
use sim_conformance_packs::{
    OperationLocalAuthority, OperationLocalEvidence, OperationLocalSupportDefinition,
    QualifiedOperationLocalCheck, find_pack,
};
use sim_kernel::ContentId;

const MAX_EVIDENCE_BYTES: u64 = 16_384;

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = Options::parse(&args)?;
    let output = refusal(
        &options,
        "owner-currentness-required",
        "check-pack receipt issuance requires an in-process handle from the live SDK owner",
    );
    Err(output.to_string())
}

/// Exact identities supplied to one conformance-pack invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckPackOptions {
    checker: String,
    binding: String,
    subject: String,
    scope: String,
}

impl CheckPackOptions {
    /// Constructs a request without treating any string as currentness authority.
    pub fn new(
        checker: impl Into<String>,
        binding: impl Into<String>,
        subject: impl Into<String>,
        scope: impl Into<String>,
    ) -> Self {
        Self {
            checker: checker.into(),
            binding: binding.into(),
            subject: subject.into(),
            scope: scope.into(),
        }
    }

    fn parse(args: &[String]) -> Result<Self, String> {
        if args.get(1).map(String::as_str) != Some("check-pack") {
            return Err(usage(args.first().map_or("xtask", String::as_str)));
        }
        let mut values = std::collections::BTreeMap::new();
        let mut index = 2;
        while index < args.len() {
            let flag = args[index].as_str();
            if !["--checker", "--binding", "--subject", "--scope"].contains(&flag) {
                return Err(format!(
                    "unknown check-pack argument `{flag}`; {}",
                    usage(&args[0])
                ));
            }
            let value = args
                .get(index + 1)
                .ok_or_else(|| format!("missing value for `{flag}`; {}", usage(&args[0])))?;
            if values.insert(flag, value.clone()).is_some() {
                return Err(format!("duplicate check-pack argument `{flag}`"));
            }
            index += 2;
        }
        let take = |flag| {
            values
                .get(flag)
                .cloned()
                .ok_or_else(|| format!("missing `{flag}`; {}", usage(&args[0])))
        };
        Ok(Self {
            checker: take("--checker")?,
            binding: take("--binding")?,
            subject: take("--subject")?,
            scope: take("--scope")?,
        })
    }
}

type Options = CheckPackOptions;

/// Runs one exact pack and binds its passing result to owner currentness.
///
/// `authority` must be the weak handle retained from the installed SDK owner;
/// this adapter deliberately has no decoder or constructor for that authority.
/// Results and refusals use the bounded `check/result-v1` JSON projection.
pub fn execute_with_owner_currentness(
    options: &CheckPackOptions,
    bytes: &[u8],
    support_definition: &OperationLocalSupportDefinition,
    authority: &OperationLocalAuthority,
) -> Result<Value, Value> {
    if bytes.len() as u64 > MAX_EVIDENCE_BYTES {
        return Err(refusal(
            options,
            "evidence-bound",
            "standard input exceeds 16384 bytes",
        ));
    }
    let evidence = OperationLocalEvidence::parse(bytes)
        .map_err(|error| refusal(options, "malformed-evidence", &error.to_string()))?;
    let subject = evidence
        .subject_id()
        .map_err(|error| refusal(options, "subject-identity", &error.to_string()))?;
    let computed = render(subject.content_id());
    if options.subject != computed {
        return Err(refusal(
            options,
            "subject-mismatch",
            &format!("expected {computed}"),
        ));
    }
    let spec = find_pack(&options.checker).ok_or_else(|| {
        refusal(
            options,
            "unknown-checker",
            &format!("no static binding for {}", options.checker),
        )
    })?;
    if options.checker != "checker/c-op" {
        return Err(refusal(
            options,
            "owner-currentness-unsupported",
            "the installed SDK owner currently issues exact currentness only for checker/c-op operation/local",
        ));
    }
    if options.scope != "operation/local" {
        return Err(refusal(
            options,
            "wrong-scope",
            "the installed SDK owner permits only operation/local",
        ));
    }
    let binding = spec
        .native_operation_local_binding()
        .map_err(|error| refusal(options, "invalid-static-binding", &error.to_string()))?;
    let expected_binding = render(binding.id().content_id());
    if options.binding != expected_binding {
        return Err(refusal(
            options,
            "wrong-binding",
            &format!("expected {expected_binding}"),
        ));
    }
    let qualified =
        sim_check_pack::invoke_operation_local(&evidence, support_definition, authority)
            .map_err(|error| refusal(options, "receipt-refused", &error.to_string()))?;
    project_qualified(options, authority, &qualified)
}

fn project_qualified(
    options: &CheckPackOptions,
    authority: &OperationLocalAuthority,
    qualified: &QualifiedOperationLocalCheck,
) -> Result<Value, Value> {
    let observation = qualified
        .verify_current(authority)
        .map_err(|error| refusal(options, "receipt-verification", &error.to_string()))?;
    let receipt = qualified.receipt();
    let invocation = qualified.invocation();
    let spec = find_pack(&options.checker).ok_or_else(|| {
        refusal(
            options,
            "unknown-checker",
            &format!("no static binding for {}", options.checker),
        )
    })?;
    Ok(json!({
        "shape": "check/result-v1",
        "outcome": "pass",
        "checker": options.checker,
        "activation_binding": spec.activation_binding,
        "binding": options.binding,
        "checker_code": render(invocation.checker_code().content_id()),
        "pack": render(invocation.pack().content_id()),
        "subject": options.subject,
        "scope": options.scope,
        "scope_id": render(invocation.scope().content_id()),
        "execution": render(invocation.execution().id().content_id()),
        "input_closure": render(invocation.input_closure().content_id()),
        "invocation": render(invocation.id().content_id()),
        "result": render(receipt.result().content_id()),
        "grade": if receipt.grade() == EvidenceGrade::Release { "release" } else { "bootstrap" },
        "provenance": render(receipt.provenance().content_id()),
        "policy": render(receipt.policy().content_id()),
        "support": render(receipt.support().content_id()),
        "receipt": render(receipt.id().content_id()),
        "revocation": match observation.status() {
            RevocationStatus::Unknown => "unknown",
            RevocationStatus::Current => "current",
            RevocationStatus::Revoked => "revoked",
        },
        "revocation_issuer": render(observation.issuer().content_id()),
        "revocation_source": render(observation.source().content_id()),
        "revocation_key": render(observation.key().id().content_id()),
        "revocation_set": render(observation.set().content_id()),
        "revocation_head": render(observation.head().content_id()),
        "revocation_receipt": render(observation.receipt().content_id()),
        "observations": qualified.observations().iter().map(|value| json!({
            "key": value.key,
            "value": value.value,
        })).collect::<Vec<_>>(),
    }))
}

fn refusal(options: &Options, code: &str, detail: &str) -> Value {
    json!({
        "shape": "check/result-v1",
        "outcome": "refused",
        "checker": options.checker,
        "binding": options.binding,
        "subject": options.subject,
        "scope": options.scope,
        "reason": code,
        "detail": detail.chars().take(4096).collect::<String>(),
    })
}

fn render(id: &ContentId) -> String {
    let digest = id
        .bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{}:{digest}", id.algorithm.as_qualified_str())
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} check-pack --checker <CheckerId> --binding <CheckerBindingId> --subject <CheckedSubjectId> --scope <CheckScopeId>"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::check_pack_test_support::operation_evidence;
    use sim_conformance_packs::OperationLocalOwner;
    use sim_kernel::{Datum, Symbol};

    // conformance: the host adapter binds canonical evidence to one exact
    // checker, typed runtime binding, subject, and scope and fails closed on
    // substitution or noncanonical input.

    fn subject_id(evidence: &str) -> Result<sim_conformance_core::CheckedSubjectId, String> {
        OperationLocalEvidence::parse(evidence.as_bytes())
            .map_err(|error| error.to_string())?
            .subject_id()
            .map_err(|error| error.to_string())
    }

    fn options(checker: &str, scope: &str, evidence: &str) -> CheckPackOptions {
        let subject = subject_id(evidence).unwrap();
        let spec = sim_conformance_packs::find_pack(checker).unwrap();
        let binding = if checker == "checker/c-op" && scope == "operation/local" {
            spec.native_operation_local_binding().unwrap()
        } else {
            spec.checker_binding().unwrap()
        };
        CheckPackOptions::new(
            checker,
            render(binding.id().content_id()),
            render(subject.content_id()),
            scope,
        )
    }

    fn owner(byte: u8) -> OperationLocalOwner {
        OperationLocalOwner::boot(ContentId::from_bytes(
            Symbol::qualified("conformance", "tooling-test-generation"),
            [byte; 32],
        ))
    }

    fn support(evidence: &str) -> OperationLocalSupportDefinition {
        OperationLocalSupportDefinition::new(evidence.lines().map(|line| {
            let (name, _) = line.split_once('=').unwrap();
            (
                name.to_owned(),
                Datum::String(format!("support:{name}"))
                    .content_id()
                    .unwrap(),
            )
        }))
        .unwrap()
    }

    fn execute_operation(
        evidence: &str,
        authority: &OperationLocalAuthority,
    ) -> Result<Value, Value> {
        execute_with_owner_currentness(
            &options("checker/c-op", "operation/local", evidence),
            evidence.as_bytes(),
            &support(evidence),
            authority,
        )
    }

    #[test]
    fn sdk_owner_currentness_yields_receipt_bound_fresh_observation() {
        let evidence = operation_evidence("positive");
        let owner = owner(1);
        owner.mark_current(subject_id(&evidence).unwrap()).unwrap();
        let output = execute_operation(&evidence, &owner.operation_handle()).unwrap();
        assert_eq!(output["outcome"], "pass");
        assert_eq!(output["grade"], "bootstrap");
        assert!(output["invocation"].as_str().unwrap().contains(':'));
        assert!(output["receipt"].as_str().unwrap().contains(':'));
        assert_eq!(output["revocation"], "current");
        assert_eq!(output["revocation_receipt"], output["receipt"]);
        assert!(output["revocation_set"].as_str().unwrap().contains(':'));
        assert!(output["revocation_head"].as_str().unwrap().contains(':'));
    }

    #[test]
    fn exact_currentness_yields_deterministic_distinct_receipts() {
        let mut invocations = BTreeSet::new();
        let mut receipts = BTreeSet::new();
        for (byte, variant) in [(2, "a"), (3, "b")] {
            let evidence = operation_evidence(variant);
            let owner = owner(byte);
            owner.mark_current(subject_id(&evidence).unwrap()).unwrap();
            let authority = owner.operation_handle();
            let first = execute_operation(&evidence, &authority).unwrap();
            let second = execute_operation(&evidence, &authority).unwrap();
            assert_eq!(first, second);
            invocations.insert(first["invocation"].as_str().unwrap().to_owned());
            receipts.insert(first["receipt"].as_str().unwrap().to_owned());
        }
        assert_eq!(invocations.len(), 2);
        assert_eq!(receipts.len(), 2);
    }

    #[test]
    fn missing_revoked_and_dead_owner_currentness_fail_closed() {
        let evidence = operation_evidence("refusals");
        let missing = owner(4);
        assert_eq!(
            execute_operation(&evidence, &missing.operation_handle()).unwrap_err()["reason"],
            "receipt-refused"
        );

        let revoked = owner(5);
        revoked.revoke(subject_id(&evidence).unwrap()).unwrap();
        assert_eq!(
            execute_operation(&evidence, &revoked.operation_handle()).unwrap_err()["reason"],
            "receipt-refused"
        );

        let dead = owner(6);
        dead.mark_current(subject_id(&evidence).unwrap()).unwrap();
        let authority = dead.operation_handle();
        drop(dead);
        assert_eq!(
            execute_operation(&evidence, &authority).unwrap_err()["reason"],
            "receipt-refused"
        );
    }

    #[test]
    fn substituted_subject_scope_binding_and_noncanonical_input_fail_closed() {
        let evidence = operation_evidence("original");
        let owner = owner(7);
        owner.mark_current(subject_id(&evidence).unwrap()).unwrap();
        let authority = owner.operation_handle();
        let substituted = operation_evidence("substituted");
        assert_eq!(
            execute_operation(&substituted, &authority).unwrap_err()["reason"],
            "receipt-refused"
        );

        let mut wrong = options("checker/c-op", "operation/local", &evidence);
        wrong.subject =
            "core/sha256-datum-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .into();
        assert_eq!(
            execute_with_owner_currentness(
                &wrong,
                evidence.as_bytes(),
                &support(&evidence),
                &authority,
            )
            .unwrap_err()["reason"],
            "subject-mismatch"
        );
        assert_eq!(
            execute_with_owner_currentness(
                &options("checker/c-op", "operation/local", &evidence),
                b"b=true\na=true\n",
                &support(&evidence),
                &authority,
            )
            .unwrap_err()["reason"],
            "malformed-evidence"
        );

        let wrong_scope = options("checker/c-op", "operation/not-bound", &evidence);
        assert_eq!(
            execute_with_owner_currentness(
                &wrong_scope,
                evidence.as_bytes(),
                &support(&evidence),
                &authority,
            )
            .unwrap_err()["reason"],
            "wrong-scope"
        );

        let mut wrong_binding = options("checker/c-op", "operation/local", &evidence);
        wrong_binding.binding =
            "core/sha256-datum-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .into();
        assert_eq!(
            execute_with_owner_currentness(
                &wrong_binding,
                evidence.as_bytes(),
                &support(&evidence),
                &authority,
            )
            .unwrap_err()["reason"],
            "wrong-binding"
        );
    }

    #[test]
    fn unsupported_pack_and_cli_without_owner_object_refuse() {
        let evidence =
            "identity.cross-architecture-confirmed=true\nidentity.cross-toolchain-confirmed=true\n";
        let operation = operation_evidence("unsupported");
        let owner = owner(8);
        owner.mark_current(subject_id(&operation).unwrap()).unwrap();
        let authority = owner.operation_handle();
        assert_eq!(
            execute_with_owner_currentness(
                &options("checker/c-id", "identity/vectors", evidence),
                evidence.as_bytes(),
                &support(evidence),
                &authority,
            )
            .unwrap_err()["reason"],
            "owner-currentness-unsupported"
        );

        let command = vec![
            "xtask".into(),
            "check-pack".into(),
            "--checker".into(),
            "checker/c-op".into(),
            "--binding".into(),
            "binding".into(),
            "--subject".into(),
            "subject".into(),
            "--scope".into(),
            "operation/local".into(),
        ];
        assert!(
            run(command)
                .unwrap_err()
                .contains("owner-currentness-required")
        );
    }
}
