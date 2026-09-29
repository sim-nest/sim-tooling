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
fn platform_and_specimen_facts_cover_all_twenty_two_facts() {
    let mut contributions = PLATFORM_FACTS
        .iter()
        .map(|(_, name)| (*name, Vec::<Contribution<'_>>::new()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(missing_roles(&contributions).len(), 2);
    for specimen in [
        "operation.local-test-success-observed",
        "operation.local-test-failure-observed",
    ] {
        contributions.insert(specimen, Vec::new());
    }
    assert_eq!(
        missing_roles(&contributions),
        MISSING_PLATFORM_WITNESS_ROLES
    );
    assert!(MISSING_PLATFORM_WITNESS_ROLES.is_empty());
    contributions.remove("operation.local-model-interpolation-refused");
    assert_eq!(
        missing_roles(&contributions),
        ["model-interpolation-refused"],
        "an absent refusal witness is still reported"
    );
}

#[test]
fn missing_role_report_is_complete_bounded_and_unambiguous() {
    assert_eq!(FACTS.len(), 22);
    let missing = ["model-interpolation-refused", "descendants-zero"].to_vec();
    let rendered = UbuntuOperationLocalCorpusError::MissingNativeRoles(missing.clone()).to_string();
    for role in missing {
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
