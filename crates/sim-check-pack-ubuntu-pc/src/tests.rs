// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// conformance: Ubuntu operation-local composition accounts exact native roles
// and refuses incomplete, duplicated, ambiguous, or forged platform evidence.

use std::collections::BTreeMap;

use super::*;
use crate::composition::{Contribution, PLATFORM_FACTS, missing_roles, platform_contributions};
use crate::contract::FACTS;

#[test]
fn platform_fact_table_is_exact_closed_and_named_by_the_platform() {
    let mut names = PLATFORM_FACTS
        .iter()
        .map(|(fact, name)| {
            assert!(FACTS.contains(name), "{name} is not a C-OP fact");
            assert_eq!(
                name.strip_prefix("operation.local-"),
                Some(fact.as_str()),
                "platform fact name drifted"
            );
            *name
        })
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), PLATFORM_FACTS.len());
    for role in MISSING_PLATFORM_WITNESS_ROLES {
        assert!(
            PLATFORM_FACTS.iter().all(|(fact, _)| fact.as_str() != role),
            "{role} is both missing and platform-derived"
        );
    }
    assert_eq!(
        PLATFORM_FACTS.len() + 2 + MISSING_PLATFORM_WITNESS_ROLES.len(),
        FACTS.len()
    );
}

#[test]
fn current_platform_facts_cannot_blanket_prove_twenty_two_facts() {
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
        (
            "operation.local-timeout-terminates-group",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-cancellation-terminates-group",
            Vec::<Contribution<'_>>::new(),
        ),
        (
            "operation.local-signal-escalation-bounded",
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
    assert_eq!(MISSING_PLATFORM_WITNESS_ROLES.len(), 6);
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

#[test]
fn absent_platform_witnesses_are_all_reported_in_one_refusal() {
    let contributions = platform_contributions(|_| None).unwrap();
    assert!(contributions.is_empty());
    let missing = missing_roles(&contributions);
    assert_eq!(missing.len(), FACTS.len());
    for (fact, _) in PLATFORM_FACTS {
        assert!(
            missing.contains(&fact.as_str()),
            "{} not reported",
            fact.as_str()
        );
    }
    for role in MISSING_PLATFORM_WITNESS_ROLES {
        assert!(missing.contains(&role), "{role} not reported");
    }
}
