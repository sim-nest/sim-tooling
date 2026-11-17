// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Caller-named source-identity observation.
//!
//! The manifest text is injected. This module does not read a path, choose
//! the subject, or store a second receipt.

use sim_conformance_packs::PackFailure;

/// Source-identity evidence for one caller-named subject.
///
/// [`Self::result`] is `source-identity` plus the sorted git identities taken
/// from Cargo dependency tables in the injected manifest. It is not a
/// completion claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceIdentityObservation {
    subject: String,
    result: String,
}

impl SourceIdentityObservation {
    /// Subject the caller named and the receipt records.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Source-identity result bound to [`Self::subject`].
    #[must_use]
    pub fn result(&self) -> &str {
        &self.result
    }
}

/// Observes one injected manifest for `subject`.
///
/// A refusal admits nothing. The returned result is the pack's source-identity
/// evidence for the subject argument.
///
/// # Errors
///
/// Returns the pack refusal when the manifest is missing a required fact,
/// names a publication or the construction specimen, or the catalog refuses
/// the receipt.
pub fn observe_source_identity(
    subject: &str,
    manifest: &str,
) -> Result<SourceIdentityObservation, PackFailure> {
    let observed =
        sim_conformance_packs::packs::release::observe_source_identity(subject, manifest)?;
    Ok(SourceIdentityObservation {
        subject: observed.subject,
        result: observed.result,
    })
}

#[cfg(test)]
mod tests {
    const REV: &str = "0123456789abcdef0123456789abcdef01234567";

    fn git_dep() -> String {
        format!(
            "sim-example = {{ version = \"0.1.0\", git = \"https://example.invalid/sim-example.git\", rev = \"{REV}\" }}"
        )
    }

    fn manifest() -> String {
        format!(
            "[package]\npublish = false\n[dependencies]\n{}\n",
            git_dep()
        )
    }

    fn identity() -> String {
        format!("git+{REV}:sim-example@0.1.0")
    }

    #[test]
    fn caller_named_subject_admits_source_identity() {
        let subject = "subject/tooling-caller";
        let observed = crate::observe_source_identity(subject, &manifest()).expect("admitted");
        assert_eq!(observed.subject(), subject);
        assert_eq!(observed.result(), format!("source-identity {}", identity()));
        assert!(!observed.result().contains("completion/claim-"));
        assert_ne!(observed.subject(), "subject/source-example");
    }

    #[test]
    fn metadata_table_admits_nothing() {
        let text = format!(
            "[package]\npublish = false\n[package.metadata]\n{}\n",
            git_dep()
        );
        let error = crate::observe_source_identity("subject/tooling-caller", &text)
            .expect_err("metadata table");
        assert_eq!(error.code, "missing-source");
    }

    #[test]
    fn construction_specimen_admits_nothing() {
        let text = format!(
            "[package]\npublish = false\nshade-1\n[dependencies]\n{}\n",
            git_dep()
        );
        let error = crate::observe_source_identity("subject/tooling-caller", &text)
            .expect_err("construction specimen");
        assert_eq!(error.code, "construction-specimen");
    }
}
