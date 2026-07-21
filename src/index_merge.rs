//! Merge generated SIM Index fragments into one checked constellation graph.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use sim_codec_index::{IndexCodec, IndexForm};
use sim_index_core::{
    AnchorId, DiscoveredAnchor, DiscoveredSpecimen, DiscoveredSurface, FeatureDraft, FeatureId,
    FeatureRecord, GrammarContract, IndexDoc, IndexEdge, RouteRecord, RouteStep, SpecimenId,
    SubjectId, SubjectRecord, SurfaceId, Visibility, check_index_doc,
};
use sim_kernel::EncodePosition;

const GENERATOR: &str = "xtask index merge v1";

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = MergeOptions::parse(&args)?;
    let doc = merge_fragment_paths(&options.fragments, options.public_only)?;
    let output = encode_sx(&doc)?;
    write_or_check(&options.out, &output, options.check)?;
    if options.check {
        println!("index merge: generated index.sx is current");
    } else {
        println!(
            "index merge: {} fragment(s), {} subject(s), {} feature(s)",
            options.fragments.len(),
            doc.subjects.len(),
            doc.features.len()
        );
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MergeOptions {
    fragments: Vec<PathBuf>,
    out: PathBuf,
    check: bool,
    public_only: bool,
}

impl MergeOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let program = args.first().map(String::as_str).unwrap_or("xtask");
        if !matches!(args.get(1).map(String::as_str), Some("index"))
            || !matches!(args.get(2).map(String::as_str), Some("merge"))
        {
            return Err(usage(program));
        }

        let mut fragments = Vec::new();
        let mut out = None;
        let mut check = false;
        let mut public_only = true;
        let mut index = 3;
        while index < args.len() {
            match args[index].as_str() {
                "--fragment" => {
                    index += 1;
                    fragments.push(PathBuf::from(
                        args.get(index).ok_or("--fragment requires a path")?,
                    ));
                }
                "--out" => {
                    index += 1;
                    out = Some(PathBuf::from(
                        args.get(index).ok_or("--out requires a path")?,
                    ));
                }
                "--check" => check = true,
                "--include-private" => public_only = false,
                "-h" | "--help" => return Err(usage(program)),
                other => {
                    return Err(format!(
                        "unknown index merge argument `{other}`; {}",
                        usage(program)
                    ));
                }
            }
            index += 1;
        }

        if fragments.is_empty() {
            return Err(format!(
                "index merge requires at least one --fragment; {}",
                usage(program)
            ));
        }
        Ok(Self {
            fragments,
            out: out.ok_or_else(|| format!("index merge requires --out; {}", usage(program)))?,
            check,
            public_only,
        })
    }
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} index merge --fragment <path>... --out <path> [--check] [--include-private]"
    )
}

pub(crate) fn merge_fragment_paths(
    fragment_paths: &[PathBuf],
    public_only: bool,
) -> Result<IndexDoc, String> {
    let mut fragments = Vec::new();
    for path in fragment_paths {
        let source =
            fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
        let doc = IndexCodec
            .decode(IndexForm::Sx, &source)
            .map_err(|err| format!("decode {}: {err}", path.display()))?;
        if public_only && doc.visibility != Visibility::Public {
            continue;
        }
        fragments.push(Fragment {
            repo: repo_name_from_fragment(path),
            doc,
        });
    }
    merge_fragments(&fragments)
}

fn merge_fragments(fragments: &[Fragment]) -> Result<IndexDoc, String> {
    if fragments.is_empty() {
        return Err("index merge found no public fragments".to_owned());
    }

    let counts = CollisionCounts::from_fragments(fragments);
    let mut merged = IndexDoc::public(GENERATOR);
    if fragments
        .iter()
        .any(|fragment| fragment.doc.visibility == Visibility::PrivateLocal)
    {
        merged.visibility = Visibility::PrivateLocal;
    }

    for fragment in fragments {
        let maps = IdMaps::for_fragment(fragment, &counts);
        let mut doc = rewrite_doc(&fragment.doc, &maps);
        merged.subjects.append(&mut doc.subjects);
        merged.anchors.append(&mut doc.anchors);
        merged.surfaces.append(&mut doc.surfaces);
        merged.specimens.append(&mut doc.specimens);
        merged.drafts.append(&mut doc.drafts);
        merged.features.append(&mut doc.features);
        merged.routes.append(&mut doc.routes);
        merged.edges.append(&mut doc.edges);
    }

    sort_doc(&mut merged);
    check_index_doc(&merged).map_err(|err| format!("merged index is invalid: {err}"))?;
    Ok(merged)
}

fn encode_sx(doc: &IndexDoc) -> Result<String, String> {
    IndexCodec
        .encode(doc, EncodePosition::Data, IndexForm::Sx)
        .map_err(|err| format!("encode index.sx: {err}"))
}

