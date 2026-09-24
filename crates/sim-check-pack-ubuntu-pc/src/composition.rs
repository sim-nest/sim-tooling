// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One-way platform-corpus to neutral-checker composition.

use std::collections::BTreeMap;

use sim_conformance_packs::{OperationLocalEvidence, OperationLocalSupportDefinition, PackSubject};
use sim_kernel::{ContentId, Datum, Symbol};
use sim_platform_ubuntu_pc::{
    NativeAcceptanceClass, OperationLocalSpecimenRole, OperationLocalVerifiedFact,
    VerifiedOperationLocalCorpus, VerifiedOperationLocalFactWitness,
    VerifiedOperationLocalSpecimenBinding,
};

use crate::contract::FACTS;
use crate::{
    UbuntuOperationLocalCorpusError, UbuntuOperationLocalInputs, build_inputs_sha256,
    dependency_lock_sha256,
};

#[derive(Clone, Copy)]
pub(crate) enum Contribution<'a> {
    Specimen {
        role: OperationLocalSpecimenRole,
        acceptance: &'a ContentId,
        binding: &'a VerifiedOperationLocalSpecimenBinding,
    },
    Platform(&'a VerifiedOperationLocalFactWitness),
}

/// Every C-OP fact the platform owner can witness, with its checker name.
pub(crate) const PLATFORM_FACTS: [(OperationLocalVerifiedFact, &str); 19] = [
    (
        OperationLocalVerifiedFact::PortIsPortable,
        "operation.local-port-is-portable",
    ),
    (
        OperationLocalVerifiedFact::CommandIsInstalledAndAllowlisted,
        "operation.local-command-is-installed-and-allowlisted",
    ),
    (
        OperationLocalVerifiedFact::CommandIdBindsCompleteSpec,
        "operation.local-command-id-binds-complete-spec",
    ),
    (
        OperationLocalVerifiedFact::EnvironmentSealed,
        "operation.local-environment-sealed",
    ),
    (
        OperationLocalVerifiedFact::WritableRootsConfined,
        "operation.local-writable-roots-confined",
    ),
    (
        OperationLocalVerifiedFact::NetworkAbsent,
        "operation.local-network-absent",
    ),
    (
        OperationLocalVerifiedFact::NetworkGrantSeparate,
        "operation.local-network-grant-separate",
    ),
    (
        OperationLocalVerifiedFact::ReleaseCredentialsAbsent,
        "operation.local-release-credentials-absent",
    ),
    (
        OperationLocalVerifiedFact::DescendantsZero,
        "operation.local-descendants-zero",
    ),
    (
        OperationLocalVerifiedFact::ScratchZero,
        "operation.local-scratch-zero",
    ),
    (
        OperationLocalVerifiedFact::IndependentPostcondition,
        "operation.local-independent-postcondition",
    ),
    (
        OperationLocalVerifiedFact::TimeoutTerminatesGroup,
        "operation.local-timeout-terminates-group",
    ),
    (
        OperationLocalVerifiedFact::CancellationTerminatesGroup,
        "operation.local-cancellation-terminates-group",
    ),
    (
        OperationLocalVerifiedFact::SignalEscalationBounded,
        "operation.local-signal-escalation-bounded",
    ),
    (
        OperationLocalVerifiedFact::FormatterMutationObserved,
        "operation.local-formatter-mutation-observed",
    ),
    (
        OperationLocalVerifiedFact::ManifestScriptBytesExact,
        "operation.local-manifest-script-bytes-exact",
    ),
    (
        OperationLocalVerifiedFact::ValidationCommandExact,
        "operation.local-validation-command-exact",
    ),
    (
        OperationLocalVerifiedFact::DocsCommandExact,
        "operation.local-docs-command-exact",
    ),
    (
        OperationLocalVerifiedFact::LaterOwnerGatesReachable,
        "operation.local-later-owner-gates-reachable",
    ),
];

pub(crate) fn compose(
    corpus: &VerifiedOperationLocalCorpus,
) -> Result<UbuntuOperationLocalInputs, UbuntuOperationLocalCorpusError> {
    validate_platform_corpus(corpus)?;
    let mut contributions = platform_contributions(|fact| corpus.fact_witness(fact))?;
    add_platform_contribution(
        &mut contributions,
        "operation.local-test-success-observed",
        corpus,
        OperationLocalSpecimenRole::CompletedSuccess,
    );
    add_platform_contribution(
        &mut contributions,
        "operation.local-test-failure-observed",
        corpus,
        OperationLocalSpecimenRole::CompletedDiverged,
    );
    let missing = missing_roles(&contributions);
    if !missing.is_empty() {
        return Err(UbuntuOperationLocalCorpusError::MissingNativeRoles(missing));
    }
    finish_composition(corpus, &contributions)
}

