// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Read-only composition of the Ubuntu C-OP evidence from copied native
//! specimen evidence.
//!
//! usage: `native_corpus_compose EVIDENCE_ROOT --expect-sim-sha256 HEX
//! --expect-runtime-rid HEX16 [--expect-corpus ALGORITHM:HEX]`
//!
//! `--expect-sim-sha256` and `--expect-runtime-rid` are the sealed MANIFEST's
//! `sim` digest and the runtime identifier derived from its four sealed runtime
//! member digests. Every specimen's recorded `entry-runtime` must be the pinned
//! runtime with exactly that `RID` and that `sim` digest; a specimen recorded as
//! plain, or with another runtime or image, refuses the composition. Both flags
//! are required: a batch of the pinned `sim` has no plain role.
//!
//! `EVIDENCE_ROOT` holds one directory per specimen role, each in the fixed
//! layout its class owner reads (`sim_platform_ubuntu_pc::native_evidence`):
//! `completed-success`, `completed-diverged`, `before-cas`,
//! `before-acknowledgement`, `timeout`, `cancellation`, `formatter`,
//! `validation`, `docs`, `refusal`, and `post-gate-<site>` for every native
//! post-gate site (projection-final-image stays a typed pending member).
//! Every specimen is re-verified through its public owner verifier; the
//! definitions are frozen from those verified specimens, the four-role corpus
//! is built with its recorded provider-bound before-acknowledgement
//! attestation, every admission runs, and the 22 facts are composed. Nothing
//! executes and no authority is produced.
//!
//! The before-acknowledgement specimen is a recorded attestation, not live
//! evidence. That PID 1 retained each observer's successful terminal and that
//! both observer installations were current around the reads is attested
//! once, live, by the product collector, whose standard output is the
//! specimen's `live-observation` record. This composition re-derives every
//! content fact from ordinary copies (payloads, stop-time formatter
//! terminals, installation configurations, provider lifecycle, target cut,
//! plan and job) and checks that they are consistent with that attestation;
//! it trusts the record exactly as it trusts every other root-held copy.
//! `--expect-corpus` pins a reviewed corpus identity, so a later
//! retrospective, after teardown or a reboot, re-derives the evidence instead
//! of trusting printed output.

use sim_check_pack_ubuntu_pc::compose_operation_local;
use sim_kernel::ContentId;
use sim_platform_ubuntu_pc::{
    ExpectedBatchRuntime, NATIVE_POST_GATE_SITES, NativeAcceptanceClass,
    OperationLocalCorpusDefinition,
    OperationLocalFormatterDefinition, OperationLocalOwnerCommandDefinition,
    OperationLocalPostGateDefinition, OperationLocalRefusalDefinition,
    OperationLocalSpecimenExpectation, OperationLocalStopDefinition, VerifiedNativeAcceptance,
    RecordedEntryRuntime, RecordedProviderBoundAttestation, recorded_entry_runtime,
    verify_native_specimen_directory,
    verify_operation_local_corpus_with_recorded_provider_bound,
    verify_recorded_provider_bound_specimen_directory,
};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const USAGE: &str = "usage: native_corpus_compose EVIDENCE_ROOT --expect-sim-sha256 HEX --expect-runtime-rid HEX16 [--expect-corpus ALGORITHM:HEX]";

fn content_text(id: &ContentId) -> String {
    format!(
        "{}:{}",
        id.algorithm,
        id.bytes.map(|byte| format!("{byte:02x}")).concat()
    )
}

/// The parsed invocation: an absolute evidence root, the batch's expected
/// runtime identity, and an optional corpus pin.
#[derive(Debug, Eq, PartialEq)]
struct Invocation {
    root: PathBuf,
    runtime: ExpectedBatchRuntime,
    expect_corpus: Option<String>,
}

fn parse(arguments: &[String]) -> Result<Invocation> {
    let [root, flags @ ..] = arguments else {
        return Err(USAGE.into());
    };
    let (mut sim, mut rid, mut expect_corpus) = (None, None, None);
    let mut rest = flags.iter();
    while let Some(flag) = rest.next() {
        let slot = match flag.as_str() {
            "--expect-sim-sha256" => &mut sim,
            "--expect-runtime-rid" => &mut rid,
            "--expect-corpus" => &mut expect_corpus,
            _ => return Err(USAGE.into()),
        };
        match (rest.next(), slot.is_some()) {
            (Some(value), false) => *slot = Some(value.clone()),
            _ => return Err(USAGE.into()),
        }
    }
    let (Some(sim), Some(rid)) = (sim, rid) else {
        return Err(USAGE.into());
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
        runtime: ExpectedBatchRuntime::parse(&sim, &rid)?,
        expect_corpus,
    })
}

