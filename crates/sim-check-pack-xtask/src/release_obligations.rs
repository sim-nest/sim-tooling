// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Admission of one release-obligation receipt whose result is already claim 21.
//!
//! The pack reads crate bytes, the release tag, both mirrors, the pin, the
//! checkout, dependency order, and the boot program. This module copies that
//! receipt into the world catalog. It does not rewrite a checker result and
//! it does not derive a seal.

use std::collections::BTreeMap;

use sim_conformance_packs::packs::release::{
    ObservedReleaseObligations, ReleaseObligations, observe_release_obligations,
};
use sim_kernel::Datum;
use sim_lib_world::completion::{SuppliedOwnerClaim, final_catalog, supply_owner_claim};

use crate::owner_receipt::{OwnerReceiptRefusal, claim_number};

/// Observes `input` for `subject` and assembles the catalog with that receipt.
///
/// The release input lists each published package as `registry:name@version`
/// and each unpublished package as the git identity the pack read, in the
/// pack's dependency order. A refused observation is not supplied.
///
/// # Errors
///
/// Returns the pack refusal when a checksum, tag, mirror, pin, checkout,
/// order, or boot read does not agree, a parse refusal when the result is
/// not a completion claim, and a catalog refusal when assembly fails.
pub fn admit_release_obligations(
    subject: &str,
    input: &ReleaseObligations,
) -> Result<Datum, OwnerReceiptRefusal> {
    let observed =
        observe_release_obligations(subject, input).map_err(|failure| OwnerReceiptRefusal {
            code: failure.code.to_owned(),
            detail: failure.detail,
        })?;
    let claim = claim_number(&observed.result)?;
    let releases = release_input(&observed)?;
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
    final_catalog(&releases).map_err(|error| OwnerReceiptRefusal {
        code: "catalog-refused".to_owned(),
        detail: error.to_string(),
    })
}

