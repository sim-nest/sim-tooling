// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Admission of one portable-seam receipt whose result is already claim 19.
//!
//! The pack runs the consumer-seams specimen and the foreign-project
//! specimen against the compiled executor closure. This module copies that
//! receipt into the world catalog. It does not rewrite a checker result
//! and it does not derive a seal.

use std::collections::BTreeMap;

use sim_conformance_packs::packs::release::observe_portable_seams;
use sim_kernel::Datum;
use sim_lib_world::completion::{SuppliedOwnerClaim, final_catalog, supply_owner_claim};

use crate::owner_receipt::{OwnerReceiptRefusal, claim_number};

/// Observes `freeze` for `subject` and assembles the catalog with that receipt.
///
/// `freeze` is accepted only when it is the compiled executor closure. The
/// specimens still run. A refused observation is not supplied. The release
/// input is empty because this result has no tail.
///
/// # Errors
///
/// Returns the pack refusal when the digest differs or either specimen
/// refuses, a parse refusal when the result is not a completion claim, and
/// a catalog refusal when assembly fails.
pub fn admit_portable_seams(subject: &str, freeze: &str) -> Result<Datum, OwnerReceiptRefusal> {
    let observed =
        observe_portable_seams(subject, freeze).map_err(|failure| OwnerReceiptRefusal {
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
    use sim_conformance_packs::compiled_executor_closure;
    use sim_kernel::Datum;
    use sim_lib_world::completion::final_catalog;

    use super::admit_portable_seams;

    #[test]
    fn the_compiled_executor_is_admitted_as_claim_19() {
        let before = final_catalog("").expect("catalog");
        let subject = field(&before, "subject").expect("subject");
        assert!(
            !claims(&before).iter().any(|claim| claim == "19"),
            "claim 19 is not proved before the seam receipt"
        );
        let freeze = compiled_executor_closure();
        let after = admit_portable_seams(&subject, &freeze).expect("admitted");
        assert!(
            claims(&after).iter().any(|claim| claim == "19"),
            "claims {:?}",
            claims(&after)
        );
        assert!(!claims(&after).iter().any(|claim| claim == "17"));
        assert!(!claims(&after).iter().any(|claim| claim == "21"));
        assert_eq!(field(&after, "subject").as_deref(), Some(subject.as_str()));
        assert_eq!(field(&after, "seal").as_deref(), Some("unclaimed"));
    }

    #[test]
    fn a_wrong_freeze_supplies_nothing() {
        let before = final_catalog("").expect("catalog");
        let subject = field(&before, "subject").expect("subject");
        let wrong = "a".repeat(64);
        assert_ne!(wrong, compiled_executor_closure());
        let error = admit_portable_seams(&subject, &wrong).expect_err("refused");
        assert_eq!(error.code, "freeze-mismatch");
        let after = final_catalog("").expect("catalog");
        assert!(
            !claims(&after).iter().any(|claim| claim == "19"),
            "a wrong freeze must not admit claim 19"
        );
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
