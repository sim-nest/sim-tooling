//! Prose-only feature overlay loading for SIM Index fragments.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use sim_index_core::{
    AnchorId, FeatureDraft, FeatureId, GrammarContract, IndexDoc, IndexEdge, RouteId, RouteRecord,
    RouteStep, SpecimenId, SubjectId, SurfaceId, check_index_doc, draft::materialize_draft,
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
    audiences: Vec<String>,
    relations: Vec<AuthoredRelation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AuthoredRelation {
    rel: String,
    to: FeatureId,
}

pub(crate) fn load_optional(repo: &Path) -> Result<Option<AuthoredOverlay>, String> {
    let path = repo.join(FEATURES_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    parse_file(&path).map(Some)
}

pub(crate) fn feature_audiences(repo: &Path) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let Some(overlay) = load_optional(repo)? else {
        return Ok(BTreeMap::new());
    };
    Ok(overlay
        .features
        .into_iter()
        .map(|feature| {
            (
                feature.draft.id.to_string(),
                feature.audiences.into_iter().collect::<BTreeSet<_>>(),
            )
        })
        .collect())
}

pub(crate) fn merge_authored(
    mut doc: IndexDoc,
    overlay: AuthoredOverlay,
) -> Result<IndexDoc, String> {
    let mut relation_edges = Vec::new();
    for authored in overlay.features {
        let mut draft = authored.draft;
        reject_literal_claims(&draft)?;
        resolve_claims(&draft, &doc)?;
        preserve_covered_grammar_contracts(&mut draft, &doc);
        let feature_id = draft.id.clone();
        for relation in authored.relations {
            relation_edges.push(IndexEdge::relates(
                feature_id.clone(),
                relation.rel,
                relation.to,
            ));
        }
        doc.features.push(materialize_draft(draft));
    }
    remove_covered_drafts(&mut doc);
    doc.routes.extend(overlay.routes);
    doc.edges.extend(relation_edges);
    check_index_doc(&doc).map_err(|err| format!("invalid authored feature overlay: {err}"))?;
    Ok(doc)
}

fn preserve_covered_grammar_contracts(draft: &mut FeatureDraft, doc: &IndexDoc) {
    let mut seen = draft
        .grammar_contracts
        .iter()
        .map(grammar_contract_key)
        .collect::<BTreeSet<_>>();
    for generated in &doc.drafts {
        if !covers_generated_draft(draft, generated) {
            continue;
        }
        for contract in &generated.grammar_contracts {
            if seen.insert(grammar_contract_key(contract)) {
                draft.grammar_contracts.push(contract.clone());
            }
        }
    }
}

fn covers_generated_draft(authored: &FeatureDraft, generated: &FeatureDraft) -> bool {
    overlaps(
        authored.claims_anchors.iter().map(|id| id.as_str()),
        generated.claims_anchors.iter().map(|id| id.as_str()),
    ) || overlaps(
        authored.claims_surfaces.iter().map(|id| id.as_str()),
        generated.claims_surfaces.iter().map(|id| id.as_str()),
    ) || overlaps(
        authored.claims_specimens.iter().map(|id| id.as_str()),
        generated.claims_specimens.iter().map(|id| id.as_str()),
    )
}

fn overlaps<'a>(left: impl Iterator<Item = &'a str>, right: impl Iterator<Item = &'a str>) -> bool {
    let left = left.collect::<BTreeSet<_>>();
    right.into_iter().any(|item| left.contains(item))
}

fn grammar_contract_key(contract: &GrammarContract) -> String {
    let decoder = contract
        .decoder
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let encoder = contract
        .encoder
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let surface = contract
        .surface
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    format!(
        "{}|{}|{}|{}|{}",
        contract.id, decoder, encoder, surface, contract.round_trip
    )
}

fn remove_covered_drafts(doc: &mut IndexDoc) {
    let mut anchors = BTreeSet::new();
    let mut surfaces = BTreeSet::new();
    let mut specimens = BTreeSet::new();
    for feature in &doc.features {
        anchors.extend(feature.anchors.iter().map(|id| id.to_string()));
        surfaces.extend(feature.surfaces.iter().map(|id| id.to_string()));
        specimens.extend(feature.specimens.iter().map(|id| id.to_string()));
    }
    doc.drafts.retain(|draft| {
        !draft
            .claims_anchors
            .iter()
            .any(|id| anchors.contains(id.as_str()))
            && !draft
                .claims_surfaces
                .iter()
                .any(|id| surfaces.contains(id.as_str()))
            && !draft
                .claims_specimens
                .iter()
                .any(|id| specimens.contains(id.as_str()))
    });
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
            "presents",
            "replaces",
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
    let audiences = optional_string_list(table, "audiences", &label)?;

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
        audiences,
        relations: relation_lists(table, &label)?,
    })
}

fn relation_lists(table: &Table, label: &str) -> Result<Vec<AuthoredRelation>, String> {
    let mut relations = Vec::new();
    for key in ["supports", "presents", "replaces"] {
        relations.extend(
            optional_string_list(table, key, label)?
                .into_iter()
                .map(|id| AuthoredRelation {
                    rel: key.to_owned(),
                    to: FeatureId::new(id),
                }),
        );
    }
    Ok(relations)
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
#[path = "index_author_tests.rs"]
mod tests;