fn specimen(
    root: &Path,
    role: &str,
    class: NativeAcceptanceClass,
    runtime: &ExpectedBatchRuntime,
) -> Result<VerifiedNativeAcceptance> {
    bound(
        role,
        verify_native_specimen_directory(&root.join(role), class),
        runtime,
        recorded_entry_runtime,
    )
}

/// Binds one verified role to the batch's runtime and sim image: a role
/// recorded as plain, or as another runtime or image, refuses composition. The
/// one path every role takes, so no role is composed unbound.
fn bound<T>(
    role: &str,
    verified: std::io::Result<T>,
    runtime: &ExpectedBatchRuntime,
    recorded: impl FnOnce(&T) -> std::io::Result<RecordedEntryRuntime>,
) -> Result<T> {
    let verified = verified.map_err(|error| format!("specimen {role}: {error}"))?;
    recorded(&verified)
        .and_then(|entry| runtime.require(&entry))
        .map_err(|error| format!("specimen {role}: {error}"))?;
    Ok(verified)
}

fn run(invocation: &Invocation) -> Result<()> {
    let root = invocation.root.as_path();
    let runtime = &invocation.runtime;
    let success = specimen(
        root,
        "completed-success",
        NativeAcceptanceClass::CompletedSuccess,
        runtime,
    )?;
    let diverged = specimen(
        root,
        "completed-diverged",
        NativeAcceptanceClass::CompletedDiverged,
        runtime,
    )?;
    let before_cas = specimen(
        root,
        "before-cas",
        NativeAcceptanceClass::IncompleteBeforeFinalBeforeCas,
        runtime,
    )?;
    let before_acknowledgement = bound(
        "before-acknowledgement",
        verify_recorded_provider_bound_specimen_directory(&root.join("before-acknowledgement")),
        runtime,
        RecordedProviderBoundAttestation::entry_runtime,
    )?;
    let definition = OperationLocalCorpusDefinition::new([
        OperationLocalSpecimenExpectation::from_verified(&success)?,
        OperationLocalSpecimenExpectation::from_verified(&diverged)?,
        OperationLocalSpecimenExpectation::from_verified(&before_cas)?,
        OperationLocalSpecimenExpectation::from_recorded_provider_bound(&before_acknowledgement)?,
    ])?;
    let mut corpus = verify_operation_local_corpus_with_recorded_provider_bound(
        &definition,
        [success, diverged, before_cas],
        before_acknowledgement,
    )?;

    let timeout = specimen(
        root,
        "timeout",
        NativeAcceptanceClass::StoppedAfterTimeout,
        runtime,
    )?;
    let cancellation = specimen(
        root,
        "cancellation",
        NativeAcceptanceClass::StoppedAfterCancellation,
        runtime,
    )?;
    let stops = OperationLocalStopDefinition::from_verified([&timeout, &cancellation])?;
    corpus.admit_stop_specimens(&stops, [timeout, cancellation])?;

    let formatter = specimen(
        root,
        "formatter",
        NativeAcceptanceClass::CompletedSuccess,
        runtime,
    )?;
    let formatter_definition = OperationLocalFormatterDefinition::from_verified(&formatter)?;
    corpus.admit_formatter_specimen(&formatter_definition, formatter)?;

    let validation = specimen(
        root,
        "validation",
        NativeAcceptanceClass::CompletedSuccess,
        runtime,
    )?;
    let docs = specimen(
        root,
        "docs",
        NativeAcceptanceClass::CompletedSuccess,
        runtime,
    )?;
    let owners = OperationLocalOwnerCommandDefinition::from_verified(&validation, &docs)?;
    corpus.admit_owner_command_specimens(&owners, validation, docs)?;

    let refusal = specimen(
        root,
        "refusal",
        NativeAcceptanceClass::RefusedBeforeRelease,
        runtime,
    )?;
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
                runtime,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let post_gate_definition =
        OperationLocalPostGateDefinition::from_verified(&post_gate.iter().collect::<Vec<_>>())?;
    corpus.admit_post_gate_specimens(&post_gate_definition, post_gate)?;

    let corpus_identity = pinned(
        content_text(corpus.identity()),
        invocation.expect_corpus.as_deref(),
    )?;
    let witness = corpus
        .post_gate_witness()
        .ok_or("post-gate witness absent after admission")?;
    let pending = witness
        .pending()
        .iter()
        .map(|member| member.site)
        .collect::<Vec<_>>();
    let inputs = compose_operation_local(&corpus)?;
    println!(
        "{}",
        corpus_record(&CorpusRecord {
            corpus: &corpus_identity,
            definition: &content_text(corpus.definition_identity()),
            post_gate: &content_text(witness.identity()),
            pending: &pending,
            support: &content_text(inputs.support_definition().identity()),
            subject: &content_text(inputs.evidence().subject_id()?.content_id()),
            input_closure: &content_text(inputs.evidence().input_closure_id()?.content_id()),
        })?
    );
    println!("{}", inputs.evidence().canonical());
    Ok(())
}

