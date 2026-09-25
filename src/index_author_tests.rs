// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use sim_index_core::{
    DiscoveredAnchor, DiscoveredSpecimen, DiscoveredSurface, SubjectRecord, Visibility,
};

use super::*;

#[test]
fn literal_anchor_claim_is_rejected_at_parse_time() {
    let err = parse_overlay(
        r#"
schema = "sim.features"

[[feature]]
id = "feature/sim-run/bad"
title = "Bad"
summary = "Bad literal claim."
owner = "crate/sim-lib-repl"
anchors = ["anchor/cli/repl"]
"#,
    )
    .unwrap_err();

    assert!(err.contains("literal anchor claim: rejected"));
}

#[test]
fn literal_claims_are_rejected_by_index_check_too() {
    let mut doc = test_doc();
    doc.drafts.push(FeatureDraft {
        id: FeatureId::new("feature/sim-run/bad"),
        subject: SubjectId::new("crate/sim-lib-repl"),
        title: "Bad".to_owned(),
        summary: "Bad literal claim.".to_owned(),
        claims_anchors: Vec::new(),
        claims_surfaces: Vec::new(),
        claims_specimens: Vec::new(),
        literal_anchors: vec!["anchor/cli/repl".to_owned()],
        literal_surfaces: Vec::new(),
        literal_specimens: Vec::new(),
        grammar_contracts: Vec::new(),
        doc_anchor: None,
    });

    let err = check_index_doc(&doc).unwrap_err().to_string();

    assert!(err.contains("literal anchor claim"));
}

fn test_doc() -> IndexDoc {
    IndexDoc {
        schema: "sim.index".to_owned(),
        generated_by: "test".to_owned(),
        visibility: Visibility::Public,
        source_units: Vec::new(),
        subjects: vec![SubjectRecord {
            id: SubjectId::new("crate/sim-lib-repl"),
            kind: "crate".to_owned(),
            title: "sim-lib-repl".to_owned(),
        }],
        anchors: vec![DiscoveredAnchor {
            id: AnchorId::new("anchor/cli/repl"),
            subject: SubjectId::new("crate/sim-lib-repl"),
            kind: "cli-verb".to_owned(),
        }],
        declarations: Vec::new(),
        protocol_relations: Vec::new(),
        surfaces: vec![DiscoveredSurface {
            id: SurfaceId::new("cli/repl"),
            subject: SubjectId::new("crate/sim-lib-repl"),
            kind: "cli".to_owned(),
        }],
        specimens: vec![DiscoveredSpecimen {
            id: SpecimenId::new("recipe/sim-run/01-basics/version"),
            subject: SubjectId::new("crate/sim-lib-repl"),
            kind: "recipe".to_owned(),
            path: "recipes/01-basics/version/recipe.toml".to_owned(),
            language: None,
            runnable: true,
            checked: true,
            checked_by: Some("xtask check-recipes".to_owned()),
            doc_anchor: None,
        }],
        drafts: Vec::new(),
        features: Vec::new(),
        routes: Vec::new(),
        edges: Vec::new(),
    }
}
