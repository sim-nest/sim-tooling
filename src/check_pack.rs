//! Exact conformance-pack command adapter.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sim_conformance_core::{
    CheckInputClosureId, CheckScopeId, CheckedSubjectId, CheckerReceipt, CheckerRevocationHeadId,
    CheckerRevocationKey, CheckerRevocationSet, ConformanceError, EvidenceGrade,
    EvidenceProvenanceId, EvidenceSetId, RevocationStatus,
};
use sim_conformance_packs::{
    MemorySubject, PackRequest, PackVerdict, find_pack, operation_local_policy_id, packs,
};
use sim_kernel::{ContentId, Datum, Symbol};

const MAX_EVIDENCE_BYTES: u64 = 16_384;
const MAX_FACTS: usize = 256;

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = Options::parse(&args)?;
    let output = refusal(
        &options,
        "owner-currentness-required",
        "check-pack receipt issuance requires an in-process qualified SDK owner revocation set and independently selected current head",
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
/// `currentness` must come from the independently qualified SDK owner boundary;
/// this adapter deliberately has no decoder or constructor for that authority.
/// `expected_head` is independently selected, so an older snapshot fails closed.
/// Results and refusals use the bounded `check/result-v1` JSON projection.
pub fn execute_with_owner_currentness(
    options: &CheckPackOptions,
    bytes: &[u8],
    currentness: &CheckerRevocationSet,
    expected_head: &CheckerRevocationHeadId,
) -> Result<Value, Value> {
    if bytes.len() as u64 > MAX_EVIDENCE_BYTES {
        return Err(refusal(
            options,
            "evidence-bound",
            "standard input exceeds 16384 bytes",
        ));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| refusal(options, "malformed-evidence", "standard input is not UTF-8"))?;
    let evidence =
        parse_evidence(text).map_err(|detail| refusal(options, "malformed-evidence", &detail))?;
    let canonical = canonical_evidence(&evidence);
    let subject = subject_id(&canonical)
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
    let binding = spec
        .checker_binding()
        .map_err(|error| refusal(options, "invalid-static-binding", &error.to_string()))?;
    let expected_binding = render(binding.id().content_id());
    if options.binding != expected_binding {
        return Err(refusal(
            options,
            "wrong-binding",
            &format!("expected {expected_binding}"),
        ));
    }
    let scope = CheckScopeId::from_text(&options.scope)
        .map_err(|error| refusal(options, "scope-identity", &error.to_string()))?;
    let input_closure = CheckInputClosureId::from_fields(vec![(
        Symbol::qualified("conformance", "evidence"),
        Datum::String(canonical.clone()),
    )])
    .map_err(|error| refusal(options, "input-closure-identity", &error.to_string()))?;
    let invocation = binding
        .instantiate(
            spec.checker_code_id()
                .map_err(|error| refusal(options, "checker-code-identity", &error.to_string()))?,
            spec.pack_id()
                .map_err(|error| refusal(options, "pack-identity", &error.to_string()))?,
            subject.clone(),
            scope,
            input_closure,
        )
        .map_err(|error| {
            let code = if error == ConformanceError::UnauthorizedScope {
                "wrong-scope"
            } else {
                "invocation-refused"
            };
            refusal(options, code, &error.to_string())
        })?;
    if options.checker != "checker/c-op" || options.scope != "operation/local" {
        return Err(refusal(
            options,
            "owner-currentness-unsupported",
            "the installed SDK owner currently issues exact currentness only for checker/c-op operation/local",
        ));
    }
    let policy = operation_local_policy_id()
        .map_err(|error| refusal(options, "policy-identity", &error.to_string()))?;
    if currentness.policy() != &policy {
        return Err(refusal(
            options,
            "wrong-currentness-policy",
            "selected owner set does not use the installed operation/local policy",
        ));
    }
    if currentness.head() != expected_head {
        return Err(refusal(
            options,
            "stale-currentness",
            "selected owner set does not match the independently selected current head",
        ));
    }
    let revocation_key =
        CheckerRevocationKey::for_invocation(binding.revocation_source().clone(), &invocation)
            .map_err(|error| refusal(options, "revocation-key", &error.to_string()))?;
    let selection = currentness.lookup(&revocation_key);
    let request = PackRequest {
        checker: &options.checker,
        binding: &options.binding,
        subject: &options.subject,
        scope: &options.scope,
        evidence: &evidence,
    };
    match dispatch(&request) {
        PackVerdict::Pass {
            result,
            observations,
        } => {
            let grade = if options.checker == "checker/c-release" {
                EvidenceGrade::Release
            } else {
                EvidenceGrade::Bootstrap
            };
            let provenance = provenance_id(&invocation)
                .map_err(|error| refusal(options, "provenance-identity", &error.to_string()))?;
            let support = EvidenceSetId::from_fields(vec![(
                Symbol::qualified("conformance", "input-closure"),
                invocation.input_closure().to_datum(),
            )])
            .map_err(|error| refusal(options, "support-identity", &error.to_string()))?;
            let receipt = CheckerReceipt::passing(
                &binding,
                &invocation,
                result.clone(),
                grade,
                provenance,
                policy,
                support,
                &selection,
            )
            .map_err(|error| refusal(options, "receipt-refused", &error.to_string()))?;
            let observation = selection.bind_receipt(receipt.id().clone());
            receipt
                .verify(&binding, &invocation, &observation)
                .map_err(|error| refusal(options, "receipt-verification", &error.to_string()))?;
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
                "result": render(result.content_id()),
                "grade": if grade == EvidenceGrade::Release { "release" } else { "bootstrap" },
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
                "observations": observations.into_iter().map(|value| json!({
                    "key": value.key,
                    "value": value.value,
                })).collect::<Vec<_>>(),
            }))
        }
        PackVerdict::UnimplementedPack {
            checker,
            scope,
            funded_phase,
        } => Err(json!({
            "shape": "check/result-v1",
            "outcome": "unimplemented-pack",
            "checker": checker,
            "binding": options.binding,
            "subject": options.subject,
            "scope": scope,
            "funded_phase": funded_phase,
        })),
        PackVerdict::Refused(failure) => Err(refusal(options, failure.code, &failure.detail)),
    }
}

