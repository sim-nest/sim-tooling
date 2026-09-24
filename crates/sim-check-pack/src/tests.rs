use std::collections::BTreeMap;
use std::fs;

use sha2::{Digest, Sha256};
use sim_conformance_core::EvidenceGrade;
use sim_conformance_packs::{
    OperationLocalAuthorityError, OperationLocalEvidence, OperationLocalOwner,
    OperationLocalSupportDefinition, PackSubject, find_pack,
};
use sim_kernel::{ContentId, Datum, Symbol};

use super::*;

fn evidence() -> OperationLocalEvidence {
    const KEYS: &[&str] = &[
        "operation.local-port-is-portable",
        "operation.local-command-is-installed-and-allowlisted",
        "operation.local-command-id-binds-complete-spec",
        "operation.local-manifest-script-bytes-exact",
        "operation.local-model-interpolation-refused",
        "operation.local-environment-sealed",
        "operation.local-writable-roots-confined",
        "operation.local-network-absent",
        "operation.local-network-grant-separate",
        "operation.local-release-credentials-absent",
        "operation.local-formatter-mutation-observed",
        "operation.local-test-success-observed",
        "operation.local-test-failure-observed",
        "operation.local-validation-command-exact",
        "operation.local-docs-command-exact",
        "operation.local-timeout-terminates-group",
        "operation.local-cancellation-terminates-group",
        "operation.local-signal-escalation-bounded",
        "operation.local-descendants-zero",
        "operation.local-scratch-zero",
        "operation.local-independent-postcondition",
        "operation.local-later-owner-gates-reachable",
    ];
    let facts = KEYS
        .iter()
        .map(|key| ((*key).to_owned(), "true".to_owned()))
        .collect::<BTreeMap<_, _>>();
    OperationLocalEvidence::capture(&facts as &dyn PackSubject).unwrap()
}

fn support(evidence: &OperationLocalEvidence, variant: u8) -> OperationLocalSupportDefinition {
    OperationLocalSupportDefinition::new(evidence.canonical().lines().map(|line| {
        let (name, _) = line.split_once('=').unwrap();
        (
            name.to_owned(),
            Datum::Vector(vec![
                Datum::String(name.to_owned()),
                Datum::Bytes(vec![variant]),
            ])
            .content_id()
            .unwrap(),
        )
    }))
    .unwrap()
}

#[test]
fn native_call_uses_distinct_binding_and_live_owner() {
    let evidence = evidence();
    let owner = OperationLocalOwner::boot(ContentId::from_bytes(
        Symbol::qualified("test", "native-checker-owner"),
        [7; 32],
    ));
    owner.mark_current(evidence.subject_id().unwrap()).unwrap();
    let authority = owner.operation_handle();
    let support = support(&evidence, 1);
    let qualified = invoke_operation_local(&evidence, &support, &authority).unwrap();
    assert_eq!(
        qualified.invocation().execution().entrypoint(),
        "sim_check_pack::invoke_operation_local"
    );
    let comparison = qualified.comparison_identities();
    assert_eq!(comparison.support_definition(), support.identity());
    assert_eq!(
        comparison.subject(),
        evidence.subject_id().unwrap().content_id()
    );
    assert_eq!(
        comparison.input_closure(),
        evidence.input_closure_id().unwrap().content_id()
    );
    assert_eq!(comparison.grade(), EvidenceGrade::Bootstrap);
    assert!(qualified.verify_current(&authority).is_ok());
    drop(owner);
    assert!(qualified.verify_current(&authority).is_err());
}

#[test]
fn preparation_is_authority_free_and_matches_live_issue() {
    let evidence = evidence();
    let support = support(&evidence, 1);
    let prepared = prepare_operation_local(&evidence, &support).unwrap();
    assert_eq!(prepared.grade(), EvidenceGrade::Bootstrap);
    assert_eq!(
        prepared.invocation().execution().entrypoint(),
        "sim_check_pack::invoke_operation_local"
    );
    assert_eq!(prepared.observations().len(), 22);

    let expected_result = prepared.expected_result().clone();
    let owner = OperationLocalOwner::boot(ContentId::from_bytes(
        Symbol::qualified("test", "prepared-checker-owner"),
        [8; 32],
    ));
    owner.mark_current(evidence.subject_id().unwrap()).unwrap();
    let qualified = prepared.issue(&owner.operation_handle()).unwrap();
    assert_eq!(qualified.receipt().result(), &expected_result);
}

#[test]
fn preparation_refuses_a_false_fact_without_issuing_owner_state() {
    let mut bytes = evidence().canonical().as_bytes().to_vec();
    let position = bytes
        .windows(b"=true".len())
        .position(|window| window == b"=true")
        .unwrap();
    bytes.splice(position + 1..position + 5, b"false".iter().copied());
    let false_evidence = OperationLocalEvidence::parse(&bytes).unwrap();
    let support = support(&false_evidence, 1);
    assert!(matches!(
        prepare_operation_local(&false_evidence, &support),
        Err(OperationLocalAuthorityError::Refused(_))
    ));
}

#[test]
fn sdk_identity_binds_the_invoking_workspace_lock() {
    let lock = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|directory| directory.join("Cargo.lock"))
        .find(|path| path.is_file())
        .expect("tooling workspace lock");
    let actual = Sha256::digest(fs::read(lock).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let spec = find_pack("checker/c-op").unwrap();
    assert_eq!(spec.dependency_lock_sha256(), actual);
    assert_eq!(dependency_lock_sha256(), actual);
}

#[test]
fn different_resolver_graphs_are_refused_before_checker_execution() {
    assert!(matches!(
        prepared::incompatible_build_graph_for_test(),
        Err(OperationLocalAuthorityError::IncompatibleBuildGraph)
    ));
}

#[test]
fn support_substitution_changes_typed_support_and_provenance_identities() {
    let evidence = evidence();
    let owner = OperationLocalOwner::boot(ContentId::from_bytes(
        Symbol::qualified("test", "support-substitution-owner"),
        [9; 32],
    ));
    owner.mark_current(evidence.subject_id().unwrap()).unwrap();
    let authority = owner.operation_handle();
    let first = invoke_operation_local(&evidence, &support(&evidence, 1), &authority).unwrap();
    let second = invoke_operation_local(&evidence, &support(&evidence, 2), &authority).unwrap();
    let first = first.comparison_identities();
    let second = second.comparison_identities();
    assert_eq!(first.result(), second.result());
    assert_eq!(first.grade(), EvidenceGrade::Bootstrap);
    assert_ne!(first.support(), second.support());
    assert_ne!(first.provenance(), second.provenance());
}

#[test]
fn missing_support_member_refuses_before_checker_issue() {
    let evidence = evidence();
    let owner = OperationLocalOwner::boot(ContentId::from_bytes(
        Symbol::qualified("test", "missing-support-owner"),
        [10; 32],
    ));
    owner.mark_current(evidence.subject_id().unwrap()).unwrap();
    let authority = owner.operation_handle();
    let mut members = support(&evidence, 1).members().clone();
    members.pop_first();
    let incomplete = OperationLocalSupportDefinition::new(members).unwrap();
    assert!(matches!(
        invoke_operation_local(&evidence, &incomplete, &authority),
        Err(OperationLocalAuthorityError::InvalidSupport(_))
    ));
}