fn write_or_check(path: &Path, expected: &str, check: bool) -> Result<(), String> {
    if check {
        let current =
            fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
        if current == expected {
            return Ok(());
        }
        return Err(format!(
            "stale generated index artifact: {}; run `sh bin/simctl index`",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;
    }
    fs::write(path, expected).map_err(|err| format!("write {}: {err}", path.display()))
}

#[derive(Clone, Debug)]
struct Fragment {
    repo: String,
    doc: IndexDoc,
}

#[derive(Clone, Debug, Default)]
struct CollisionCounts {
    subjects: BTreeMap<String, usize>,
    anchors: BTreeMap<String, usize>,
    surfaces: BTreeMap<String, usize>,
    specimens: BTreeMap<String, usize>,
    drafts: BTreeMap<String, usize>,
    features: BTreeMap<String, usize>,
    routes: BTreeMap<String, usize>,
}

impl CollisionCounts {
    fn from_fragments(fragments: &[Fragment]) -> Self {
        let mut counts = Self::default();
        for fragment in fragments {
            for subject in &fragment.doc.subjects {
                bump(&mut counts.subjects, subject.id.as_str());
            }
            for anchor in &fragment.doc.anchors {
                bump(&mut counts.anchors, anchor.id.as_str());
            }
            for surface in &fragment.doc.surfaces {
                bump(&mut counts.surfaces, surface.id.as_str());
            }
            for specimen in &fragment.doc.specimens {
                bump(&mut counts.specimens, specimen.id.as_str());
            }
            for draft in &fragment.doc.drafts {
                bump(&mut counts.drafts, draft.id.as_str());
            }
            for feature in &fragment.doc.features {
                bump(&mut counts.features, feature.id.as_str());
            }
            for route in &fragment.doc.routes {
                bump(&mut counts.routes, route.id.as_str());
            }
        }
        counts
    }
}

fn bump(counts: &mut BTreeMap<String, usize>, id: &str) {
    *counts.entry(id.to_owned()).or_insert(0) += 1;
}

#[derive(Clone, Debug)]
struct IdMaps {
    subjects: BTreeMap<String, String>,
    anchors: BTreeMap<String, String>,
    surfaces: BTreeMap<String, String>,
    specimens: BTreeMap<String, String>,
    drafts: BTreeMap<String, String>,
    features: BTreeMap<String, String>,
    routes: BTreeMap<String, String>,
}

impl IdMaps {
    fn for_fragment(fragment: &Fragment, counts: &CollisionCounts) -> Self {
        Self {
            subjects: map_ids(
                &fragment.repo,
                &counts.subjects,
                fragment
                    .doc
                    .subjects
                    .iter()
                    .map(|record| record.id.as_str()),
            ),
            anchors: map_ids(
                &fragment.repo,
                &counts.anchors,
                fragment.doc.anchors.iter().map(|record| record.id.as_str()),
            ),
            surfaces: map_ids(
                &fragment.repo,
                &counts.surfaces,
                fragment
                    .doc
                    .surfaces
                    .iter()
                    .map(|record| record.id.as_str()),
            ),
            specimens: map_ids(
                &fragment.repo,
                &counts.specimens,
                fragment
                    .doc
                    .specimens
                    .iter()
                    .map(|record| record.id.as_str()),
            ),
            drafts: map_ids(
                &fragment.repo,
                &counts.drafts,
                fragment.doc.drafts.iter().map(|record| record.id.as_str()),
            ),
            features: map_ids(
                &fragment.repo,
                &counts.features,
                fragment
                    .doc
                    .features
                    .iter()
                    .map(|record| record.id.as_str()),
            ),
            routes: map_ids(
                &fragment.repo,
                &counts.routes,
                fragment.doc.routes.iter().map(|record| record.id.as_str()),
            ),
        }
    }

    fn map_any(&self, id: &str) -> String {
        for map in [
            &self.subjects,
            &self.anchors,
            &self.surfaces,
            &self.specimens,
            &self.features,
            &self.routes,
        ] {
            if let Some(mapped) = map.get(id) {
                return mapped.clone();
            }
        }
        id.to_owned()
    }
}

fn map_ids<'a>(
    repo: &str,
    counts: &BTreeMap<String, usize>,
    ids: impl Iterator<Item = &'a str>,
) -> BTreeMap<String, String> {
    ids.map(|id| {
        let mapped = if counts.get(id).copied().unwrap_or(0) > 1 {
            format!("local/{repo}/{id}")
        } else {
            id.to_owned()
        };
        (id.to_owned(), mapped)
    })
    .collect()
}

fn rewrite_doc(doc: &IndexDoc, maps: &IdMaps) -> IndexDoc {
    let mut out = doc.clone();
    out.subjects = doc
        .subjects
        .iter()
        .map(|record| SubjectRecord {
            id: SubjectId::new(map(&maps.subjects, record.id.as_str())),
            kind: record.kind.clone(),
            title: record.title.clone(),
        })
        .collect();
    out.anchors = doc
        .anchors
        .iter()
        .map(|record| DiscoveredAnchor {
            id: AnchorId::new(map(&maps.anchors, record.id.as_str())),
            subject: SubjectId::new(map(&maps.subjects, record.subject.as_str())),
            kind: record.kind.clone(),
        })
        .collect();
    out.surfaces = doc
        .surfaces
        .iter()
        .map(|record| DiscoveredSurface {
            id: SurfaceId::new(map(&maps.surfaces, record.id.as_str())),
            subject: SubjectId::new(map(&maps.subjects, record.subject.as_str())),
            kind: record.kind.clone(),
        })
        .collect();
    out.specimens = doc
        .specimens
        .iter()
        .map(|record| DiscoveredSpecimen {
            id: SpecimenId::new(map(&maps.specimens, record.id.as_str())),
            subject: SubjectId::new(map(&maps.subjects, record.subject.as_str())),
            kind: record.kind.clone(),
            path: record.path.clone(),
            language: record.language.clone(),
            runnable: record.runnable,
            checked: record.checked,
            checked_by: record.checked_by.clone(),
            doc_anchor: record
                .doc_anchor
                .as_ref()
                .map(|id| AnchorId::new(map(&maps.anchors, id.as_str()))),
        })
        .collect();
    out.drafts = doc
        .drafts
        .iter()
        .map(|record| rewrite_draft(record, maps))
        .collect();
    out.features = doc
        .features
        .iter()
        .map(|record| rewrite_feature(record, maps))
        .collect();
    out.routes = doc
        .routes
        .iter()
        .map(|record| rewrite_route(record, maps))
        .collect();
    out.edges = doc
        .edges
        .iter()
        .map(|record| rewrite_edge(record, maps))
        .collect();
    out
}

fn rewrite_draft(record: &FeatureDraft, maps: &IdMaps) -> FeatureDraft {
    let mut out = record.clone();
    out.id = FeatureId::new(map(&maps.drafts, record.id.as_str()));
    out.subject = SubjectId::new(map(&maps.subjects, record.subject.as_str()));
    out.claims_anchors = record
        .claims_anchors
        .iter()
        .map(|id| AnchorId::new(map(&maps.anchors, id.as_str())))
        .collect();
    out.claims_surfaces = record
        .claims_surfaces
        .iter()
        .map(|id| SurfaceId::new(map(&maps.surfaces, id.as_str())))
        .collect();
    out.claims_specimens = record
        .claims_specimens
        .iter()
        .map(|id| SpecimenId::new(map(&maps.specimens, id.as_str())))
        .collect();
    out.grammar_contracts = record
        .grammar_contracts
        .iter()
        .map(|contract| rewrite_grammar(contract, maps))
        .collect();
    out.doc_anchor = record
        .doc_anchor
        .as_ref()
        .map(|id| AnchorId::new(map(&maps.anchors, id.as_str())));
    out
}

fn rewrite_feature(record: &FeatureRecord, maps: &IdMaps) -> FeatureRecord {
    let mut out = record.clone();
    out.id = FeatureId::new(map(&maps.features, record.id.as_str()));
    out.subject = SubjectId::new(map(&maps.subjects, record.subject.as_str()));
    out.anchors = record
        .anchors
        .iter()
        .map(|id| AnchorId::new(map(&maps.anchors, id.as_str())))
        .collect();
    out.surfaces = record
        .surfaces
        .iter()
        .map(|id| SurfaceId::new(map(&maps.surfaces, id.as_str())))
        .collect();
    out.specimens = record
        .specimens
        .iter()
        .map(|id| SpecimenId::new(map(&maps.specimens, id.as_str())))
        .collect();
    out.grammar_contracts = record
        .grammar_contracts
        .iter()
        .map(|contract| rewrite_grammar(contract, maps))
        .collect();
    out.doc_anchor = record
        .doc_anchor
        .as_ref()
        .map(|id| AnchorId::new(map(&maps.anchors, id.as_str())));
    out
}

fn rewrite_grammar(record: &GrammarContract, maps: &IdMaps) -> GrammarContract {
    GrammarContract {
        id: record.id.clone(),
        decoder: record
            .decoder
            .as_ref()
            .map(|id| AnchorId::new(map(&maps.anchors, id.as_str()))),
        encoder: record
            .encoder
            .as_ref()
            .map(|id| AnchorId::new(map(&maps.anchors, id.as_str()))),
        surface: record
            .surface
            .as_ref()
            .map(|id| SurfaceId::new(map(&maps.surfaces, id.as_str()))),
        round_trip: record.round_trip,
    }
}

fn rewrite_route(record: &RouteRecord, maps: &IdMaps) -> RouteRecord {
    RouteRecord {
        id: RouteId::new(map(&maps.routes, record.id.as_str())),
        title: record.title.clone(),
        audiences: record.audiences.clone(),
        steps: record
            .steps
            .iter()
            .map(|step| match step {
                RouteStep::Feature { id, why } => RouteStep::Feature {
                    id: FeatureId::new(map(&maps.features, id.as_str())),
                    why: why.clone(),
                },
                RouteStep::Specimen { id, why } => RouteStep::Specimen {
                    id: SpecimenId::new(map(&maps.specimens, id.as_str())),
                    why: why.clone(),
                },
            })
            .collect(),
        doc_anchor: record
            .doc_anchor
            .as_ref()
            .map(|id| AnchorId::new(map(&maps.anchors, id.as_str()))),
    }
}

use sim_index_core::RouteId;

fn rewrite_edge(edge: &IndexEdge, maps: &IdMaps) -> IndexEdge {
    match edge.rel.as_str() {
        "contains" => IndexEdge::new(
            map(&maps.subjects, &edge.from),
            edge.rel.clone(),
            map(&maps.subjects, &edge.to),
        ),
        "anchors" => IndexEdge::new(
            map(&maps.features, &edge.from),
            edge.rel.clone(),
            map(&maps.anchors, &edge.to),
        ),
        "surfaces" => IndexEdge::new(
            map(&maps.features, &edge.from),
            edge.rel.clone(),
            map(&maps.surfaces, &edge.to),
        ),
        "demonstrates" => IndexEdge::new(
            map(&maps.features, &edge.from),
            edge.rel.clone(),
            map(&maps.specimens, &edge.to),
        ),
        "supports" | "presents" | "replaces" => IndexEdge::new(
            map(&maps.features, &edge.from),
            edge.rel.clone(),
            map(&maps.features, &edge.to),
        ),
        "routes" => IndexEdge::new(
            map(&maps.routes, &edge.from),
            edge.rel.clone(),
            maps.map_any(&edge.to),
        ),
        _ => IndexEdge::new(
            maps.map_any(&edge.from),
            edge.rel.clone(),
            maps.map_any(&edge.to),
        ),
    }
}

fn map(ids: &BTreeMap<String, String>, id: &str) -> String {
    ids.get(id).cloned().unwrap_or_else(|| id.to_owned())
}

fn sort_doc(doc: &mut IndexDoc) {
    doc.subjects.sort_by(|left, right| left.id.cmp(&right.id));
    doc.anchors.sort_by(|left, right| left.id.cmp(&right.id));
    doc.surfaces.sort_by(|left, right| left.id.cmp(&right.id));
    doc.specimens.sort_by(|left, right| left.id.cmp(&right.id));
    doc.drafts.sort_by(|left, right| left.id.cmp(&right.id));
    doc.features.sort_by(|left, right| left.id.cmp(&right.id));
    doc.routes.sort_by(|left, right| left.id.cmp(&right.id));
    for draft in &mut doc.drafts {
        draft.claims_anchors.sort();
        draft.claims_surfaces.sort();
        draft.claims_specimens.sort();
        draft
            .grammar_contracts
            .sort_by(|left, right| left.id.cmp(&right.id));
    }
    for feature in &mut doc.features {
        feature.anchors.sort();
        feature.surfaces.sort();
        feature.specimens.sort();
        feature
            .grammar_contracts
            .sort_by(|left, right| left.id.cmp(&right.id));
    }
    doc.edges.sort_by(|left, right| {
        (&left.from, &left.rel, &left.to).cmp(&(&right.from, &right.rel, &right.to))
    });
    doc.edges.dedup();
}

fn repo_name_from_fragment(path: &Path) -> String {
    let repo = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .or_else(|| path.file_stem().and_then(|name| name.to_str()))
        .unwrap_or("fragment");
    id_segment(repo)
}

fn id_segment(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        let normalized = match ch {
            'A'..='Z' => ch.to_ascii_lowercase(),
            'a'..='z' | '0'..='9' | '_' | '.' => ch,
            '-' | '/' | ' ' => '-',
            _ => '-',
        };
        if normalized == '-' {
            if !last_dash && !out.is_empty() {
                out.push('-');
            }
            last_dash = true;
        } else {
            out.push(normalized);
            last_dash = false;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("fragment");
    }
    out
}

#[cfg(test)]
#[path = "index_merge_tests.rs"]
mod tests;