/// The derived corpus identity, only once it is exactly the reviewed pin
/// when one is given: nothing is printed from an unpinned value.
fn pinned(corpus: String, expected: Option<&str>) -> Result<String> {
    match expected {
        Some(expected) if corpus != expected => {
            Err(format!("corpus {corpus} differs from expected {expected}").into())
        }
        _ => Ok(corpus),
    }
}

/// The identities one composed corpus record names.
struct CorpusRecord<'a> {
    corpus: &'a str,
    definition: &'a str,
    post_gate: &'a str,
    pending: &'a [&'a str],
    support: &'a str,
    subject: &'a str,
    input_closure: &'a str,
}

/// The v2 corpus record: the three-site post-gate witness always names its
/// pending projection member, and both authority flags stay false.
fn corpus_record(record: &CorpusRecord<'_>) -> Result<String> {
    if record.pending != ["projection-final-image"] {
        return Err(
            "the NV12.05 post-gate witness names exactly its pending projection site".into(),
        );
    }
    Ok(format!(
        "SIM_NATIVE_CORPUS schema=v2 corpus={} definition={} post-gate={} post-gate-pending={} support={} subject={} input-closure={} native-authority=false m5-qualified=false",
        record.corpus,
        record.definition,
        record.post_gate,
        record.pending.join(","),
        record.support,
        record.subject,
        record.input_closure,
    ))
}

