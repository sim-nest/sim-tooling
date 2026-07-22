//! Framework view rendering for the public SIM Index.

use std::collections::BTreeSet;

use sim_index_core::{FeatureRecord, IndexDoc, RouteStep};

use crate::index_render_features::feature_page_path;

pub(crate) fn frameworks_page(doc: &IndexDoc) -> String {
    let mut out = page("SIM Index Frameworks");
    let route_ids = framework_route_ids(doc);
    let rows = doc
        .features
        .iter()
        .filter(|feature| {
            feature_is_routed_for_framework(doc, feature.id.as_str())
                || feature_claims_framework_fact(doc, feature)
                || looks_framework(&feature.title)
                || looks_framework(&feature.summary)
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        out.push_str("No framework-specific features are present.\n");
        return out;
    }
    out.push_str("| Feature | Subject | Summary | Framework routes |\n| --- | --- | --- | --- |\n");
    for feature in rows {
        let routes =
            framework_routes_for_feature(doc, feature.id.as_str(), &route_ids).join("<br>");
        out.push_str(&format!(
            "| [`{}`]({}) | `{}` | {} | {} |\n",
            feature.id,
            feature_page_path(feature.id.as_str()),
            feature.subject,
            cell(&feature.summary),
            cell(&routes)
        ));
    }
    out
}

fn framework_route_ids(doc: &IndexDoc) -> BTreeSet<String> {
    doc.routes
        .iter()
        .filter(|route| {
            route
                .audiences
                .iter()
                .any(|audience| audience == "framework")
        })
        .map(|route| route.id.to_string())
        .collect()
}

fn feature_is_routed_for_framework(doc: &IndexDoc, feature_id: &str) -> bool {
    doc.routes.iter().any(|route| {
        route
            .audiences
            .iter()
            .any(|audience| audience == "framework")
            && route.steps.iter().any(|step| match step {
                RouteStep::Feature { id, .. } => id.as_str() == feature_id,
                RouteStep::Specimen { .. } => false,
            })
    })
}

fn framework_routes_for_feature(
    doc: &IndexDoc,
    feature_id: &str,
    route_ids: &BTreeSet<String>,
) -> Vec<String> {
    doc.routes
        .iter()
        .filter(|route| route_ids.contains(route.id.as_str()))
        .filter(|route| {
            route.steps.iter().any(|step| match step {
                RouteStep::Feature { id, .. } => id.as_str() == feature_id,
                RouteStep::Specimen { .. } => false,
            })
        })
        .map(|route| route.id.to_string())
        .collect()
}

fn feature_claims_framework_fact(doc: &IndexDoc, feature: &FeatureRecord) -> bool {
    feature.anchors.iter().any(|id| {
        doc.anchors.iter().any(|anchor| {
            anchor.id.as_str() == id.as_str()
                && subject_kind(doc, anchor.subject.as_str()) == Some("runtime-lib")
        })
    }) || feature.surfaces.iter().any(|id| {
        doc.surfaces.iter().any(|surface| {
            surface.id.as_str() == id.as_str()
                && (matches!(
                    surface.kind.as_str(),
                    "view" | "view-edit" | "model-exchange" | "site"
                ) || subject_kind(doc, surface.subject.as_str()) == Some("runtime-lib"))
        })
    }) || subject_kind(doc, feature.subject.as_str()) == Some("runtime-lib")
}

fn subject_kind<'a>(doc: &'a IndexDoc, id: &str) -> Option<&'a str> {
    doc.subjects
        .iter()
        .find(|subject| subject.id.as_str() == id)
        .map(|subject| subject.kind.as_str())
}

fn looks_framework(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("framework") || value.contains("runtime-lib") || value.contains("library")
}

fn page(title: &str) -> String {
    format!("# {title}\n\n{}\n\n", crate::index_render::GENERATED)
}

fn cell(value: &str) -> String {
    value.replace(['\n', '\r'], " ").replace('|', "\\|")
}
