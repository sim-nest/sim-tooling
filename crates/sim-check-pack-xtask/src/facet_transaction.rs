// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Admission of one facet-transaction receipt whose result is already claim 13.
//!
//! The pack runs the model transaction law. This module copies that receipt
//! into the world catalog. It does not rewrite a checker result and it does
//! not derive a seal.

use std::collections::BTreeMap;

use sim_conformance_packs::packs::release::observe_facet_transaction;
use sim_kernel::Datum;
use sim_lib_world::completion::{SuppliedOwnerClaim, final_catalog, supply_owner_claim};

use crate::owner_receipt::{OwnerReceiptRefusal, claim_number};

/// Observes the facet transaction for `subject` and assembles that receipt.
///
/// A refused observation is not supplied. The release input is empty because
/// this result has no tail.
///
/// # Errors
///
/// Returns the pack refusal when the transaction law fails, a parse refusal
/// when the result is not a completion claim, and a catalog refusal when
/// assembly fails.
pub fn admit_facet_transaction(subject: &str) -> Result<Datum, OwnerReceiptRefusal> {
    let observed = observe_facet_transaction(subject).map_err(|failure| OwnerReceiptRefusal {
        code: failure.code.to_owned(),
        detail: failure.detail,
    })?;
    let claim = claim_number(&observed.result)?;
    let mut revocations = BTreeMap::new();
    revocations.insert(observed.checker.clone(), observed.revocation);
    supply_owner_claim(SuppliedOwnerClaim {
        subject: observed.subject,
        claim,
        admitted: observed.admitted,
        checker: observed.checker,
        catalog: observed.catalog,
        scope: observed.scope,
        revocations,
    });
    final_catalog("").map_err(|error| OwnerReceiptRefusal {
        code: "catalog-refused".to_owned(),
        detail: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use sim_kernel::Datum;
    use sim_lib_world::completion::final_catalog;

    use super::admit_facet_transaction;

    #[test]
    fn a_claim_label_supplies_nothing() {
        let before = final_catalog("").expect("catalog");
        let error = admit_facet_transaction("completion/claim-13").expect_err("refused");
        assert_eq!(error.code, "bad-record");
        let after = final_catalog("").expect("catalog");
        assert!(
            !claims(&after).iter().any(|claim| claim == "13"),
            "a claim label must not admit claim 13"
        );
        assert_eq!(
            field(&before, "subject").as_deref(),
            field(&after, "subject").as_deref()
        );
    }

    #[test]
    fn the_transaction_is_admitted_as_claim_13() {
        let before = final_catalog("").expect("catalog");
        let subject = field(&before, "subject").expect("subject");
        assert!(
            !claims(&before).iter().any(|claim| claim == "13"),
            "claim 13 is not proved before the transaction receipt"
        );
        let after = admit_facet_transaction(&subject).expect("admitted");
        assert_eq!(claims(&after), vec!["13".to_owned()]);
        assert_eq!(field(&after, "subject").as_deref(), Some(subject.as_str()));
        assert_eq!(field(&after, "seal").as_deref(), Some("unclaimed"));
    }

    fn field(datum: &Datum, name: &str) -> Option<String> {
        let Datum::Node { fields, .. } = datum else {
            return None;
        };
        fields.iter().find_map(|(symbol, value)| {
            (symbol.as_qualified_str() == name).then(|| match value {
                Datum::String(text) => text.clone(),
                _ => String::new(),
            })
        })
    }

    fn claims(datum: &Datum) -> Vec<String> {
        let Datum::Node { fields, .. } = datum else {
            return Vec::new();
        };
        let Some(value) = fields
            .iter()
            .find(|(symbol, _)| symbol.as_qualified_str() == "admissions")
            .map(|(_, value)| value)
        else {
            return Vec::new();
        };
        let Datum::Vector(items) = value else {
            return Vec::new();
        };
        items
            .iter()
            .map(|item| match item {
                Datum::String(text) => text.clone(),
                _ => String::new(),
            })
            .collect()
    }
}