fn main() -> Result<()> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    run(&parse(&arguments)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_corpus_record_is_exactly_v2_with_its_pending_projection_member() {
        let record = |pending: &'static [&'static str]| CorpusRecord {
            corpus: "c",
            definition: "d",
            post_gate: "p",
            pending,
            support: "s",
            subject: "j",
            input_closure: "i",
        };
        assert_eq!(
            corpus_record(&record(&["projection-final-image"])).unwrap(),
            "SIM_NATIVE_CORPUS schema=v2 corpus=c definition=d post-gate=p post-gate-pending=projection-final-image support=s subject=j input-closure=i native-authority=false m5-qualified=false"
        );
        for refused in [
            &[][..],
            &[""][..],
            &["gate-release"][..],
            &["projection-final-image", "capture-start"][..],
        ] {
            assert!(corpus_record(&record(refused)).is_err(), "{refused:?}");
        }
    }

    #[test]
    fn only_the_exact_pinned_corpus_is_accepted() {
        let corpus = "core/sha256:ab01";
        assert_eq!(pinned(corpus.into(), None).unwrap(), corpus);
        assert_eq!(pinned(corpus.into(), Some(corpus)).unwrap(), corpus);
        for other in [
            "core/sha256:ab02",
            "core/sha256:ab0",
            "core/sha256:AB01",
            "",
        ] {
            let error = pinned(corpus.into(), Some(other)).unwrap_err().to_string();
            assert_eq!(
                error,
                format!("corpus {corpus} differs from expected {other}")
            );
        }
    }

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    const SIM: &str = "0707070707070707070707070707070707070707070707070707070707070707";
    const RID: &str = "9c967476576e63f1";

    fn full(extra: &[&str]) -> Vec<String> {
        let mut values = vec![
            "/evidence",
            "--expect-sim-sha256",
            SIM,
            "--expect-runtime-rid",
            RID,
        ];
        values.extend_from_slice(extra);
        arguments(&values)
    }

    #[test]
    fn invocations_are_a_root_the_batch_runtime_and_an_optional_pin() {
        let parsed = parse(&full(&[])).unwrap();
        assert_eq!(parsed.root, PathBuf::from("/evidence"));
        assert_eq!(parsed.expect_corpus, None);
        assert_eq!(parsed.runtime, ExpectedBatchRuntime::parse(SIM, RID).unwrap());
        assert_eq!(
            parse(&full(&["--expect-corpus", "core/sha256:ab"]))
                .unwrap()
                .expect_corpus
                .as_deref(),
            Some("core/sha256:ab")
        );
        // The flags may come in any order.
        parse(&arguments(&[
            "/evidence",
            "--expect-runtime-rid",
            RID,
            "--expect-sim-sha256",
            SIM,
        ]))
        .unwrap();
        let no_runtime = arguments(&["/evidence"]);
        let only_sim = arguments(&["/evidence", "--expect-sim-sha256", SIM]);
        let only_rid = arguments(&["/evidence", "--expect-runtime-rid", RID]);
        let old_form = arguments(&["/evidence", "--expect-corpus", "core/sha256:ab"]);
        let mut duplicate = full(&[]);
        duplicate.extend(arguments(&["--expect-runtime-rid", RID]));
        let mut bad_rid = full(&[]);
        bad_rid[4] = "9C967476576E63F1".to_owned();
        let mut bad_sim = full(&[]);
        bad_sim[2] = "07".to_owned();
        for (label, refused) in [
            ("empty", vec![]),
            ("no runtime", no_runtime),
            ("only sim", only_sim),
            ("only rid", only_rid),
            ("old form without the runtime", old_form),
            ("duplicate flag", duplicate),
            ("uppercase rid", bad_rid),
            ("short sim", bad_sim),
            ("relative root", arguments(&["relative", "--expect-sim-sha256", SIM, "--expect-runtime-rid", RID])),
            ("dangling flag", full(&["--expect-corpus"])),
            ("unknown flag", full(&["--other", "x:y"])),
            ("corpus without colon", full(&["--expect-corpus", "nocolon"])),
            ("corpus without value", full(&["--expect-corpus", "x:"])),
        ] {
            assert!(parse(&refused).is_err(), "{label}");
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
            runtime: ExpectedBatchRuntime::parse(SIM, RID).unwrap(),
            expect_corpus: None,
        })
        .unwrap_err()
        .to_string();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(error.contains("specimen completed-success"), "{error}");
    }

    fn recorded(rid: &str, image: [u8; 32]) -> std::io::Result<RecordedEntryRuntime> {
        Ok(RecordedEntryRuntime {
            rid: rid.to_owned(),
            image_sha256: image,
            directory_device: 1,
            directory_inode: 2,
        })
    }

    /// Every role goes through `bound`: only the batch's pinned runtime and
    /// `sim` image composes; a plain (relabelled) record, another `RID`, another
    /// image and an unverifiable specimen each refuse, naming the role.
    #[test]
    fn a_role_composes_only_as_the_batch_runtime() {
        let runtime = ExpectedBatchRuntime::parse(SIM, RID).unwrap();
        let sim = [7_u8; 32];
        assert_eq!(
            bound("timeout", Ok(11), &runtime, |_| recorded(RID, sim)).unwrap(),
            11
        );
        let refused = |verified: std::io::Result<i32>,
                       entry: std::io::Result<RecordedEntryRuntime>| {
            bound("timeout", verified, &runtime, |_| entry)
                .unwrap_err()
                .to_string()
        };
        let plain = Err(std::io::Error::other(
            "the specimen recorded a plain entry image, not the pinned runtime",
        ));
        assert!(refused(Ok(1), plain).contains("specimen timeout: the specimen recorded a plain"));
        assert!(refused(Ok(1), recorded("0000000000000000", sim)).contains("specimen timeout"));
        assert!(refused(Ok(1), recorded(RID, [8; 32])).contains("specimen timeout"));
        let unverified = Err(std::io::Error::other("not verified"));
        assert!(
            refused(unverified, recorded(RID, sim)).contains("specimen timeout: not verified")
        );
    }

    /// No role is read except through `bound`: the only specimen reads are the
    /// two helpers, and both wrap their result in it.
    #[test]
    fn no_role_is_read_outside_the_binding_path() {
        let source = include_str!("native_corpus_compose.rs");
        let production = source.split("#[cfg(test)]\nmod tests").next().unwrap();
        let run = production.split("fn run(").nth(1).unwrap();
        assert!(!run.contains("verify_native_specimen_directory("));
        assert_eq!(run.matches("verify_recorded_provider_bound_specimen_directory(").count(), 1);
        assert!(run.contains("bound(\n        \"before-acknowledgement\""));
        let reader = production
            .split("fn specimen(")
            .nth(1)
            .unwrap()
            .split("fn bound<")
            .next()
            .unwrap();
        assert!(reader.contains("recorded_entry_runtime,"));
    }
}
