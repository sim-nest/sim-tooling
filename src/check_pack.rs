//! Exact conformance-pack command adapter.

use std::io::Read;

use serde_json::{Value, json};
use sim_conformance_core::CheckedSubjectId;
use sim_conformance_packs::{MemorySubject, PackRequest, PackVerdict, packs};
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
        } => Ok(json!({
            "shape": "check/result-v1",
            "outcome": "pass",
            "checker": options.checker,
            "binding": options.binding,
            "subject": options.subject,
            "scope": options.scope,
            "result": render(result.content_id()),
            "observations": observations.into_iter().map(|value| json!({
                "key": value.key,
                "value": value.value,
            })).collect::<Vec<_>>(),
        })),
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
    use super::*;

    // conformance: the host adapter binds canonical evidence to one exact
    // checker, activation binding, subject, and scope and fails closed on
    // substitution or noncanonical input.

    fn options(checker: &str, scope: &str, evidence: &str) -> Options {
        let subject = subject_id(evidence).unwrap();
        Options {
            checker: checker.into(),
            binding: sim_conformance_packs::find_pack(checker)
                .unwrap()
                .binding
                .into(),
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
        assert_eq!(
            output["observations"][0]["key"],
            "identity.cross-architecture-confirmed"
        );
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
