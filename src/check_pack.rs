//! Exact conformance-pack command adapter.

use std::io::Read;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sim_conformance_core::{
    CheckInputClosureId, CheckScopeId, CheckedSubjectId, CheckerReceipt, ConformanceError,
    EvidenceGrade, EvidenceProvenanceId, EvidenceSetId, PolicyId, RevocationStatus,
};
use sim_conformance_packs::{MemorySubject, PackRequest, PackVerdict, find_pack, packs};
use sim_kernel::{ContentId, Datum, Symbol};

const MAX_EVIDENCE_BYTES: u64 = 16_384;
const MAX_FACTS: usize = 256;

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = Options::parse(&args)?;
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(MAX_EVIDENCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("check-pack could not read evidence: {error}"))?;
    let output = execute(&options, &bytes).map_err(|failure| failure.to_string())?;
    println!(
        "{}",
        serde_json::to_string(&output)
            .map_err(|error| format!("check-pack could not encode result: {error}"))?
    );
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Options {
    checker: String,
    binding: String,
    subject: String,
    scope: String,
}

impl Options {
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

fn execute(options: &Options, bytes: &[u8]) -> Result<Value, Value> {
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
            let policy = PolicyId::from_text("sim-conformance-packs/revocation-current-v1")
                .map_err(|error| refusal(options, "policy-identity", &error.to_string()))?;
            let receipt = CheckerReceipt::passing(
                &invocation,
                result.clone(),
                grade,
                provenance,
                policy,
                support,
                RevocationStatus::Current,
            )
            .map_err(|error| refusal(options, "receipt-refused", &error.to_string()))?;
            receipt
                .verify(&binding, &invocation, RevocationStatus::Current)
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
                "revocation": "current",
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

    // conformance: the host adapter binds canonical evidence to one exact
    // checker, typed runtime binding, subject, and scope and fails closed on
    // substitution or noncanonical input.

    fn options(checker: &str, scope: &str, evidence: &str) -> Options {
        let subject = subject_id(evidence).unwrap();
        Options {
            checker: checker.into(),
            binding: render(
                sim_conformance_packs::find_pack(checker)
                    .unwrap()
                    .checker_binding()
                    .unwrap()
                    .id()
                    .content_id(),
            ),
            subject: render(subject.content_id()),
            scope: scope.into(),
        }
    }

    #[test]
    fn canonical_identity_vector_subject_passes() {
        let evidence =
            "identity.cross-architecture-confirmed=true\nidentity.cross-toolchain-confirmed=true\n";
        let output = execute(
            &options("checker/c-id", "identity/vectors", evidence),
            evidence.as_bytes(),
        )
        .unwrap();
        assert_eq!(output["outcome"], "pass");
        assert_eq!(output["grade"], "bootstrap");
        assert!(output["invocation"].as_str().unwrap().contains(':'));
        assert!(output["receipt"].as_str().unwrap().contains(':'));
        assert_eq!(
            output["observations"][0]["key"],
            "identity.cross-architecture-confirmed"
        );
    }

    #[test]
    fn production_binding_yields_four_exact_receipts_deterministically() {
        let facts = |variant: &str| {
            format!(
                "identity.all-constructions-funded=true\nidentity.cross-architecture-confirmed=true\nidentity.cross-toolchain-confirmed=true\nidentity.domain-tags-unique=true\nidentity.ephemeral-authority-absent=true\nidentity.expected-constructions=31\nidentity.registered-constructions=31\nidentity.semantic-digests-256-bit=true\nidentity.semantic-storage-separated=true\nsubject.variant={variant}\n"
            )
        };
        let mut invocations = BTreeSet::new();
        let mut receipts = BTreeSet::new();
        for variant in ["a", "b"] {
            let evidence = facts(variant);
            for scope in ["identity/register", "identity/vectors"] {
                let options = options("checker/c-id", scope, &evidence);
                let first = execute(&options, evidence.as_bytes()).unwrap();
                let second = execute(&options, evidence.as_bytes()).unwrap();
                assert_eq!(first, second);
                invocations.insert(first["invocation"].as_str().unwrap().to_owned());
                receipts.insert(first["receipt"].as_str().unwrap().to_owned());
            }
        }
        assert_eq!(invocations.len(), 4);
        assert_eq!(receipts.len(), 4);
    }

    #[test]
    fn release_pack_issues_release_grade_only_after_every_fact_passes() {
        let evidence = "release.audit-passed=true\nrelease.authorship-passed=true\nrelease.boot-smoke-passed=true\nrelease.generated-converged=true\nrelease.mirrors-current=true\nrelease.owner-docs-passed=true\nrelease.owner-validation-passed=true\nrelease.packages-assembled=true\nrelease.pins-exact=true\nrelease.publication-confirmed=true\nrelease.standalone-ci-green=true\nrelease.tags-exact=true\n";
        let output = execute(
            &options("checker/c-release", "release/nv12-01", evidence),
            evidence.as_bytes(),
        )
        .unwrap();
        assert_eq!(output["grade"], "release");
        assert_eq!(output["revocation"], "current");
    }

    #[test]
    fn substituted_subject_and_unsorted_facts_fail_closed() {
        let evidence = "a=true\nb=true\n";
        let mut wrong = options("checker/c-id", "identity/vectors", evidence);
        wrong.subject =
            "core/sha256-datum-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .into();
        assert_eq!(
            execute(&wrong, evidence.as_bytes()).unwrap_err()["reason"],
            "subject-mismatch"
        );
        assert!(parse_evidence("b=true\na=true\n").is_err());

        let wrong_scope = options("checker/c-id", "identity/not-bound", evidence);
        assert_eq!(
            execute(&wrong_scope, evidence.as_bytes()).unwrap_err()["reason"],
            "wrong-scope"
        );

        let mut wrong_binding = options("checker/c-id", "identity/vectors", evidence);
        wrong_binding.binding =
            "core/sha256-datum-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .into();
        assert_eq!(
            execute(&wrong_binding, evidence.as_bytes()).unwrap_err()["reason"],
            "wrong-binding"
        );
    }
}