fn release_input(observed: &ObservedReleaseObligations) -> Result<String, OwnerReceiptRefusal> {
    let mut lines = Vec::new();
    for name in &observed.order {
        if let Some(published) = observed.published.iter().find(|row| &row.name == name) {
            lines.push(format!("registry:{}@{}", published.name, published.version));
            continue;
        }
        let identity = observed.identities.iter().find(|identity| {
            identity
                .split_once(':')
                .and_then(|(_, spec)| spec.split_once('@'))
                .is_some_and(|(package, _)| package == name)
        });
        let Some(identity) = identity else {
            return Err(OwnerReceiptRefusal {
                code: "release-order".to_owned(),
                detail: name.clone(),
            });
        };
        lines.push(identity.clone());
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use sim_conformance_packs::packs::release::{
        GitPackage, PublishedObligation, ReleaseObligations,
    };
    use sim_kernel::Datum;
    use sim_lib_world::completion::final_catalog;

    use super::admit_release_obligations;

    const TOP: &str = "\
[package]
name = \"sim-top\"
version = \"0.1.0\"
edition = \"2021\"
publish = true

[dependencies]
serde = \"1\"
sim-base = \"0.1.0\"
";

    struct Lab {
        root: PathBuf,
        obligations: ReleaseObligations,
        mark: PathBuf,
    }

    impl Drop for Lab {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    impl Lab {
        fn open(name: &str, checksum_ok: bool) -> Self {
            let root = std::env::temp_dir().join(format!(
                "nv12-release-obligations-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let repo = root.join("repo");
            std::fs::create_dir_all(repo.join("sim-base")).expect("repo");
            std::fs::write(repo.join("Cargo.toml"), TOP).expect("top");
            std::fs::write(
                repo.join("sim-base/Cargo.toml"),
                "[package]\nname = \"sim-base\"\nversion = \"0.1.0\"\nedition = \"2021\"\npublish = false\n",
            )
            .expect("base");
            git(&repo, &["init", "-b", "main"]);
            git(&repo, &["config", "user.email", "nv12@example.com"]);
            git(&repo, &["config", "user.name", "nv12"]);
            git(&repo, &["add", "Cargo.toml", "sim-base"]);
            git(&repo, &["commit", "-m", "packages"]);
            let commit = head(&repo);
            git(&repo, &["tag", "sim-top-v0.1.0", &commit]);
            let pin = root.join("repos.toml");
            std::fs::write(
                &pin,
                format!("[[repo]]\nname = \"sim-top\"\ncommit = \"{commit}\"\n"),
            )
            .expect("pin");
            let krate = root.join("sim-top.crate");
            pack_crate(&root.join("stage"), &krate, &commit);
            let sum = if checksum_ok {
                sha256_file(&krate)
            } else {
                "a".repeat(64)
            };
            let index = root.join("index");
            std::fs::write(
                &index,
                format!(r#"{{"name":"sim-top","vers":"0.1.0","cksum":"{sum}","yanked":false}}"#),
            )
            .expect("index");
            let mark = root.join("boot.mark");
            let boot = root.join("boot");
            std::fs::write(
                &boot,
                format!("#!/bin/sh\ntouch {}\nprintf 'booted\\n'\n", mark.display()),
            )
            .expect("boot");
            let mut permissions = std::fs::metadata(&boot).expect("meta").permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&boot, permissions).expect("mode");
            let mirror = root.join("mirror.git");
            let cloned = Command::new("git")
                .args([
                    "clone",
                    "--bare",
                    repo.to_str().expect("repo"),
                    mirror.to_str().expect("mirror"),
                ])
                .output()
                .expect("clone");
            assert!(
                cloned.status.success(),
                "{}",
                String::from_utf8_lossy(&cloned.stderr)
            );
            let obligations = ReleaseObligations {
                unpublished: vec![GitPackage {
                    repository: repo.clone(),
                    commit: commit.clone(),
                    manifest: "sim-base/Cargo.toml".to_owned(),
                }],
                published: vec![PublishedObligation {
                    index,
                    krate,
                    repository: repo.clone(),
                    pin_manifest: pin,
                    pin_name: "sim-top".to_owned(),
                    checkout: repo.clone(),
                    mirror_left: repo.display().to_string(),
                    mirror_right: mirror.display().to_string(),
                }],
                boot_program: boot,
                boot_args: Vec::new(),
            };
            Self {
                root,
                obligations,
                mark,
            }
        }
    }

    #[test]
    fn a_read_release_is_admitted_as_claim_21() {
        let before = final_catalog("").expect("catalog");
        let subject = field(&before, "subject").expect("subject");
        assert!(!claims(&before).iter().any(|claim| claim == "21"));
        let lab = Lab::open("pass", true);
        let after = admit_release_obligations(&subject, &lab.obligations).expect("admitted");
        assert!(
            claims(&after).iter().any(|claim| claim == "21"),
            "claims {:?}",
            claims(&after)
        );
        assert!(!claims(&after).iter().any(|claim| claim == "17"));
        assert_eq!(field(&after, "subject").as_deref(), Some(subject.as_str()));
        assert_eq!(field(&after, "seal").as_deref(), Some("unclaimed"));
        assert!(
            releases(&after).iter().any(|line| line.starts_with("git+")),
            "releases {:?}",
            releases(&after)
        );
        assert!(
            releases(&after)
                .iter()
                .any(|line| line == "registry:sim-top@0.1.0"),
            "releases {:?}",
            releases(&after)
        );
        assert!(lab.mark.is_file(), "boot program did not run");
    }

    #[test]
    fn a_checksum_mismatch_supplies_nothing() {
        let before = final_catalog("").expect("catalog");
        let subject = field(&before, "subject").expect("subject");
        let lab = Lab::open("checksum", false);
        let error = admit_release_obligations(&subject, &lab.obligations).expect_err("refused");
        assert_eq!(error.code, "checksum-mismatch");
        let after = final_catalog("").expect("catalog");
        assert!(
            !claims(&after).iter().any(|claim| claim == "21"),
            "a refused release must not admit claim 21"
        );
        assert!(!lab.mark.is_file(), "a refused release must not boot");
    }

    fn pack_crate(stage: &Path, krate: &Path, sha: &str) {
        let member = stage.join("sim-top-0.1.0");
        std::fs::create_dir_all(&member).expect("stage");
        std::fs::write(member.join("Cargo.toml"), TOP).expect("crate manifest");
        std::fs::write(
            member.join(".cargo_vcs_info.json"),
            format!(r#"{{"git":{{"sha1":"{sha}"}},"path_in_vcs":""}}"#),
        )
        .expect("vcs");
        let packed = Command::new("tar")
            .args([
                "-czf",
                krate.to_str().expect("crate"),
                "-C",
                stage.to_str().expect("stage"),
                "sim-top-0.1.0",
            ])
            .output()
            .expect("tar");
        assert!(
            packed.status.success(),
            "{}",
            String::from_utf8_lossy(&packed.stderr)
        );
    }

    fn sha256_file(path: &Path) -> String {
        let output = Command::new("sha256sum")
            .arg(path)
            .output()
            .expect("sha256sum");
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .expect("utf8")
            .split_whitespace()
            .next()
            .expect("digest")
            .to_owned()
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
        strings(datum, "admissions")
    }

    fn releases(datum: &Datum) -> Vec<String> {
        strings(datum, "releases")
    }

    fn strings(datum: &Datum, name: &str) -> Vec<String> {
        let Datum::Node { fields, .. } = datum else {
            return Vec::new();
        };
        let Some(value) = fields
            .iter()
            .find(|(symbol, _)| symbol.as_qualified_str() == name)
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
