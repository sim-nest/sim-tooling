//! Shared coverage and strictness rules for SIM Index maintenance.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use sim_index_core::{FeatureRecord, IndexDoc, SubjectId};
use toml::{Table, Value};

const FEATURES_FILE: &str = "features.toml";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ClaimKind {
    Anchor,
    Surface,
    Specimen,
}

impl ClaimKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Anchor => "anchor",
            Self::Surface => "surface",
            Self::Specimen => "specimen",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaimableItem {
    pub(crate) kind: ClaimKind,
    pub(crate) id: String,
    pub(crate) owner: SubjectId,
    facet: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MissingDraftRow {
    pub(crate) kind: &'static str,
    pub(crate) id: String,
    pub(crate) owner: SubjectId,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Strictness {
    strict_audiences: BTreeSet<String>,
    strict_surfaces: BTreeSet<String>,
    strict_specimens: BTreeSet<String>,
    strict_routes: BTreeSet<String>,
    strict_overlap: BTreeSet<String>,
}

impl Strictness {
    pub(crate) fn load(repo: &Path) -> Result<Self, String> {
        let path = repo.join(FEATURES_FILE);
        if !path.is_file() {
            return Ok(Self::default());
        }
        let source =
            fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        Self::parse_features_toml(&source).map_err(|err| format!("parse {}: {err}", path.display()))
    }

    pub(crate) fn parse_features_toml(source: &str) -> Result<Self, String> {
        let value = source
            .parse::<Value>()
            .map_err(|err| format!("invalid TOML: {err}"))?;
        let Some(enforcement) = value.as_table().and_then(|root| root.get("enforcement")) else {
            return Ok(Self::default());
        };
        let enforcement = enforcement
            .as_table()
            .ok_or("enforcement must be a TOML table")?;
        let mut out = Self::default();
        for (name, value) in enforcement {
            let table = value
                .as_table()
                .ok_or_else(|| format!("enforcement.{name} must be a TOML table"))?;
            match name.as_str() {
                "audience" => strict_entries(table, &mut out.strict_audiences)?,
                "surface" => strict_entries(table, &mut out.strict_surfaces)?,
                "specimen" => strict_entries(table, &mut out.strict_specimens)?,
                "route" => strict_entries(table, &mut out.strict_routes)?,
                "overlap" => strict_entries(table, &mut out.strict_overlap)?,
                other => return Err(format!("unsupported enforcement category {other:?}")),
            }
        }
        Ok(out)
    }

    pub(crate) fn apply_strict_selectors(&mut self, selectors: &str) -> Result<(), String> {
        for raw in selectors.split(',') {
            let selector = raw.trim();
            if selector.is_empty() {
                continue;
            }
            let Some((category, value)) = selector.split_once(':') else {
                return Err(format!(
                    "strict selector {selector:?} must use category:value"
                ));
            };
            let value = value.trim();
            if value.is_empty() {
                return Err(format!("strict selector {selector:?} has an empty value"));
            }
            match category.trim() {
                "audience" => {
                    self.strict_audiences.insert(value.to_owned());
                }
                "surface" => {
                    self.strict_surfaces.insert(value.to_owned());
                }
                "specimen" => {
                    self.strict_specimens.insert(value.to_owned());
                }
                "route" => {
                    self.strict_routes.insert(value.to_owned());
                }
                "overlap" => {
                    self.strict_overlap.insert(value.to_owned());
                }
                other => return Err(format!("unsupported strict selector category {other:?}")),
            }
        }
        Ok(())
    }

    pub(crate) fn requires(&self, item: &ClaimableItem) -> bool {
        match item.kind {
            ClaimKind::Anchor => {
                (self.strict_audiences.contains("user") && item.facet == "cli-verb")
                    || (self.strict_audiences.contains("framework") && item.facet == "runtime-lib")
                    || (self.strict_audiences.contains("code")
                        && matches!(item.facet.as_str(), "crate" | "runtime-lib" | "export"))
            }
            ClaimKind::Surface => {
                self.strict_surfaces.contains(&item.facet)
                    || (self.strict_audiences.contains("user") && item.facet == "cli")
            }
            ClaimKind::Specimen => {
                self.strict_specimens.contains("all") || self.strict_specimens.contains(&item.facet)
            }
        }
    }

    fn requires_route(&self, category: &str) -> bool {
        self.strict_routes.contains(category) || self.strict_routes.contains("all")
    }
}

fn strict_entries(table: &Table, out: &mut BTreeSet<String>) -> Result<(), String> {
    for (key, value) in table {
        let level = value
            .as_str()
            .ok_or_else(|| format!("enforcement value for {key:?} must be a string"))?;
        match level {
            "strict" => {
                out.insert(key.to_owned());
            }
            "advisory" => {}
            other => {
                return Err(format!(
                    "unsupported enforcement level {other:?} for {key:?}"
                ));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoverageReport {
    pub(crate) covered: usize,
    pub(crate) advisory_missing: Vec<ClaimableItem>,
    pub(crate) route_gaps: Vec<RouteGap>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RouteGap {
    pub(crate) category: String,
    pub(crate) id: String,
    pub(crate) reason: String,
}

pub(crate) fn check_coverage(
    doc: &IndexDoc,
    strictness: &Strictness,
) -> Result<CoverageReport, String> {
    let counts = claim_counts(doc);
    for ((kind, id), owners) in &counts {
        if owners.len() > 1 {
            return Err(format!(
                "duplicate claim: {} {} claimed by {}",
                kind.as_str(),
                id,
                owners.join(", ")
            ));
        }
    }

    let mut covered = 0;
    let mut advisory_missing = Vec::new();
    for item in claimable_items(doc) {
        match counts
            .get(&(item.kind, item.id.clone()))
            .map(Vec::len)
            .unwrap_or(0)
        {
            0 if strictness.requires(&item) => {
                return Err(format!("unindexed: {} {}", item.kind.as_str(), item.id));
            }
            0 => advisory_missing.push(item),
            1 => covered += 1,
            _ => unreachable!("duplicate claims are rejected before coverage"),
        }
    }
    let route_gaps = route_coverage_gaps(doc);
    for gap in &route_gaps {
        if strictness.requires_route(&gap.category) {
            return Err(format!("unrouted {}: {}", gap.category, gap.id));
        }
    }

    Ok(CoverageReport {
        covered,
        advisory_missing,
        route_gaps,
    })
}

pub(crate) fn route_coverage_gaps(doc: &IndexDoc) -> Vec<RouteGap> {
    let targets = doc
        .routes
        .iter()
        .flat_map(|route| route.steps.iter().map(|step| step.id().to_owned()))
        .collect::<BTreeSet<_>>();
    let mut gaps = Vec::new();
    for feature in &doc.features {
        if targets.contains(feature.id.as_str()) {
            continue;
        }
        if is_major_entrypoint(doc, feature) {
            gaps.push(RouteGap {
                category: "major_entrypoints".to_owned(),
                id: feature.id.to_string(),
                reason: "feature claims a CLI entry point but no route step reaches it".to_owned(),
            });
        }
        if is_framework_feature(doc, feature) {
            gaps.push(RouteGap {
                category: "frameworks".to_owned(),
                id: feature.id.to_string(),
                reason: "feature exposes framework-facing runtime or surface facts but no route step reaches it"
                    .to_owned(),
            });
        }
    }
    gaps.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then(left.id.cmp(&right.id))
    });
    gaps
}

fn is_major_entrypoint(doc: &IndexDoc, feature: &FeatureRecord) -> bool {
    feature.surfaces.iter().any(|id| {
        doc.surfaces
            .iter()
            .any(|surface| surface.id.as_str() == id.as_str() && surface.kind == "cli")
    })
}

fn is_framework_feature(doc: &IndexDoc, feature: &FeatureRecord) -> bool {
    feature_claims_runtime_subject(doc, feature)
        || feature
            .surfaces
            .iter()
            .filter_map(|id| {
                doc.surfaces
                    .iter()
                    .find(|surface| surface.id.as_str() == id.as_str())
            })
            .any(|surface| {
                matches!(
                    surface.kind.as_str(),
                    "view" | "view-edit" | "model-exchange" | "site"
                ) || subject_kind(doc, surface.subject.as_str()) == Some("runtime-lib")
            })
        || feature
            .anchors
            .iter()
            .filter_map(|id| {
                doc.anchors
                    .iter()
                    .find(|anchor| anchor.id.as_str() == id.as_str())
            })
            .any(|anchor| subject_kind(doc, anchor.subject.as_str()) == Some("runtime-lib"))
        || text_mentions_framework(&feature.title)
        || text_mentions_framework(&feature.summary)
}

fn feature_claims_runtime_subject(doc: &IndexDoc, feature: &FeatureRecord) -> bool {
    subject_kind(doc, feature.subject.as_str()) == Some("runtime-lib")
}

fn subject_kind<'a>(doc: &'a IndexDoc, id: &str) -> Option<&'a str> {
    doc.subjects
        .iter()
        .find(|subject| subject.id.as_str() == id)
        .map(|subject| subject.kind.as_str())
}

fn text_mentions_framework(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("framework") || value.contains("runtime-lib")
}

pub(crate) fn missing_draft_rows(doc: &IndexDoc) -> Vec<MissingDraftRow> {
    let counts = claim_counts(doc);
    let mut rows = claimable_items(doc)
        .into_iter()
        .filter(|item| !counts.contains_key(&(item.kind, item.id.clone())))
        .map(|item| MissingDraftRow {
            kind: item.kind.as_str(),
            id: item.id,
            owner: item.owner,
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.kind
            .cmp(right.kind)
            .then(left.id.cmp(&right.id))
            .then(left.owner.as_str().cmp(right.owner.as_str()))
    });
    rows
}

fn claimable_items(doc: &IndexDoc) -> Vec<ClaimableItem> {
    let mut items = Vec::new();
    items.extend(doc.anchors.iter().map(|record| ClaimableItem {
        kind: ClaimKind::Anchor,
        id: record.id.to_string(),
        owner: record.subject.clone(),
        facet: record.kind.clone(),
    }));
    items.extend(doc.surfaces.iter().map(|record| ClaimableItem {
        kind: ClaimKind::Surface,
        id: record.id.to_string(),
        owner: record.subject.clone(),
        facet: record.kind.clone(),
    }));
    items.extend(
        doc.specimens
            .iter()
            .filter(|record| record.runnable && record.checked)
            .map(|record| ClaimableItem {
                kind: ClaimKind::Specimen,
                id: record.id.to_string(),
                owner: record.subject.clone(),
                facet: record.kind.clone(),
            }),
    );
    items.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then(left.id.cmp(&right.id))
            .then(left.owner.as_str().cmp(right.owner.as_str()))
    });
    items
}

fn claim_counts(doc: &IndexDoc) -> BTreeMap<(ClaimKind, String), Vec<String>> {
    let mut counts = BTreeMap::<(ClaimKind, String), Vec<String>>::new();
    for draft in &doc.drafts {
        push_claims(
            &mut counts,
            draft.id.as_str(),
            ClaimKind::Anchor,
            draft.claims_anchors.iter().map(|id| id.as_str()),
        );
        push_claims(
            &mut counts,
            draft.id.as_str(),
            ClaimKind::Surface,
            draft.claims_surfaces.iter().map(|id| id.as_str()),
        );
        push_claims(
            &mut counts,
            draft.id.as_str(),
            ClaimKind::Specimen,
            draft.claims_specimens.iter().map(|id| id.as_str()),
        );
    }
    for feature in &doc.features {
        push_claims(
            &mut counts,
            feature.id.as_str(),
            ClaimKind::Anchor,
            feature.anchors.iter().map(|id| id.as_str()),
        );
        push_claims(
            &mut counts,
            feature.id.as_str(),
            ClaimKind::Surface,
            feature.surfaces.iter().map(|id| id.as_str()),
        );
        push_claims(
            &mut counts,
            feature.id.as_str(),
            ClaimKind::Specimen,
            feature.specimens.iter().map(|id| id.as_str()),
        );
    }
    counts
}

fn push_claims<'a>(
    counts: &mut BTreeMap<(ClaimKind, String), Vec<String>>,
    owner: &str,
    kind: ClaimKind,
    ids: impl Iterator<Item = &'a str>,
) {
    for id in ids {
        counts
            .entry((kind, id.to_owned()))
            .or_default()
            .push(owner.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use sim_index_core::{
        AnchorId, DiscoveredAnchor, DiscoveredSpecimen, DiscoveredSurface, FeatureId,
        FeatureRecord, SubjectRecord, SurfaceId, Visibility, key::CanonicalFeatureKey,
    };

    use super::*;

    #[test]
    fn strict_selectors_parse_category_values() {
        let mut strictness = Strictness::default();
        strictness
            .apply_strict_selectors("audience:user,surface:cli")
            .expect("parse selectors");

        assert!(strictness.strict_audiences.contains("user"));
        assert!(strictness.strict_surfaces.contains("cli"));
    }

    #[test]
    fn enforcement_table_marks_only_strict_entries() {
        let strictness = Strictness::parse_features_toml(
            r#"
schema = "sim.features"

[enforcement.audience]
user = "strict"
code = "advisory"

[enforcement.surface]
cli = "strict"
view = "advisory"
"#,
        )
        .expect("parse enforcement");

        assert!(strictness.strict_audiences.contains("user"));
        assert!(strictness.strict_surfaces.contains("cli"));
        assert!(!strictness.strict_audiences.contains("code"));
    }

    #[test]
    fn duplicate_claim_across_features_fails() {
        let mut doc = base_doc();
        doc.features
            .push(feature("feature/demo/one", &["cli/demo"], &[], &[]));
        doc.features
            .push(feature("feature/demo/two", &["cli/demo"], &[], &[]));

        let err = check_coverage(&doc, &Strictness::default()).unwrap_err();

        assert!(err.contains("duplicate claim: surface cli/demo"));
    }

    #[test]
    fn missing_strict_cli_surface_fails() {
        let doc = base_doc();
        let mut strictness = Strictness::default();
        strictness.apply_strict_selectors("surface:cli").unwrap();

        let err = check_coverage(&doc, &strictness).unwrap_err();

        assert!(err.contains("unindexed: surface cli/demo"));
    }

    #[test]
    fn strict_code_requires_reusable_code_anchors() {
        let mut doc = base_doc();
        doc.anchors.push(DiscoveredAnchor {
            id: AnchorId::new("anchor/crate/demo"),
            subject: SubjectId::new("crate/demo"),
            kind: "crate".to_owned(),
        });
        doc.anchors.push(DiscoveredAnchor {
            id: AnchorId::new("anchor/export/demo/runtime/install"),
            subject: SubjectId::new("crate/demo"),
            kind: "export".to_owned(),
        });
        doc.anchors.push(DiscoveredAnchor {
            id: AnchorId::new("anchor/rustdoc/demo/helper"),
            subject: SubjectId::new("crate/demo"),
            kind: "rustdoc-item".to_owned(),
        });
        doc.features.push(feature(
            "feature/demo/crate",
            &[],
            &[],
            &["anchor/crate/demo"],
        ));
        let mut strictness = Strictness::default();
        strictness.apply_strict_selectors("audience:code").unwrap();

        let err = check_coverage(&doc, &strictness).unwrap_err();

        assert!(err.contains("unindexed: anchor anchor/export/demo/runtime/install"));
        assert!(!err.contains("anchor/rustdoc/demo/helper"));
    }

    #[test]
    fn advisory_specimen_gap_is_reported() {
        let report = check_coverage(&base_doc(), &Strictness::default()).expect("coverage report");

        assert!(
            report
                .advisory_missing
                .iter()
                .any(|item| { item.kind == ClaimKind::Specimen && item.id == "recipe/demo/hello" })
        );
    }

    fn base_doc() -> IndexDoc {
        IndexDoc {
            schema: "sim.index".to_owned(),
            generated_by: "test".to_owned(),
            visibility: Visibility::Public,
            subjects: vec![SubjectRecord {
                id: SubjectId::new("crate/demo"),
                kind: "crate".to_owned(),
                title: "demo".to_owned(),
            }],
            anchors: vec![DiscoveredAnchor {
                id: AnchorId::new("anchor/cli/demo"),
                subject: SubjectId::new("crate/demo"),
                kind: "cli-verb".to_owned(),
            }],
            surfaces: vec![DiscoveredSurface {
                id: SurfaceId::new("cli/demo"),
                subject: SubjectId::new("crate/demo"),
                kind: "cli".to_owned(),
            }],
            specimens: vec![DiscoveredSpecimen {
                id: sim_index_core::SpecimenId::new("recipe/demo/hello"),
                subject: SubjectId::new("crate/demo"),
                kind: "recipe".to_owned(),
                path: "recipes/hello/recipe.toml".to_owned(),
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

    fn feature(id: &str, surfaces: &[&str], specimens: &[&str], anchors: &[&str]) -> FeatureRecord {
        FeatureRecord {
            id: FeatureId::new(id),
            key: CanonicalFeatureKey::new(format!("crate/demo/{}", id.replace('/', "-"))),
            subject: SubjectId::new("crate/demo"),
            title: id.to_owned(),
            summary: "Demo feature.".to_owned(),
            anchors: anchors.iter().map(|id| AnchorId::new(*id)).collect(),
            surfaces: surfaces.iter().map(|id| SurfaceId::new(*id)).collect(),
            specimens: specimens
                .iter()
                .map(|id| sim_index_core::SpecimenId::new(*id))
                .collect(),
            grammar_contracts: Vec::new(),
            doc_anchor: None,
        }
    }
}
