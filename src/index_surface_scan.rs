//! Discovery of addressable surfaces for SIM Index fragments.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use sim_index_core::{
    AnchorId, DiscoveredAnchor, DiscoveredSurface, FeatureDraft, GrammarContract, SpecimenId,
    SubjectId, SubjectRecord, SurfaceId,
};

use crate::{
    index_anchor_scan::{is_simple_symbol_tail, non_test_source_text, quoted_values},
    index_fragment::{
        codec_language, insert_subject, is_test_source, package_rust_files, rel_path, repo_name,
        slug_path, subject_id,
    },
    repo_contract::PackageContract,
};

/// Surface, grammar, and subject facts discovered for a repo fragment.
#[derive(Default)]
pub(crate) struct DiscoveredFacts {
    pub(crate) subjects: Vec<SubjectRecord>,
    pub(crate) anchors: Vec<DiscoveredAnchor>,
    pub(crate) surfaces: Vec<DiscoveredSurface>,
    pub(crate) drafts: Vec<FeatureDraft>,
}

/// Discovers surfaces and grammar contracts from the existing source tree.
pub(crate) fn discovered(
    repo: &Path,
    packages: &[PackageContract],
    _anchors: &[DiscoveredAnchor],
) -> DiscoveredFacts {
    let repo_subject = subject_id("repo", &repo_name(repo));
    let doc_set_subject = subject_id("doc-set", &format!("{}/generated", repo_name(repo)));
    let mut subjects = BTreeMap::new();
    let mut surface_rows = BTreeMap::new();
    let mut grammar_anchors = BTreeMap::new();
    let mut drafts = BTreeMap::new();

    insert_surface(
        &mut surface_rows,
        "docs",
        &format!("{}/generated", repo_name(repo)),
        &doc_set_subject,
        "docs",
    );

    for package in packages {
        let crate_subject = subject_id("crate", &package.name);
        discover_cli_surfaces(repo, package, &crate_subject, &mut surface_rows);
        discover_view_surfaces(repo, package, &crate_subject, &mut surface_rows);
        discover_site_surfaces(repo, package, &crate_subject, &mut surface_rows);
        discover_model_surfaces(package, &crate_subject, &mut surface_rows);
        discover_codec_surfaces(
            package,
            &mut subjects,
            &mut surface_rows,
            &mut grammar_anchors,
            &mut drafts,
        );
    }

    if repo_name(repo) == "sim-private" {
        insert_surface(&mut surface_rows, "cli", "simctl", &repo_subject, "cli");
    }

    DiscoveredFacts {
        subjects: subjects.into_values().collect(),
        anchors: grammar_anchors.into_values().collect(),
        surfaces: surface_rows.into_values().collect(),
        drafts: drafts.into_values().collect(),
    }
}

fn discover_cli_surfaces(
    repo: &Path,
    package: &PackageContract,
    subject: &SubjectId,
    surfaces: &mut BTreeMap<String, DiscoveredSurface>,
) {
    if package.target_kinds.iter().any(|kind| kind == "bin") {
        insert_surface(surfaces, "cli", &package.name, subject, "cli");
    }
    for verb in cli_verbs(repo, package) {
        insert_surface(surfaces, "cli", &verb, subject, "cli");
    }
}

fn discover_view_surfaces(
    repo: &Path,
    package: &PackageContract,
    subject: &SubjectId,
    surfaces: &mut BTreeMap<String, DiscoveredSurface>,
) {
    if package.name.contains("-view") || package_sources_contain(repo, package, "SurfaceCaps") {
        insert_surface(surfaces, "view", &package.name, subject, "view");
    }
    if package_sources_contain(repo, package, "UNIVERSAL_EDITOR_ID")
        || package_sources_contain(repo, package, "SurfaceCodec")
    {
        insert_surface(surfaces, "view-edit", &package.name, subject, "view-edit");
    }
}

fn discover_site_surfaces(
    repo: &Path,
    package: &PackageContract,
    subject: &SubjectId,
    surfaces: &mut BTreeMap<String, DiscoveredSurface>,
) {
    if package_sources_contain(repo, package, "Export::Site")
        || package_sources_contain(repo, package, "Export::SiteRecord")
        || package_sources_contain(repo, package, "ExportRecord::site")
    {
        insert_surface(surfaces, "site", &package.name, subject, "site");
    }
    for preset in surface_presets(repo, package) {
        insert_surface(surfaces, "site-device", &preset, subject, "site-device");
    }
}

fn discover_model_surfaces(
    package: &PackageContract,
    subject: &SubjectId,
    surfaces: &mut BTreeMap<String, DiscoveredSurface>,
) {
    let name = package.name.as_str();
    if name.contains("agent")
        || name.contains("openai")
        || name.contains("anthropic")
        || name.contains("ollama")
        || name.contains("model")
    {
        insert_surface(surfaces, "model", name, subject, "model-exchange");
    }
}

