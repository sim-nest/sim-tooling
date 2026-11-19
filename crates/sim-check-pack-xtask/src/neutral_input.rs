// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Admission of one neutral-input receipt whose result is already claim 4.
//!
//! The pack reads the owner slot and the source it authorizes. This module
//! copies that receipt into the world catalog. It does not rewrite a checker
//! result and it does not derive a seal.

use std::collections::BTreeMap;

use sim_conformance_packs::packs::release::{SelectedSource, observe_neutral_input};
use sim_kernel::Datum;
use sim_lib_world::completion::{SuppliedOwnerClaim, final_catalog, supply_owner_claim};

use crate::owner_receipt::{OwnerReceiptRefusal, claim_number};

/// Observes `source` for `subject` and assembles the catalog with that receipt.
///
/// A refused observation is not supplied. The release input is empty because
/// this result has no tail.
///
/// # Errors
///
/// Returns the pack refusal when the records do not authorize the slot, a
/// parse refusal when the result is not a completion claim, and a catalog
/// refusal when assembly fails.
pub fn admit_neutral_input(
    subject: &str,
    source: &SelectedSource,
) -> Result<Datum, OwnerReceiptRefusal> {
    let observed =
        observe_neutral_input(subject, source).map_err(|failure| OwnerReceiptRefusal {
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
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use sim_conformance_packs::packs::release::SelectedSource;
    use sim_kernel::Datum;
    use sim_lib_world::completion::final_catalog;

    use super::admit_neutral_input;

    const SOURCE_PATH: &str = "docs/workbench/NO_V3_18.md";

    #[test]
    fn a_copied_selection_supplies_nothing() {
        let before = final_catalog("").expect("catalog");
        let subject = field(&before, "subject").expect("subject");
        let dir = scratch("copied");
        let rev = commit_file(&dir, "selection.txt", "copied\n");
        let copied = SelectedSource {
            repository: dir.clone(),
            commit: rev,
            source: SOURCE_PATH.to_owned(),
            selection: "selection.txt".to_owned(),
        };
        let error = admit_neutral_input(&subject, &copied).expect_err("refused");
        assert_eq!(error.code, "bad-record");
        let after = final_catalog("").expect("catalog");
        assert!(
            !claims(&after).iter().any(|claim| claim == "4"),
            "a copied selection must not admit claim 4"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_owner_slot_is_admitted_as_claim_4() {
        let Ok(repo) = std::env::var("NV12_OWNER_REPOSITORY") else {
            if std::env::var_os("NV12_OWNER_REQUIRED").is_some() {
                panic!("NV12_OWNER_REPOSITORY is required");
            }
            return;
        };
        let before = final_catalog("").expect("catalog");
        let subject = field(&before, "subject").expect("subject");
        assert!(
            !claims(&before).iter().any(|claim| claim == "4"),
            "claim 4 is not proved before the neutral-input receipt"
        );
        let owner = PathBuf::from(repo);
        let rev = head(&owner);
        let source = SelectedSource {
            repository: owner,
            commit: rev,
            source: SOURCE_PATH.to_owned(),
            selection: "docs/workbench/NV12_FINISH_1.md".to_owned(),
        };
        let after = admit_neutral_input(&subject, &source).expect("admitted");
        assert!(
            claims(&after).iter().any(|claim| claim == "4"),
            "claims {:?}",
            claims(&after)
        );
        assert!(!claims(&after).iter().any(|claim| claim == "1"));
        assert!(!claims(&after).iter().any(|claim| claim == "17"));
        assert!(!claims(&after).iter().any(|claim| claim == "19"));
        assert!(!claims(&after).iter().any(|claim| claim == "21"));
        assert_eq!(field(&after, "subject").as_deref(), Some(subject.as_str()));
        assert_eq!(field(&after, "seal").as_deref(), Some("unclaimed"));
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nv12-neutral-admit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    fn commit_file(dir: &Path, path: &str, body: &str) -> String {
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["config", "user.email", "nv12@example.com"]);
        git(dir, &["config", "user.name", "nv12"]);
        std::fs::write(dir.join(path), body).expect("write");
        git(dir, &["add", path]);
        git(dir, &["commit", "-q", "-m", "records"]);
        head(dir)
    }

    fn head(dir: &Path) -> String {
        let output = Command::new("git")
            .args(["--no-replace-objects", "rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .expect("rev-parse");
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .expect("utf8")
            .trim()
            .to_owned()
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git");
        assert!(status.success(), "{args:?}");
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