fn dispatch(request: &PackRequest<'_>) -> PackVerdict {
    match request.checker {
        "checker/c-v3" => packs::retirement::check(request),
        "checker/c-id" => packs::identity::check(request),
        "checker/c-own" => packs::ownership::check(request),
        "checker/c-boundary" => packs::boundary::check(request),
        "checker/c-source" => packs::source::check(request),
        "checker/c-evidence" => packs::evidence::check(request),
        "checker/c-op" => packs::operation::check(request),
        "checker/c-journal" => packs::journal::check(request),
        "checker/c-closure" => packs::closure::check(request),
        "checker/c-control" => packs::control::check(request),
        "checker/c-work" => packs::work::check(request),
        "checker/c-drive" => packs::drive::check(request),
        "checker/c-converge" => packs::convergence::check(request),
        "checker/c-facet" => packs::facet::check(request),
        "checker/c-disclose" => packs::disclosure::check(request),
        "checker/c-deliver" => packs::delivery::check(request),
        "checker/c-author" => packs::authoring::check(request),
        "checker/c-port" => packs::portability::check(request),
        "checker/c-product" => packs::product::check(request),
        "checker/c-release" => packs::release::check(request),
        "checker/c-succeed" => packs::succession::check(request),
        _ => PackVerdict::Refused(sim_conformance_packs::PackFailure {
            code: "unknown-checker",
            detail: request.checker.into(),
        }),
    }
}

fn parse_evidence(text: &str) -> Result<MemorySubject, String> {
    if text.is_empty() {
        return Err("empty evidence".into());
    }
    let mut subject = MemorySubject::default();
    let mut previous: Option<&str> = None;
    let mut count = 0usize;
    for line in text.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("fact {count} has no `=`"))?;
        if key.is_empty()
            || value.is_empty()
            || key.len() > 256
            || value.len() > 4_096
            || key
                .chars()
                .any(|value| value.is_control() || value.is_whitespace())
            || value.chars().any(char::is_control)
        {
            return Err(format!(
                "fact {count} is empty, unbounded, or contains control text"
            ));
        }
        if previous.is_some_and(|seen| seen >= key) {
            return Err(format!("fact keys are duplicate or unsorted at `{key}`"));
        }
        subject = subject.with(key, value);
        previous = Some(key);
        count += 1;
        if count > MAX_FACTS {
            return Err(format!("evidence exceeds {MAX_FACTS} facts"));
        }
    }
    if !text.ends_with('\n') {
        return Err("canonical evidence must end with a newline".into());
    }
    Ok(subject)
}

