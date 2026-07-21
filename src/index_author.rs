//! Prose-only feature overlay loading for SIM Index fragments.

use std::{collections::BTreeSet, fs, path::Path};

use sim_index_core::{
    AnchorId, FeatureDraft, FeatureId, IndexDoc, IndexEdge, RouteId, RouteRecord, RouteStep,
    SpecimenId, SubjectId, SurfaceId, check_index_doc, draft::materialize_draft,
};
use toml::{Table, Value};

const FEATURES_FILE: &str = "features.toml";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthoredOverlay {
    features: Vec<AuthoredFeature>,
    routes: Vec<RouteRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AuthoredFeature {
    draft: FeatureDraft,
    supports: Vec<FeatureId>,
}

pub(crate) fn load_optional(repo: &Path) -> Result<Option<AuthoredOverlay>, String> {
    let path = repo.join(FEATURES_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    parse_file(&path).map(Some)
}

pub(crate) fn merge_authored(
    mut doc: IndexDoc,
    overlay: AuthoredOverlay,
) -> Result<IndexDoc, String> {
    let mut support_edges = Vec::new();
    for authored in overlay.features {
        reject_literal_claims(&authored.draft)?;
        resolve_claims(&authored.draft, &doc)?;
        let feature_id = authored.draft.id.clone();
        for supported in authored.supports {
            support_edges.push(IndexEdge::relates(
                feature_id.clone(),
                "supports",
                supported,
            ));
        }
        doc.features.push(materialize_draft(authored.draft));
    }
    doc.routes.extend(overlay.routes);
    doc.edges.extend(support_edges);
    check_index_doc(&doc).map_err(|err| format!("invalid authored feature overlay: {err}"))?;
    Ok(doc)
}

fn parse_file(path: &Path) -> Result<AuthoredOverlay, String> {
    let source =
        fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    parse_overlay(&source).map_err(|err| format!("parse {}: {err}", path.display()))
}

pub(crate) fn parse_overlay(source: &str) -> Result<AuthoredOverlay, String> {
    let value = source
        .parse::<Value>()
        .map_err(|err| format!("invalid TOML: {err}"))?;
    let root = value
        .as_table()
        .ok_or("features overlay must be a TOML table")?;
    reject_unexpected_keys(root, &["schema", "feature", "route", "enforcement"], "root")?;
    let schema = required_string(root, "schema", "root")?;
    if schema != "sim.features" {
        return Err(format!(
            "unsupported schema {schema:?}; expected \"sim.features\""
        ));
    }

    Ok(AuthoredOverlay {
        features: table_array(root, "feature", "feature")?
            .into_iter()
            .enumerate()
            .map(|(index, table)| feature_from_table(table, index))
            .collect::<Result<Vec<_>, _>>()?,
        routes: table_array(root, "route", "route")?
            .into_iter()
            .enumerate()
            .map(|(index, table)| route_from_table(table, index))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn feature_from_table(table: &Table, index: usize) -> Result<AuthoredFeature, String> {
    let label = format!("feature[{index}]");
    reject_unexpected_keys(
        table,
        &[
            "id",
            "title",
            "summary",
            "owner",
            "subject",
            "audiences",
            "guidance",
            "claims_anchors",
            "claims_surfaces",
            "claims_specimens",
            "supports",
            "doc_anchor",
        ],
        &label,
    )?;
    let owner = optional_string(table, "owner", &label)?
        .or(optional_string(table, "subject", &label)?)
        .ok_or_else(|| format!("{label}.owner is required"))?;
    if let Some(guidance) = optional_string(table, "guidance", &label)? {
        ensure_ascii(&format!("{label}.guidance"), &guidance)?;
    }
    let _audiences = optional_string_list(table, "audiences", &label)?;

    Ok(AuthoredFeature {
        draft: FeatureDraft {
            id: FeatureId::new(required_string(table, "id", &label)?),
            subject: SubjectId::new(owner),
            title: required_string(table, "title", &label)?,
            summary: required_string(table, "summary", &label)?,
            claims_anchors: optional_string_list(table, "claims_anchors", &label)?
                .into_iter()
                .map(AnchorId::new)
                .collect(),
            claims_surfaces: optional_string_list(table, "claims_surfaces", &label)?
                .into_iter()
                .map(SurfaceId::new)
                .collect(),
            claims_specimens: optional_string_list(table, "claims_specimens", &label)?
                .into_iter()
                .map(SpecimenId::new)
                .collect(),
            literal_anchors: Vec::new(),
            literal_surfaces: Vec::new(),
            literal_specimens: Vec::new(),
            grammar_contracts: Vec::new(),
            doc_anchor: optional_string(table, "doc_anchor", &label)?.map(AnchorId::new),
        },
        supports: optional_string_list(table, "supports", &label)?
            .into_iter()
            .map(FeatureId::new)
            .collect(),
    })
}

fn route_from_table(table: &Table, index: usize) -> Result<RouteRecord, String> {
    let label = format!("route[{index}]");
    reject_unexpected_keys(
        table,
        &["id", "title", "task", "audiences", "steps", "doc_anchor"],
        &label,
    )?;
    let audiences = optional_string_list(table, "audiences", &label)?;
    let title = optional_string(table, "title", &label)?
        .or(optional_string(table, "task", &label)?)
        .ok_or_else(|| format!("{label}.task is required"))?;
    Ok(RouteRecord {
        id: RouteId::new(required_string(table, "id", &label)?),
        title,
        audiences,
        steps: steps_from_value(
            table
                .get("steps")
                .ok_or_else(|| format!("{label}.steps is required"))?,
            &label,
        )?,
        doc_anchor: optional_string(table, "doc_anchor", &label)?.map(AnchorId::new),
    })
}

fn steps_from_value(value: &Value, label: &str) -> Result<Vec<RouteStep>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| format!("{label}.steps must be an array"))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let step_label = format!("{label}.steps[{index}]");
            let table = item
                .as_table()
                .ok_or_else(|| format!("{step_label} must be an inline table"))?;
            reject_unexpected_keys(table, &["feature", "specimen", "why"], &step_label)?;
            let why = required_string(table, "why", &step_label)?;
            ensure_ascii(&format!("{step_label}.why"), &why)?;
            let feature = optional_string(table, "feature", &step_label)?;
            let specimen = optional_string(table, "specimen", &step_label)?;
            match (feature, specimen) {
                (Some(id), None) => Ok(RouteStep::Feature {
                    id: FeatureId::new(id),
                    why,
                }),
                (None, Some(id)) => Ok(RouteStep::Specimen {
                    id: SpecimenId::new(id),
                    why,
                }),
                (Some(_), Some(_)) => Err(format!(
                    "{step_label} must not target both feature and specimen"
                )),
                (None, None) => Err(format!("{step_label} must target feature or specimen")),
            }
        })
        .collect()
}

fn reject_literal_claims(draft: &FeatureDraft) -> Result<(), String> {
    if !draft.literal_anchors.is_empty() {
        return Err("literal anchor claim: rejected".to_owned());
    }
    if !draft.literal_surfaces.is_empty() {
        return Err("literal surface claim: rejected".to_owned());
    }
    if !draft.literal_specimens.is_empty() {
        return Err("literal specimen claim: rejected".to_owned());
    }
    Ok(())
}

fn resolve_claims(draft: &FeatureDraft, doc: &IndexDoc) -> Result<(), String> {
    let subjects = ids(doc.subjects.iter().map(|record| record.id.as_str()));
    let anchors = ids(doc.anchors.iter().map(|record| record.id.as_str()));
    let surfaces = ids(doc.surfaces.iter().map(|record| record.id.as_str()));
    let specimens = ids(doc.specimens.iter().map(|record| record.id.as_str()));
    require_discovered(&subjects, "subject", draft.subject.as_str())?;
    for id in &draft.claims_anchors {
        require_discovered(&anchors, "anchor", id.as_str())?;
    }
    for id in &draft.claims_surfaces {
        require_discovered(&surfaces, "surface", id.as_str())?;
    }
    for id in &draft.claims_specimens {
        require_discovered(&specimens, "specimen", id.as_str())?;
    }
    Ok(())
}

fn ids<'a>(values: impl Iterator<Item = &'a str>) -> BTreeSet<&'a str> {
    values.collect()
}