fn validate_platform_corpus(
    corpus: &VerifiedOperationLocalCorpus,
) -> Result<(), UbuntuOperationLocalCorpusError> {
    const ROLES: [OperationLocalSpecimenRole; 4] = [
        OperationLocalSpecimenRole::CompletedSuccess,
        OperationLocalSpecimenRole::CompletedDiverged,
        OperationLocalSpecimenRole::IncompleteBeforeFinalBeforeCas,
        OperationLocalSpecimenRole::IncompleteBeforeFinalBeforeAcknowledgement,
    ];
    let mut acceptances = Vec::with_capacity(ROLES.len());
    let mut bindings = Vec::with_capacity(ROLES.len());
    for role in ROLES {
        let specimen = corpus.specimen(role);
        if specimen.class() != role_class(role) {
            return Err(UbuntuOperationLocalCorpusError::AmbiguousNativeRole(
                role_name(role),
            ));
        }
        if acceptances
            .iter()
            .any(|identity| *identity == specimen.identity())
        {
            return Err(UbuntuOperationLocalCorpusError::DuplicateNativeRole(
                role_name(role),
            ));
        }
        let binding = corpus.binding(role);
        if bindings
            .iter()
            .any(|identity| *identity == binding.identity())
        {
            return Err(UbuntuOperationLocalCorpusError::AmbiguousNativeRole(
                role_name(role),
            ));
        }
        acceptances.push(specimen.identity());
        bindings.push(binding.identity());
    }
    Ok(())
}

fn add_platform_contribution<'a>(
    contributions: &mut BTreeMap<&'static str, Vec<Contribution<'a>>>,
    fact: &'static str,
    corpus: &'a VerifiedOperationLocalCorpus,
    role: OperationLocalSpecimenRole,
) {
    contributions
        .entry(fact)
        .or_default()
        .push(Contribution::Specimen {
            role,
            acceptance: corpus.specimen(role).identity(),
            binding: corpus.binding(role),
        });
}

/// Collects every platform-derived witness the lookup can supply.
///
/// An absent witness is not an error here: it is reported, together with
/// every other absent fact, by [`missing_roles`] before any evidence exists.
pub(crate) fn platform_contributions<'a>(
    lookup: impl Fn(OperationLocalVerifiedFact) -> Option<&'a VerifiedOperationLocalFactWitness>,
) -> Result<BTreeMap<&'static str, Vec<Contribution<'a>>>, UbuntuOperationLocalCorpusError> {
    let mut contributions = BTreeMap::new();
    for (role, fact) in PLATFORM_FACTS {
        let Some(witness) = lookup(role) else {
            continue;
        };
        if witness.fact() != role {
            return Err(UbuntuOperationLocalCorpusError::AmbiguousNativeRole(
                role.as_str(),
            ));
        }
        contributions.insert(fact, vec![Contribution::Platform(witness)]);
    }
    Ok(contributions)
}

pub(crate) fn missing_roles(
    contributions: &BTreeMap<&'static str, Vec<Contribution<'_>>>,
) -> Vec<&'static str> {
    FACTS
        .iter()
        .filter(|fact| !contributions.contains_key(*fact))
        .map(|fact| {
            fact.strip_prefix("operation.local-")
                .expect("static fact prefix")
        })
        .collect()
}

fn finish_composition(
    corpus: &VerifiedOperationLocalCorpus,
    contributions: &BTreeMap<&'static str, Vec<Contribution<'_>>>,
) -> Result<UbuntuOperationLocalInputs, UbuntuOperationLocalCorpusError> {
    let facts = FACTS
        .iter()
        .map(|fact| ((*fact).to_owned(), "true".to_owned()))
        .collect::<BTreeMap<_, _>>();
    let evidence = OperationLocalEvidence::capture(&facts as &dyn PackSubject)
        .map_err(|error| UbuntuOperationLocalCorpusError::InvalidComposition(error.to_string()))?;
    let members = FACTS.iter().map(|fact| {
        let witnesses = contributions
            .get(fact)
            .expect("completeness checked before construction");
        support_identity(fact, corpus, witnesses).map(|identity| ((*fact).to_owned(), identity))
    });
    let support = OperationLocalSupportDefinition::new(
        members.collect::<Result<Vec<_>, UbuntuOperationLocalCorpusError>>()?,
    )
    .map_err(|error| UbuntuOperationLocalCorpusError::InvalidComposition(error.to_string()))?;
    Ok(UbuntuOperationLocalInputs {
        evidence,
        support,
        corpus: corpus.identity().clone(),
        definition: corpus.definition_identity().clone(),
    })
}

