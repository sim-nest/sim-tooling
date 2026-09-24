// conformance: Ubuntu operation-local composition accounts exact native roles
// and refuses incomplete, duplicated, ambiguous, or forged platform evidence.

use std::collections::BTreeMap;

use super::*;
use crate::composition::{Contribution, missing_roles};
use crate::contract::FACTS;

#[test]
fn current_jk_baseline_cannot_blanket_prove_twenty_two_facts() {
    let contributions = BTreeMap::from([
        (
            "operation.local-port-is-portable",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-command-is-installed-and-allowlisted",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-command-id-binds-complete-spec",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-environment-sealed",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-writable-roots-confined",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-network-absent",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-network-grant-separate",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-release-credentials-absent",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-descendants-zero",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-scratch-zero",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-independent-postcondition",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-test-success-observed",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-test-failure-observed",
            Vec::<Contribution<'_>>::new(),
        ),
    ]);
    assert_eq!(
        missing_roles(&contributions),
        MISSING_PLATFORM_WITNESS_ROLES
    );
}

#[test]
fn missing_role_report_is_complete_bounded_and_unambiguous() {
    assert_eq!(FACTS.len(), 22);
    assert_eq!(MISSING_PLATFORM_WITNESS_ROLES.len(), 9);
    assert!(
        MISSING_PLATFORM_WITNESS_ROLES
            .windows(2)
            .all(|pair| pair[0] != pair[1])
    );
    let error = UbuntuOperationLocalCorpusError::MissingNativeRoles(
        MISSING_PLATFORM_WITNESS_ROLES.to_vec(),
    );
    let rendered = error.to_string();
    for role in MISSING_PLATFORM_WITNESS_ROLES {
        assert!(rendered.contains(role));
    }
}