fn require_discovered(known: &BTreeSet<&str>, kind: &str, id: &str) -> Result<(), String> {
    if known.contains(id) {
        Ok(())
    } else {
        Err(format!("unresolved discovered id: rejected: {kind} {id}"))
    }
}

fn table_array<'a>(root: &'a Table, key: &str, label: &str) -> Result<Vec<&'a Table>, String> {
    let Some(value) = root.get(key) else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("{label} must be an array of tables"))?;
    array
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_table()
                .ok_or_else(|| format!("{label}[{index}] must be a table"))
        })
        .collect()
}

fn reject_unexpected_keys(table: &Table, allowed: &[&str], label: &str) -> Result<(), String> {
    for key in table.keys() {
        if allowed.contains(&key.as_str()) {
            continue;
        }
        if let Some(kind) = literal_kind_for_key(key) {
            return Err(format!("literal {kind} claim: rejected"));
        }
        return Err(format!("{label} has unsupported key {key:?}"));
    }
    Ok(())
}

fn literal_kind_for_key(key: &str) -> Option<&'static str> {
    match key {
        "anchor" | "anchors" | "literal_anchor" | "literal_anchors" => Some("anchor"),
        "surface" | "surfaces" | "literal_surface" | "literal_surfaces" => Some("surface"),
        "specimen" | "specimens" | "literal_specimen" | "literal_specimens" => Some("specimen"),
        _ => None,
    }
}