fn support_identity(
    fact: &str,
    corpus: &VerifiedOperationLocalCorpus,
    witnesses: &[Contribution<'_>],
) -> Result<ContentId, UbuntuOperationLocalCorpusError> {
    let contributors = witnesses
        .iter()
        .map(|witness| match witness {
            Contribution::Specimen {
                role,
                acceptance,
                binding,
            } => Datum::Node {
                tag: Symbol::qualified("conformance", "operation-local-native-contributor-v1"),
                fields: vec![
                    (Symbol::new("role"), Datum::String(role_name(*role).into())),
                    (Symbol::new("acceptance"), content_id_datum(acceptance)),
                    (Symbol::new("binding"), content_id_datum(binding.identity())),
                    (
                        Symbol::new("binding-value"),
                        binding.canonical_datum().clone(),
                    ),
                ],
            },
            Contribution::Platform(witness) => Datum::Node {
                tag: Symbol::qualified("conformance", "operation-local-platform-witness-v1"),
                fields: vec![
                    (Symbol::new("witness"), content_id_datum(witness.identity())),
                    (
                        Symbol::new("witness-value"),
                        witness.canonical_datum().clone(),
                    ),
                ],
            },
        })
        .collect();
    Datum::Node {
        tag: Symbol::qualified("conformance", "operation-local-native-support-v1"),
        fields: vec![
            (Symbol::new("fact"), Datum::String(fact.into())),
            (Symbol::new("corpus"), content_id_datum(corpus.identity())),
            (
                Symbol::new("definition"),
                content_id_datum(corpus.definition_identity()),
            ),
            (
                Symbol::new("producer-build-inputs-sha256"),
                Datum::String(build_inputs_sha256().into()),
            ),
            (
                Symbol::new("producer-dependency-lock-sha256"),
                Datum::String(dependency_lock_sha256().into()),
            ),
            (
                Symbol::new("checker-build-inputs-sha256"),
                Datum::String(sim_check_pack::build_inputs_sha256().into()),
            ),
            (
                Symbol::new("checker-dependency-lock-sha256"),
                Datum::String(sim_check_pack::dependency_lock_sha256().into()),
            ),
            (Symbol::new("contributors"), Datum::Vector(contributors)),
        ],
    }
    .content_id()
    .map_err(|error| UbuntuOperationLocalCorpusError::InvalidComposition(error.to_string()))
}

fn content_id_datum(identity: &ContentId) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("conformance", "content-id-v1"),
        fields: vec![
            (
                Symbol::new("algorithm"),
                Datum::Symbol(identity.algorithm.clone()),
            ),
            (Symbol::new("digest"), Datum::Bytes(identity.bytes.to_vec())),
        ],
    }
}

const fn role_name(role: OperationLocalSpecimenRole) -> &'static str {
    match role {
        OperationLocalSpecimenRole::CompletedSuccess => "completed-success",
        OperationLocalSpecimenRole::CompletedDiverged => "completed-diverged",
        OperationLocalSpecimenRole::IncompleteBeforeFinalBeforeCas => {
            "incomplete-before-final-before-cas"
        }
        OperationLocalSpecimenRole::IncompleteBeforeFinalBeforeAcknowledgement => {
            "incomplete-before-final-before-acknowledgement"
        }
    }
}

const fn role_class(role: OperationLocalSpecimenRole) -> NativeAcceptanceClass {
    match role {
        OperationLocalSpecimenRole::CompletedSuccess => NativeAcceptanceClass::CompletedSuccess,
        OperationLocalSpecimenRole::CompletedDiverged => NativeAcceptanceClass::CompletedDiverged,
        OperationLocalSpecimenRole::IncompleteBeforeFinalBeforeCas => {
            NativeAcceptanceClass::IncompleteBeforeFinalBeforeCas
        }
        OperationLocalSpecimenRole::IncompleteBeforeFinalBeforeAcknowledgement => {
            NativeAcceptanceClass::IncompleteBeforeFinalBeforeAcknowledgement
        }
    }
}
