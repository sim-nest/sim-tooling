use sim_index_core::shape::is_index_id;

use crate::{
    index_vault_graph::{VaultEndpoint, VaultNodeKind},
    index_vault_link::{
        LinkTargetStyle, MetadataStyle, NoteRef, OutlineStyle, escape_markdown_label,
        escape_wikilink_label, id_from_note_filename, id_to_note_filename, logseq_property_value,
        yaml_property_value,
    },
    index_vault_profile::{PROFILE_ALIASES, PROFILES, resolve_profile},
};

// conformance: vault profiles resolve explicit versions and derive safe note links.

#[test]
fn profile_table_maps_friendly_names_to_versioned_ids() {
    let aliases = PROFILE_ALIASES
        .iter()
        .map(|alias| (alias.friendly_name, alias.profile_id))
        .collect::<Vec<_>>();
    assert_eq!(
        aliases,
        [
            ("portable", "portable-markdown-v1"),
            ("obsidian", "obsidian-markdown-v1"),
            ("seqlog", "seqlog-markdown-v1"),
            ("logseq", "logseq-file-v1"),
        ]
    );
    assert_eq!(PROFILES.len(), 4);
    for alias in aliases {
        assert_eq!(resolve_profile(alias.0).unwrap().id, alias.1);
        assert_eq!(resolve_profile(alias.1).unwrap().id, alias.1);
    }
}

#[test]
fn profile_choices_are_descriptors() {
    let portable = resolve_profile("portable").unwrap();
    assert_eq!(portable.friendly_name, "portable");
    assert_eq!(portable.metadata.style, MetadataStyle::YamlProperties);
    assert_eq!(portable.links.style, LinkTargetStyle::RelativeCommonMark);
    assert_eq!(portable.outline.style, OutlineStyle::CommonMarkHeadings);

    let logseq = resolve_profile("logseq").unwrap();
    assert_eq!(logseq.metadata.style, MetadataStyle::LogseqProperties);
    assert_eq!(logseq.links.style, LinkTargetStyle::VaultRootWikilink);
    assert_eq!(logseq.outline.style, OutlineStyle::IndentedBlocks);
}

#[test]
fn unknown_profiles_are_rejected_without_fallback() {
    let err = resolve_profile("obsidian-vlatest").unwrap_err();

    assert!(err.contains("unknown Index vault profile"));
    assert!(err.contains("portable-markdown-v1"));
}

#[test]
fn slash_to_tilde_note_filenames_round_trip_index_ids() {
    let id = "feature/sim-runtime/incremental-query-core";
    let filename = id_to_note_filename(id).unwrap();

    assert_eq!(filename, "feature~sim-runtime~incremental-query-core.md");
    assert_eq!(id_from_note_filename(&filename).unwrap(), id);
    assert!(!is_index_id("feature/sim-runtime~incremental-query-core"));
    assert!(id_to_note_filename("feature/sim-runtime~incremental-query-core").is_err());
    assert!(id_from_note_filename("../feature~demo.md").is_err());
    assert!(id_from_note_filename("feature~demo.txt").is_err());
}

#[test]
fn note_refs_render_relative_commonmark_links() {
    let source = note(VaultNodeKind::Feature, "feature/source");
    let target = note(VaultNodeKind::Specimen, "spec-test/demo/source");
    let sibling = note(VaultNodeKind::Feature, "feature/other");

    assert_eq!(
        target.relative_commonmark_target_from(&source),
        "../specimens/spec-test~demo~source.md"
    );
    assert_eq!(
        sibling.relative_commonmark_target_from(&source),
        "feature~other.md"
    );
    assert_eq!(
        target.commonmark_link_from(&source, "Spec [checked]"),
        "[Spec \\[checked\\]](../specimens/spec-test~demo~source.md)"
    );
}

#[test]
fn note_refs_render_vault_root_wikilink_targets() {
    let target = note(VaultNodeKind::Route, "route/add-generated-doc");

    assert_eq!(
        target.vault_root_wikilink_target("SIM-Index"),
        "SIM-Index/routes/route~add-generated-doc"
    );
    assert_eq!(
        target.vault_root_wikilink("/SIM-Index/", "Route | docs"),
        "[[SIM-Index/routes/route~add-generated-doc|Route \\| docs]]"
    );
}

#[test]
fn escaping_helpers_cover_markdown_yaml_and_logseq_values() {
    assert_eq!(escape_markdown_label(r"a [b]\c"), r"a \[b\]\\c");
    assert_eq!(escape_wikilink_label(r"a | [b]\c"), r"a \| \[b\]\\c");
    assert_eq!(yaml_property_value("a\"b\nc"), "\"a\\\"b\\nc\"");
    assert_eq!(logseq_property_value("a\\b\nc\td"), r"a\\b\nc\td");
}

fn note(kind: VaultNodeKind, id: &str) -> NoteRef {
    let note = NoteRef::new(VaultEndpoint::new(kind, id)).unwrap();
    assert_eq!(note.endpoint().id, id);
    assert!(note.path().to_str().unwrap().ends_with(".md"));
    note
}