fn required_string(table: &Table, key: &str, label: &str) -> Result<String, String> {
    optional_string(table, key, label)?.ok_or_else(|| format!("{label}.{key} is required"))
}

fn optional_string(table: &Table, key: &str, label: &str) -> Result<Option<String>, String> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| format!("{label}.{key} must be a string"))?;
    ensure_ascii(&format!("{label}.{key}"), value)?;
    Ok(Some(value.to_owned()))
}

fn optional_string_list(table: &Table, key: &str, label: &str) -> Result<Vec<String>, String> {
    let Some(value) = table.get(key) else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("{label}.{key} must be an array"))?;
    array
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let value = item
                .as_str()
                .ok_or_else(|| format!("{label}.{key}[{index}] must be a string"))?;
            ensure_ascii(&format!("{label}.{key}[{index}]"), value)?;
            Ok(value.to_owned())
        })
        .collect()
}

fn ensure_ascii(label: &str, value: &str) -> Result<(), String> {
    if value.is_ascii() {
        Ok(())
    } else {
        Err(format!("{label} contains non-ASCII text"))
    }
}

#[cfg(test)]
mod tests {
    use sim_index_core::{
        DiscoveredAnchor, DiscoveredSpecimen, DiscoveredSurface, SubjectRecord, Visibility,
    };

    use super::*;

    #[test]
    fn parses_and_materializes_prose_only_features() {
        let overlay = parse_overlay(
            r#"
schema = "sim.features"

[[feature]]
id = "feature/sim-run/repl"
title = "Interactive REPL"
summary = "Run a checked interactive session."
owner = "crate/sim-lib-repl"
audiences = ["user"]
guidance = "Start here when exploring the runtime."
claims_anchors = ["anchor/cli/repl"]
claims_surfaces = ["cli/repl"]
claims_specimens = ["recipe/sim-run/01-basics/version"]

[[route]]
id = "route/start-a-session"
task = "Start a SIM session"
audiences = ["user"]
steps = [
  { feature = "feature/sim-run/repl", why = "The REPL is interactive." },
  { specimen = "recipe/sim-run/01-basics/version", why = "The recipe is runnable." },
]
"#,
        )
        .expect("parse overlay");

        let merged = merge_authored(test_doc(), overlay).expect("merge overlay");

        assert_eq!(merged.features.len(), 1);
        assert_eq!(merged.routes.len(), 1);
        assert_eq!(merged.features[0].id.as_str(), "feature/sim-run/repl");
        assert_eq!(merged.features[0].anchors[0].as_str(), "anchor/cli/repl");
    }

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

    #[test]
    fn unresolved_discovered_ids_are_rejected_before_materialization() {
        let overlay = parse_overlay(
            r#"
schema = "sim.features"

[[feature]]
id = "feature/sim-run/missing"
title = "Missing"
summary = "Missing discovered row."
owner = "crate/sim-lib-repl"
claims_specimens = ["recipe/sim-run/missing"]
"#,
        )
        .expect("parse overlay");

        let err = merge_authored(test_doc(), overlay).unwrap_err();

        assert!(err.contains("unresolved discovered id: rejected"));
    }

    fn test_doc() -> IndexDoc {
        IndexDoc {
            schema: "sim.index".to_owned(),
            generated_by: "test".to_owned(),
            visibility: Visibility::Public,
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
}
