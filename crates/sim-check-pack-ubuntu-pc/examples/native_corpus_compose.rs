// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Read-only composition of the Ubuntu C-OP evidence from copied native
//! specimen evidence.
//!
//! usage: `native_corpus_compose EVIDENCE_ROOT [--expect-corpus ALGORITHM:HEX]`
//!
//! `EVIDENCE_ROOT` holds one directory per specimen role, each in the fixed
//! layout its class owner reads (`sim_platform_ubuntu_pc::native_evidence`):
//! `completed-success`, `completed-diverged`, `before-cas`,
//! `before-acknowledgement`, `timeout`, `cancellation`, `formatter`,
//! `validation`, `docs`, `refusal`, and `post-gate-<site>` for every native
//! post-gate site (projection-final-image stays a typed pending member).
//! Every specimen is re-verified through its public owner verifier; the
//! definitions are frozen from those verified specimens, the four-role corpus
//! is built with its provider-bound before-acknowledgement, every admission
//! runs, and the 22 facts are composed. Nothing executes and no authority is
//! produced. `--expect-corpus` pins a reviewed corpus identity, so a later
//! retrospective re-derives the evidence instead of trusting printed output.

use sim_check_pack_ubuntu_pc::compose_operation_local;
use sim_kernel::ContentId;
use sim_platform_ubuntu_pc::{
    NATIVE_POST_GATE_SITES, NativeAcceptanceClass, OperationLocalCorpusDefinition,
    OperationLocalFormatterDefinition, OperationLocalOwnerCommandDefinition,
    OperationLocalPostGateDefinition, OperationLocalRefusalDefinition,
    OperationLocalSpecimenExpectation, OperationLocalStopDefinition, VerifiedNativeAcceptance,
    verify_native_specimen_directory, verify_operation_local_corpus_with_provider_bound,
    verify_provider_bound_specimen_directory,
};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "usage: native_corpus_compose EVIDENCE_ROOT [--expect-corpus ALGORITHM:HEX]";

fn content_text(id: &ContentId) -> String {
    format!(
        "{}:{}",
        id.algorithm,
        id.bytes.map(|byte| format!("{byte:02x}")).concat()
    )
}

/// The parsed invocation: an absolute evidence root and an optional pin.
#[derive(Debug, Eq, PartialEq)]
struct Invocation {
    root: PathBuf,
    expect_corpus: Option<String>,
}

fn parse(arguments: &[String]) -> Result<Invocation> {
    let (root, expect_corpus) = match arguments {
        [root] => (root, None),
        [root, flag, expected] if flag == "--expect-corpus" => (root, Some(expected.clone())),
        _ => return Err(USAGE.into()),
    };
    let root = PathBuf::from(root);
    if !root.is_absolute() {
        return Err("evidence root must be absolute".into());
    }
    if expect_corpus
        .as_deref()
        .is_some_and(|expected| !expected.contains(':') || expected.ends_with(':'))
    {
        return Err("expected corpus must be ALGORITHM:HEX".into());
    }
    Ok(Invocation {
        root,
        expect_corpus,
    })
}

fn specimen(
    root: &Path,
    role: &str,
    class: NativeAcceptanceClass,
) -> Result<VerifiedNativeAcceptance> {
    verify_native_specimen_directory(&root.join(role), class)
        .map_err(|error| format!("specimen {role}: {error}").into())
}

