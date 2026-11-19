// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Admission of one git-worktree receipt whose result is already claim 21.
//!
//! The pack reads the committed manifests. This module copies that receipt
//! into the world catalog. It does not rewrite a checker result and it does
//! not derive a seal. Tags, mirrors, pins, and a registry are not read.

use std::collections::BTreeMap;

use sim_conformance_packs::packs::release::{GitPackage, observe_git_release};
use sim_kernel::Datum;
use sim_lib_world::completion::{SuppliedOwnerClaim, final_catalog, supply_owner_claim};

use crate::owner_receipt::{OwnerReceiptRefusal, claim_number};

/// Observes `packages` for `subject` and assembles the catalog with that receipt.
///
/// The release input is the identity lines the pack read. A published manifest
/// is refused by the pack and is not supplied.
///
/// # Errors
///
/// Returns the pack refusal when a manifest is not an unpublished package, a
/// parse refusal when the result is not a completion claim, and a catalog
/// refusal when assembly fails.
pub fn admit_git_release(
    subject: &str,
    packages: &[GitPackage],
) -> Result<Datum, OwnerReceiptRefusal> {
    let observed =
        observe_git_release(subject, packages).map_err(|failure| OwnerReceiptRefusal {
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
    final_catalog(&observed.identities.join("\n")).map_err(|error| OwnerReceiptRefusal {
        code: "catalog-refused".to_owned(),
        detail: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use sim_conformance_packs::packs::release::GitPackage;
    use sim_kernel::Datum;
    use sim_lib_world::completion::final_catalog;

    use super::admit_git_release;

    #[test]
    fn an_unpublished_git_package_is_admitted_as_claim_21() {
        let before = final_catalog("").expect("catalog");
        let subject = field(&before, "subject").expect("subject");
        assert!(
            !claims(&before).iter().any(|claim| claim == "21"),
            "claim 21 is not proved before the git receipt"
        );
        let dir = scratch("unpublished");
        let revision = commit_package(&dir, "sim-example", false);
        let after = admit_git_release(&subject, &[package(&dir, &revision)]).expect("admitted");
        assert!(
            claims(&after).iter().any(|claim| claim == "21"),
            "claims {:?}",
            claims(&after)
        );
        assert!(!claims(&after).iter().any(|claim| claim == "17"));
        assert_eq!(field(&after, "subject").as_deref(), Some(subject.as_str()));
        assert_eq!(field(&after, "seal").as_deref(), Some("unclaimed"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_published_manifest_supplies_nothing() {
        let before = final_catalog("").expect("catalog");
        let subject = field(&before, "subject").expect("subject");
        let dir = scratch("published");
        let revision = commit_package(&dir, "sim-example", true);
        let error = admit_git_release(&subject, &[package(&dir, &revision)]).expect_err("refused");
        assert_ne!(error.code, "not-completion-claim");
        let after = final_catalog("").expect("catalog");
        assert!(
            !claims(&after).iter().any(|claim| claim == "21"),
            "a published manifest must not admit claim 21"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    fn package(dir: &Path, revision: &str) -> GitPackage {
        GitPackage {
            repository: dir.to_path_buf(),
            commit: revision.to_owned(),
            manifest: "Cargo.toml".to_owned(),
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nv12-git-release-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    fn commit_package(dir: &Path, name: &str, publish: bool) -> String {
        git(dir, &["init", "-b", "main"]);
        git(dir, &["config", "user.email", "nv12@example.com"]);
        git(dir, &["config", "user.name", "nv12"]);
        std::fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\npublish = {publish}\n"
            ),
        )
        .expect("manifest");
        git(dir, &["add", "Cargo.toml"]);
        git(dir, &["commit", "-m", "package"]);
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