fn canonical_evidence(subject: &MemorySubject) -> String {
    subject
        .facts()
        .iter()
        .map(|(key, value)| format!("{key}={value}\n"))
        .collect()
}

fn subject_id(
    canonical_evidence: &str,
) -> Result<CheckedSubjectId, sim_conformance_core::ConformanceError> {
    CheckedSubjectId::from_fields(vec![(
        Symbol::qualified("conformance", "evidence"),
        Datum::String(canonical_evidence.into()),
    )])
}

fn provenance_id(
    invocation: &sim_conformance_core::CheckInvocation,
) -> Result<EvidenceProvenanceId, sim_conformance_core::ConformanceError> {
    let adapter_digest = Sha256::digest(include_bytes!("check_pack.rs"));
    EvidenceProvenanceId::from_fields(vec![
        (
            Symbol::qualified("conformance", "adapter"),
            Datum::String("sim-tooling/check-pack".into()),
        ),
        (
            Symbol::qualified("conformance", "adapter-source-sha256"),
            Datum::Bytes(adapter_digest.to_vec()),
        ),
        (
            Symbol::qualified("conformance", "execution"),
            invocation.execution().id().to_datum(),
        ),
    ])
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
    use crate::check_pack_test_support::{head, operation_evidence};
    use sim_conformance_core::{
        CheckerRevocationDecision, PolicyId, ProofCodeId, RevocationSourceId,
    };
    use sim_conformance_packs::{
        OperationLocalRevocationDecision, issue_operation_local_revocation_set,
    };

    // conformance: the host adapter binds canonical evidence to one exact
    // checker, typed runtime binding, subject, and scope and fails closed on
    // substitution or noncanonical input.

    fn options(checker: &str, scope: &str, evidence: &str) -> CheckPackOptions {
        let subject = subject_id(evidence).unwrap();
        CheckPackOptions::new(
            checker,
            render(
                sim_conformance_packs::find_pack(checker)
                    .unwrap()
                    .checker_binding()
                    .unwrap()
                    .id()
                    .content_id(),
            ),
            render(subject.content_id()),
            scope,
        )
    }

    fn owner_set(
        evidence: &str,
        head: &CheckerRevocationHeadId,
        status: RevocationStatus,
    ) -> CheckerRevocationSet {
        let subject = subject_id(evidence).unwrap();
        let decision = match status {
            RevocationStatus::Current => OperationLocalRevocationDecision::current(subject),
            RevocationStatus::Revoked => OperationLocalRevocationDecision::revoked(subject),
            RevocationStatus::Unknown => {
                return issue_operation_local_revocation_set(head.clone(), []).unwrap();
            }
        };
        issue_operation_local_revocation_set(head.clone(), [decision]).unwrap()
    }

    fn execute_operation(
        evidence: &str,
        currentness: &CheckerRevocationSet,
        expected_head: &CheckerRevocationHeadId,
    ) -> Result<Value, Value> {
        execute_with_owner_currentness(
            &options("checker/c-op", "operation/local", evidence),
            evidence.as_bytes(),
            currentness,
            expected_head,
        )
    }

    #[test]
    fn sdk_owner_currentness_yields_receipt_bound_fresh_observation() {
        let evidence = operation_evidence("positive");
        let expected_head = head("sdk/revocations/positive");
        let currentness = owner_set(&evidence, &expected_head, RevocationStatus::Current);
        let output = execute_operation(&evidence, &currentness, &expected_head).unwrap();
        assert_eq!(output["outcome"], "pass");
        assert_eq!(output["grade"], "bootstrap");
        assert!(output["invocation"].as_str().unwrap().contains(':'));
        assert!(output["receipt"].as_str().unwrap().contains(':'));
        assert_eq!(output["revocation"], "current");
        assert_eq!(output["revocation_receipt"], output["receipt"]);
        assert_eq!(
            output["revocation_set"],
            render(currentness.id().content_id())
        );
        assert_eq!(
            output["revocation_head"],
            render(expected_head.content_id())
        );
    }

    #[test]
    fn exact_currentness_yields_deterministic_distinct_receipts() {
        let mut invocations = BTreeSet::new();
        let mut receipts = BTreeSet::new();
        for variant in ["a", "b"] {
            let evidence = operation_evidence(variant);
            let expected_head = head("sdk/revocations/deterministic");
            let currentness = owner_set(&evidence, &expected_head, RevocationStatus::Current);
            let first = execute_operation(&evidence, &currentness, &expected_head).unwrap();
            let second = execute_operation(&evidence, &currentness, &expected_head).unwrap();
            assert_eq!(first, second);
            invocations.insert(first["invocation"].as_str().unwrap().to_owned());
            receipts.insert(first["receipt"].as_str().unwrap().to_owned());
        }
        assert_eq!(invocations.len(), 2);
        assert_eq!(receipts.len(), 2);
    }

    #[test]
    fn missing_revoked_and_stale_currentness_fail_closed() {
        let evidence = operation_evidence("refusals");
        let current_head = head("sdk/revocations/current");
        for status in [RevocationStatus::Unknown, RevocationStatus::Revoked] {
            let set = owner_set(&evidence, &current_head, status);
            assert_eq!(
                execute_operation(&evidence, &set, &current_head).unwrap_err()["reason"],
                "receipt-refused"
            );
        }
        let stale_head = head("sdk/revocations/stale");
        let stale = owner_set(&evidence, &stale_head, RevocationStatus::Current);
        assert_eq!(
            execute_operation(&evidence, &stale, &current_head).unwrap_err()["reason"],
            "stale-currentness"
        );
    }

    #[test]
    fn substituted_subject_policy_source_and_noncanonical_input_fail_closed() {
        let evidence = operation_evidence("original");
        let expected_head = head("sdk/revocations/substitution");
        let currentness = owner_set(&evidence, &expected_head, RevocationStatus::Current);
        let substituted = operation_evidence("substituted");
        assert_eq!(
            execute_operation(&substituted, &currentness, &expected_head).unwrap_err()["reason"],
            "receipt-refused"
        );

        let binding = find_pack("checker/c-op")
            .unwrap()
            .checker_binding()
            .unwrap();
        let wrong_policy = CheckerRevocationSet::from_owner_snapshot(
            binding.owner().clone(),
            binding.revocation_source().clone(),
            PolicyId::from_text("policy/wrong").unwrap(),
            expected_head.clone(),
            vec![],
        )
        .unwrap();
        assert_eq!(
            execute_operation(&evidence, &wrong_policy, &expected_head).unwrap_err()["reason"],
            "wrong-currentness-policy"
        );

        let wrong_source = CheckerRevocationSet::from_owner_snapshot(
            binding.owner().clone(),
            RevocationSourceId::from_text("fixture/wrong-source").unwrap(),
            operation_local_policy_id().unwrap(),
            expected_head.clone(),
            vec![],
        )
        .unwrap();
        assert_eq!(
            execute_operation(&evidence, &wrong_source, &expected_head).unwrap_err()["reason"],
            "receipt-refused"
        );

        let spec = find_pack("checker/c-op").unwrap();
        let substituted_code = ProofCodeId::from_text(&format!(
            "sim-conformance-packs@0.6.1;build-inputs-sha256={}",
            "a".repeat(64)
        ))
        .unwrap();
        assert_ne!(substituted_code, spec.checker_code_id().unwrap());
        let substituted_key = CheckerRevocationKey::new(
            binding.revocation_source().clone(),
            binding.id().clone(),
            subject_id(&evidence).unwrap(),
            substituted_code,
            spec.pack_id().unwrap(),
        )
        .unwrap();
        let substituted_code_set = CheckerRevocationSet::from_owner_snapshot(
            binding.owner().clone(),
            binding.revocation_source().clone(),
            operation_local_policy_id().unwrap(),
            expected_head.clone(),
            vec![
                CheckerRevocationDecision::new(substituted_key, RevocationStatus::Current).unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(
            execute_operation(&evidence, &substituted_code_set, &expected_head).unwrap_err()["reason"],
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
                &currentness,
                &expected_head,
            )
            .unwrap_err()["reason"],
            "subject-mismatch"
        );
        assert!(parse_evidence("b=true\na=true\n").is_err());

        let wrong_scope = options("checker/c-op", "operation/not-bound", &evidence);
        assert_eq!(
            execute_with_owner_currentness(
                &wrong_scope,
                evidence.as_bytes(),
                &currentness,
                &expected_head,
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
                &currentness,
                &expected_head,
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
        let expected_head = head("sdk/revocations/unsupported");
        let currentness = owner_set(&operation, &expected_head, RevocationStatus::Current);
        assert_eq!(
            execute_with_owner_currentness(
                &options("checker/c-id", "identity/vectors", evidence),
                evidence.as_bytes(),
                &currentness,
                &expected_head,
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