fn discover_codec_surfaces(
    package: &PackageContract,
    subjects: &mut BTreeMap<String, SubjectRecord>,
    surfaces: &mut BTreeMap<String, DiscoveredSurface>,
    anchors: &mut BTreeMap<String, DiscoveredAnchor>,
    drafts: &mut BTreeMap<String, FeatureDraft>,
) {
    let Some(language) = codec_language(package) else {
        return;
    };
    let language_subject = subject_id("language", &language);
    let grammar_subject = subject_id("grammar", &language);
    insert_subject(subjects, language_subject.clone(), "language", &language);
    insert_subject(
        subjects,
        grammar_subject,
        "grammar",
        &format!("{language} grammar"),
    );

    insert_grammar_surface(
        "syntax",
        &language,
        &language_subject,
        surfaces,
        anchors,
        drafts,
    );
    if is_wire_codec(&language) {
        insert_grammar_surface(
            "wire",
            &language,
            &language_subject,
            surfaces,
            anchors,
            drafts,
        );
    }
}

fn insert_grammar_surface(
    kind: &str,
    language: &str,
    subject: &SubjectId,
    surfaces: &mut BTreeMap<String, DiscoveredSurface>,
    anchors: &mut BTreeMap<String, DiscoveredAnchor>,
    drafts: &mut BTreeMap<String, FeatureDraft>,
) {
    let surface_id = insert_surface(surfaces, kind, language, subject, kind);
    let anchor_id = AnchorId::new(format!("anchor/grammar/{kind}/{}", slug_path(language)));
    anchors
        .entry(anchor_id.to_string())
        .or_insert_with(|| DiscoveredAnchor {
            id: anchor_id.clone(),
            subject: subject.clone(),
            kind: "grammar-contract".to_owned(),
        });
    let draft_id = format!("feature/{kind}/{}", slug_path(language));
    drafts
        .entry(draft_id.clone())
        .or_insert_with(|| FeatureDraft {
            id: sim_index_core::FeatureId::new(draft_id),
            subject: subject.clone(),
            title: format!("{language} {kind} surface"),
            summary: format!("{language} {kind} grammar surface discovered from codec package"),
            claims_anchors: vec![anchor_id.clone()],
            claims_surfaces: vec![surface_id.clone()],
            claims_specimens: Vec::<SpecimenId>::new(),
            literal_anchors: Vec::new(),
            literal_surfaces: Vec::new(),
            literal_specimens: Vec::new(),
            grammar_contracts: vec![GrammarContract {
                id: format!("grammar/{kind}/{}", slug_path(language)),
                decoder: Some(anchor_id.clone()),
                encoder: Some(anchor_id),
                surface: Some(surface_id),
                round_trip: true,
            }],
            doc_anchor: None,
        });
}

fn insert_surface(
    surfaces: &mut BTreeMap<String, DiscoveredSurface>,
    prefix: &str,
    tail: &str,
    subject: &SubjectId,
    kind: &str,
) -> SurfaceId {
    let id = SurfaceId::new(format!("{prefix}/{}", slug_path(tail)));
    surfaces
        .entry(id.to_string())
        .or_insert_with(|| DiscoveredSurface {
            id: id.clone(),
            subject: subject.clone(),
            kind: kind.to_owned(),
        });
    id
}

fn is_wire_codec(language: &str) -> bool {
    matches!(
        language,
        "binary"
            | "binary-base64"
            | "bitwise"
            | "bitwise-base64"
            | "bridge"
            | "chat"
            | "config"
            | "doc"
            | "index"
            | "mcp"
    )
}

fn cli_verbs(repo: &Path, package: &PackageContract) -> BTreeSet<String> {
    let mut verbs = BTreeSet::new();
    for path in package_rust_files(repo, package) {
        let rel = rel_path(repo, &path);
        if is_test_source(&rel) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let text = non_test_source_text(&text);
        for symbol in quoted_values(&text) {
            if let Some(verb) = symbol.strip_prefix("cli/main/") {
                if !is_simple_symbol_tail(verb) {
                    continue;
                }
                let verb = slug_path(verb);
                if !verb.is_empty() {
                    verbs.insert(verb);
                }
            }
        }
    }
    verbs
}

fn surface_presets(repo: &Path, package: &PackageContract) -> BTreeSet<String> {
    let mut presets = BTreeSet::new();
    for path in package_rust_files(repo, package) {
        let rel = rel_path(repo, &path);
        if is_test_source(&rel) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let text = non_test_source_text(&text);
        if !text.contains("SurfaceCaps::from_preset") && !text.contains("surface::preset") {
            continue;
        }
        for value in quoted_values(&text) {
            if is_surface_preset(&value) {
                presets.insert(slug_path(&value));
            }
        }
    }
    presets
}