fn run(invocation: &Invocation) -> Result<()> {
    let root = invocation.root.as_path();
    let success = specimen(
        root,
        "completed-success",
        NativeAcceptanceClass::CompletedSuccess,
    )?;
    let diverged = specimen(
        root,
        "completed-diverged",
        NativeAcceptanceClass::CompletedDiverged,
    )?;
    let before_cas = specimen(
        root,
        "before-cas",
        NativeAcceptanceClass::IncompleteBeforeFinalBeforeCas,
    )?;
    let before_acknowledgement =
        verify_provider_bound_specimen_directory(&root.join("before-acknowledgement"))
            .map_err(|error| format!("specimen before-acknowledgement: {error}"))?;
    let definition = OperationLocalCorpusDefinition::new([
        OperationLocalSpecimenExpectation::from_verified(&success)?,
        OperationLocalSpecimenExpectation::from_verified(&diverged)?,
        OperationLocalSpecimenExpectation::from_verified(&before_cas)?,
        OperationLocalSpecimenExpectation::from_provider_bound(&before_acknowledgement)?,
    ])?;
    let mut corpus = verify_operation_local_corpus_with_provider_bound(
        &definition,
        [success, diverged, before_cas],
        before_acknowledgement,
    )?;

    let timeout = specimen(root, "timeout", NativeAcceptanceClass::StoppedAfterTimeout)?;
    let cancellation = specimen(
        root,
        "cancellation",
        NativeAcceptanceClass::StoppedAfterCancellation,
    )?;
    let stops = OperationLocalStopDefinition::from_verified([&timeout, &cancellation])?;
    corpus.admit_stop_specimens(&stops, [timeout, cancellation])?;

    let formatter = specimen(root, "formatter", NativeAcceptanceClass::CompletedSuccess)?;
    let formatter_definition = OperationLocalFormatterDefinition::from_verified(&formatter)?;
    corpus.admit_formatter_specimen(&formatter_definition, formatter)?;

    let validation = specimen(root, "validation", NativeAcceptanceClass::CompletedSuccess)?;
    let docs = specimen(root, "docs", NativeAcceptanceClass::CompletedSuccess)?;
    let owners = OperationLocalOwnerCommandDefinition::from_verified(&validation, &docs)?;
    corpus.admit_owner_command_specimens(&owners, validation, docs)?;

    let refusal = specimen(root, "refusal", NativeAcceptanceClass::RefusedBeforeRelease)?;
    let refusal_definition = OperationLocalRefusalDefinition::from_verified(&refusal)?;
    corpus.admit_refusal_specimen(&refusal_definition, refusal)?;

    // Only the native sites; projection-final-image stays a typed pending
    // member completed by the projection-boundary packet (G5).
    let post_gate = NATIVE_POST_GATE_SITES
        .iter()
        .map(|site| {
            specimen(
                root,
                &format!("post-gate-{site}"),
                NativeAcceptanceClass::ReleasedErrorDisposed,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let post_gate_definition =
        OperationLocalPostGateDefinition::from_verified(&post_gate.iter().collect::<Vec<_>>())?;
    corpus.admit_post_gate_specimens(&post_gate_definition, post_gate)?;

    let corpus_identity = content_text(corpus.identity());
    if let Some(expected) = &invocation.expect_corpus
        && &corpus_identity != expected
    {
        return Err(format!("corpus {corpus_identity} differs from expected {expected}").into());
    }
    let witness = corpus
        .post_gate_witness()
        .ok_or("post-gate witness absent after admission")?;
    let pending = witness
        .pending()
        .iter()
        .map(|member| member.site)
        .collect::<Vec<_>>()
        .join(",");
    let inputs = compose_operation_local(&corpus)?;
    println!(
        "SIM_NATIVE_CORPUS schema=v2 corpus={corpus_identity} definition={} post-gate={} post-gate-pending={pending} support={} subject={} input-closure={} native-authority=false m5-qualified=false",
        content_text(corpus.definition_identity()),
        content_text(witness.identity()),
        content_text(inputs.support_definition().identity()),
        content_text(inputs.evidence().subject_id()?.content_id()),
        content_text(inputs.evidence().input_closure_id()?.content_id()),
    );
    println!("{}", inputs.evidence().canonical());
    Ok(())
}

fn main() -> Result<()> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    run(&parse(&arguments)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn invocations_are_exactly_a_root_and_an_optional_pin() {
        assert_eq!(
            parse(&arguments(&["/evidence"])).unwrap(),
            Invocation {
                root: PathBuf::from("/evidence"),
                expect_corpus: None
            }
        );
        assert_eq!(
            parse(&arguments(&[
                "/evidence",
                "--expect-corpus",
                "core/sha256:ab"
            ]))
            .unwrap()
            .expect_corpus
            .as_deref(),
            Some("core/sha256:ab")
        );
        for refused in [
            &[][..],
            &["relative"][..],
            &["/evidence", "--expect-corpus"][..],
            &["/evidence", "--other", "x:y"][..],
            &["/evidence", "--expect-corpus", "nocolon"][..],
            &["/evidence", "--expect-corpus", "x:"][..],
            &["/evidence", "extra", "more", "args"][..],
        ] {
            assert!(parse(&arguments(refused)).is_err(), "{refused:?}");
        }
    }

    #[test]
    fn a_missing_specimen_is_named_and_nothing_is_composed() {
        let root = std::env::temp_dir().join(format!(
            "native-corpus-compose-missing-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let error = run(&Invocation {
            root: root.clone(),
            expect_corpus: None,
        })
        .unwrap_err()
        .to_string();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(error.contains("specimen completed-success"), "{error}");
    }
}
