// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Admission of one pack receipt whose result is already a completion claim.
//!
//! The pack observes the journal. This module copies that receipt into the
//! world catalog. It does not rewrite a checker result and it does not
//! derive a seal.

use std::collections::BTreeMap;

use sim_conformance_packs::packs::release::observe_journal_completion;
use sim_kernel::Datum;
use sim_lib_world::completion::{SuppliedOwnerClaim, final_catalog, supply_owner_claim};

/// Refusal from pack observation, claim parsing, or catalog assembly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerReceiptRefusal {
    /// Stable machine-facing reason.
    pub code: String,
    /// Bounded context from the pack or the catalog.
    pub detail: String,
}

impl std::fmt::Display for OwnerReceiptRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for OwnerReceiptRefusal {}

/// Observes `body` for `subject` and assembles the catalog with that receipt.
///
/// The claim number is parsed from the pack result. A tail is accepted only
/// on claim 21. Any other result is refused and is not supplied. The catalog
/// datum is the world's own assembly of receipts already supplied on this
/// thread.
///
/// # Errors
///
/// Returns the pack refusal when the journal is not admissible, a parse
/// refusal when the result is not a completion claim, and a catalog refusal
/// when assembly fails.
pub fn admit_executed_journal(subject: &str, body: &str) -> Result<Datum, OwnerReceiptRefusal> {
    let observed =
        observe_journal_completion(subject, body).map_err(|failure| OwnerReceiptRefusal {
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

pub(crate) fn claim_number(result: &str) -> Result<u8, OwnerReceiptRefusal> {
    let (head, tail) = match result.split_once(' ') {
        Some((head, tail)) => {
            if tail.is_empty() || tail.split(' ').any(str::is_empty) {
                return Err(refusal("claim-tail", result));
            }
            (head, true)
        }
        None => (result, false),
    };
    let Some(number) = head.strip_prefix("completion/claim-") else {
        return Err(refusal("not-completion-claim", result));
    };
    let Ok(claim) = number.parse::<u8>() else {
        return Err(refusal("not-completion-claim", result));
    };
    if !(1..=25).contains(&claim) || format!("completion/claim-{claim}") != head {
        return Err(refusal("not-completion-claim", result));
    }
    if tail && claim != 21 {
        return Err(refusal("claim-tail", result));
    }
    Ok(claim)
}

fn refusal(code: &str, detail: &str) -> OwnerReceiptRefusal {
    OwnerReceiptRefusal {
        code: code.to_owned(),
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sim_conformance_packs::packs::release::{
        ExecutedEntry, ExecutedJournal, format_executed_journal,
    };
    use sim_kernel::{Datum, Symbol};
    use sim_lib_journal::{Journal, JournalEntry, JournalObject, MemoryBackend};
    use sim_lib_world::completion::final_catalog;

    use super::admit_executed_journal;

    #[test]
    fn an_executed_journal_is_admitted_as_its_own_claim() {
        let before = final_catalog("").expect("catalog");
        let subject = field(&before, "subject").expect("subject");
        assert!(
            !claims(&before).iter().any(|claim| claim == "17"),
            "claim 17 is not proved before the journal receipt"
        );
        let body = executed_body(&subject);
        let after = admit_executed_journal(&subject, &body).expect("admitted");
        assert!(
            claims(&after).iter().any(|claim| claim == "17"),
            "claims {:?}",
            claims(&after)
        );
        assert_eq!(field(&after, "subject").as_deref(), Some(subject.as_str()));
        assert_eq!(field(&after, "seal").as_deref(), Some("unclaimed"));
    }

    #[test]
    fn a_refused_journal_supplies_nothing() {
        let before = final_catalog("").expect("catalog");
        let subject = field(&before, "subject").expect("subject");
        let error = admit_executed_journal(&subject, "not-a-journal\n").expect_err("refused");
        assert_ne!(error.code, "not-completion-claim");
        let after = final_catalog("").expect("catalog");
        assert!(
            !claims(&after).iter().any(|claim| claim == "17"),
            "a refused journal must not admit claim 17"
        );
    }

    fn executed_body(subject: &str) -> String {
        let backend = Arc::new(MemoryBackend::new());
        let journal = Journal::new(Arc::clone(&backend));
        let lease = journal.acquire_lease().expect("lease");
        let object = JournalObject::from_bytes(subject.as_bytes().to_vec());
        let kind = Symbol::qualified("world", "fact");
        let first = JournalEntry::new(0, None, kind.clone(), vec![object.id.clone()]);
        let head = journal
            .publish(&lease, None, vec![object.clone()], vec![first.clone()])
            .expect("publish");
        let second = JournalEntry::new(1, Some(first.id), kind, vec![object.id.clone()]);
        journal
            .publish(&lease, Some(&head), Vec::new(), vec![second])
            .expect("reference");
        let qualified = object_kind();
        format_executed_journal(&ExecutedJournal {
            subject: subject.to_owned(),
            objects: vec![object.bytes],
            entries: vec![entry(0, &qualified), entry(1, &qualified)],
            telemetry: vec!["side-channel".to_owned()],
        })
    }

    fn object_kind() -> (String, String) {
        let qualified = Symbol::qualified("world", "fact").as_qualified_str();
        let (namespace, name) = qualified.split_once('/').expect("kind");
        (namespace.to_owned(), name.to_owned())
    }

    fn entry(sequence: u64, kind: &(String, String)) -> ExecutedEntry {
        ExecutedEntry {
            sequence,
            namespace: kind.0.clone(),
            name: kind.1.clone(),
            object: 0,
        }
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