fn is_surface_preset(value: &str) -> bool {
    matches!(
        value,
        "desktop"
            | "phone"
            | "watch"
            | "watch-glance-large"
            | "watch-sport"
            | "watch-sleep"
            | "glasses"
            | "glasses-hud"
            | "glasses-hud-camera"
            | "glasses-3dof"
            | "glasses-stereo"
            | "glasses-luma-ultra"
    )
}

fn package_sources_contain(repo: &Path, package: &PackageContract, needle: &str) -> bool {
    package_rust_files(repo, package).into_iter().any(|path| {
        let rel = rel_path(repo, &path);
        if is_test_source(&rel) {
            return false;
        }
        fs::read_to_string(path)
            .map(|text| non_test_source_text(&text).contains(needle))
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        env, fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn index_surface_scan_discovers_required_surface_families() {
        let root = temp_root("sim-tooling-surface-scan");
        fs::create_dir_all(root.join("crates/sim-codec-demo/src")).unwrap();
        fs::write(
            root.join("crates/sim-codec-demo/src/lib.rs"),
            "pub const CLI: &str = \"cli/main/demo\";\n\
             pub const PRESET: &str = \"watch\";\n\
             pub fn surface() { let _ = \"SurfaceCaps::from_preset\"; }\n",
        )
        .unwrap();
        let packages = vec![
            package("sim-codec-demo", "crates/sim-codec-demo"),
            package("sim-lib-agent-openai", ""),
        ];
        let anchors = Vec::new();

        let facts = discovered(&root, &packages, &anchors);
        let surfaces = facts
            .surfaces
            .iter()
            .map(|surface| (surface.id.as_str(), surface.kind.as_str()))
            .collect::<BTreeSet<_>>();
        let subjects = facts
            .subjects
            .iter()
            .map(|subject| subject.id.as_str())
            .collect::<BTreeSet<_>>();
        let docs_surface = format!(
            "docs/{}/generated",
            root.file_name().unwrap().to_string_lossy()
        );

        assert!(surfaces.contains(&(docs_surface.as_str(), "docs")));
        assert!(surfaces.contains(&("cli/demo", "cli")));
        assert!(surfaces.contains(&("syntax/demo", "syntax")));
        assert!(surfaces.contains(&("model/sim-lib-agent-openai", "model-exchange")));
        assert!(subjects.contains("language/demo"));
        assert!(subjects.contains("grammar/demo"));
        assert_eq!(facts.drafts.len(), 1);
        assert_eq!(
            facts.drafts[0].grammar_contracts[0]
                .surface
                .as_ref()
                .unwrap()
                .as_str(),
            "syntax/demo"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn syntax_and_wire_drafts_do_not_share_claims() {
        let root = temp_root("sim-tooling-surface-scan-wire");
        fs::create_dir_all(root.join("crates/sim-codec-binary/src")).unwrap();
        fs::write(root.join("crates/sim-codec-binary/src/lib.rs"), "").unwrap();
        let packages = vec![package("sim-codec-binary", "crates/sim-codec-binary")];
        let anchors = vec![DiscoveredAnchor {
            id: AnchorId::new("anchor/card/cookbook/codec/binary"),
            subject: SubjectId::new("crate/sim-codec-binary"),
            kind: "cookbook-recipe".to_owned(),
        }];

        let facts = discovered(&root, &packages, &anchors);
        let mut claim_counts = BTreeMap::<String, usize>::new();
        for draft in &facts.drafts {
            for anchor in &draft.claims_anchors {
                *claim_counts
                    .entry(format!("anchor:{}", anchor.as_str()))
                    .or_default() += 1;
            }
            for surface in &draft.claims_surfaces {
                *claim_counts
                    .entry(format!("surface:{}", surface.as_str()))
                    .or_default() += 1;
            }
        }

        assert_eq!(facts.drafts.len(), 2);
        assert!(
            !claim_counts.contains_key("anchor:anchor/card/cookbook/codec/binary"),
            "generated grammar drafts should not claim unrelated discovered anchors"
        );
        assert!(
            claim_counts.values().all(|count| *count == 1),
            "syntax and wire drafts should have distinct claims: {claim_counts:?}"
        );

        fs::remove_dir_all(root).unwrap();
    }

    fn package(name: &str, root: &str) -> PackageContract {
        PackageContract {
            name: name.to_owned(),
            crate_name: name.replace('-', "_"),
            manifest: if root.is_empty() {
                "Cargo.toml".to_owned()
            } else {
                format!("{root}/Cargo.toml")
            },
            root: root.to_owned(),
            group: "workspace".to_owned(),
            publish: "false".to_owned(),
            description: format!("{name} package"),
            target_kinds: vec!["lib".to_owned()],
            targets: Vec::new(),
            dependencies: Vec::new(),
            features: Vec::new(),
            rustdoc_summary: format!("{name} docs"),
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("{name}-{}-{stamp}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
}
