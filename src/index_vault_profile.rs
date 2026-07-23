use std::collections::BTreeSet;

use sim_index_core::shape::is_index_id;

use crate::{
    index_vault_graph::{VaultEndpoint, VaultNodeKind},
    index_vault_link::{
        LinkTargetStyle, MetadataStyle, NoteRef, OutlineStyle, escape_markdown_label,
        escape_wikilink_label, id_from_note_filename, id_to_note_filename, logseq_property_value,
        yaml_property_value,
    },
};

pub(crate) const PROFILES: &[VaultProfile] = &[
    VaultProfile::portable_markdown_v1(),
    VaultProfile::obsidian_markdown_v1(),
    VaultProfile::seqlog_markdown_v1(),
    VaultProfile::logseq_file_v1(),
];

pub(crate) const PROFILE_ALIASES: &[VaultProfileAlias] = &[
    VaultProfileAlias {
        friendly_name: "portable",
        profile_id: "portable-markdown-v1",
    },
    VaultProfileAlias {
        friendly_name: "obsidian",
        profile_id: "obsidian-markdown-v1",
    },
    VaultProfileAlias {
        friendly_name: "seqlog",
        profile_id: "seqlog-markdown-v1",
    },
    VaultProfileAlias {
        friendly_name: "logseq",
        profile_id: "logseq-file-v1",
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VaultProfile {
    pub(crate) id: &'static str,
    pub(crate) friendly_name: &'static str,
    pub(crate) metadata: MetadataDescriptor,
    pub(crate) links: LinkDescriptor,
    pub(crate) outline: OutlineDescriptor,
}

impl VaultProfile {
    pub(crate) const fn portable_markdown_v1() -> Self {
        Self {
            id: "portable-markdown-v1",
            friendly_name: "portable",
            metadata: MetadataDescriptor {
                style: MetadataStyle::YamlProperties,
                id_property: "sim_id",
                profile_property: "sim_profile",
            },
            links: LinkDescriptor {
                style: LinkTargetStyle::RelativeCommonMark,
                root_namespace: "SIM-Index",
            },
            outline: OutlineDescriptor {
                style: OutlineStyle::CommonMarkHeadings,
                relation_heading: "Relations",
            },
        }
    }

    pub(crate) const fn obsidian_markdown_v1() -> Self {
        Self {
            id: "obsidian-markdown-v1",
            friendly_name: "obsidian",
            metadata: MetadataDescriptor {
                style: MetadataStyle::YamlProperties,
                id_property: "sim_id",
                profile_property: "sim_profile",
            },
            links: LinkDescriptor {
                style: LinkTargetStyle::VaultRootWikilink,
                root_namespace: "SIM-Index",
            },
            outline: OutlineDescriptor {
                style: OutlineStyle::CommonMarkHeadings,
                relation_heading: "Relations",
            },
        }
    }

    pub(crate) const fn seqlog_markdown_v1() -> Self {
        Self {
            id: "seqlog-markdown-v1",
            friendly_name: "seqlog",
            metadata: MetadataDescriptor {
                style: MetadataStyle::YamlProperties,
                id_property: "sim_id",
                profile_property: "sim_profile",
            },
            links: LinkDescriptor {
                style: LinkTargetStyle::RelativeCommonMark,
                root_namespace: "SIM-Index",
            },
            outline: OutlineDescriptor {
                style: OutlineStyle::CommonMarkHeadings,
                relation_heading: "Relations",
            },
        }
    }

    pub(crate) const fn logseq_file_v1() -> Self {
        Self {
            id: "logseq-file-v1",
            friendly_name: "logseq",
            metadata: MetadataDescriptor {
                style: MetadataStyle::LogseqProperties,
                id_property: "sim_id",
                profile_property: "sim_profile",
            },
            links: LinkDescriptor {
                style: LinkTargetStyle::VaultRootWikilink,
                root_namespace: "SIM-Index",
            },
            outline: OutlineDescriptor {
                style: OutlineStyle::IndentedBlocks,
                relation_heading: "Relations",
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VaultProfileAlias {
    pub(crate) friendly_name: &'static str,
    pub(crate) profile_id: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MetadataDescriptor {
    pub(crate) style: MetadataStyle,
    pub(crate) id_property: &'static str,
    pub(crate) profile_property: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LinkDescriptor {
    pub(crate) style: LinkTargetStyle,
    pub(crate) root_namespace: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutlineDescriptor {
    pub(crate) style: OutlineStyle,
    pub(crate) relation_heading: &'static str,
}

pub(crate) fn resolve_profile(name: &str) -> Result<&'static VaultProfile, String> {
    let id = PROFILE_ALIASES
        .iter()
        .find(|alias| alias.friendly_name == name)
        .map(|alias| alias.profile_id)
        .unwrap_or(name);
    PROFILES
        .iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| {
            format!(
                "unknown Index vault profile `{name}`; expected one of {}",
                profile_names().join(", ")
            )
        })
}

pub(crate) fn check_vault_profile_contracts() -> Result<(), String> {
    if PROFILES.len() != 4 || PROFILE_ALIASES.len() != 4 {
        return Err("Index vault profile table must contain four profiles and aliases".to_owned());
    }

    let mut ids = BTreeSet::new();
    let mut friendly_names = BTreeSet::new();
    for profile in PROFILES {
        if !ids.insert(profile.id) {
            return Err(format!("duplicate Index vault profile id `{}`", profile.id));
        }
        if !friendly_names.insert(profile.friendly_name) {
            return Err(format!(
                "duplicate Index vault profile friendly name `{}`",
                profile.friendly_name
            ));
        }
        if profile.metadata.id_property.is_empty()
            || profile.metadata.profile_property.is_empty()
            || profile.outline.relation_heading.is_empty()
            || profile.links.root_namespace.is_empty()
        {
            return Err(format!(
                "Index vault profile `{}` has an empty descriptor",
                profile.id
            ));
        }
    }
    for alias in PROFILE_ALIASES {
        let profile = resolve_profile(alias.friendly_name)?;
        if profile.id != alias.profile_id {
            return Err(format!(
                "Index vault profile alias `{}` resolved to `{}`, expected `{}`",
                alias.friendly_name, profile.id, alias.profile_id
            ));
        }
        if resolve_profile(alias.profile_id)?.friendly_name != alias.friendly_name {
            return Err(format!(
                "Index vault profile id `{}` does not round-trip to `{}`",
                alias.profile_id, alias.friendly_name
            ));
        }
    }

    let id = "feature/sim-tooling/generated-docs";
    let filename = id_to_note_filename(id)?;
    if id_from_note_filename(&filename)? != id {
        return Err("Index vault note filename transform is not reversible".to_owned());
    }
    if is_index_id("feature/sim-tooling~generated-docs") {
        return Err("Index id grammar unexpectedly accepts `~`".to_owned());
    }

    let source = NoteRef::new(VaultEndpoint::new(
        VaultNodeKind::Feature,
        "feature/sim-tooling/generated-docs",
    ))?;
    let target = NoteRef::new(VaultEndpoint::new(
        VaultNodeKind::Route,
        "route/add-generated-doc",
    ))?;
    if source.endpoint().id != "feature/sim-tooling/generated-docs" {
        return Err("Index vault note endpoint changed during construction".to_owned());
    }
    if !source.path_str().ends_with(".md") || source.path().is_absolute() {
        return Err("Index vault note path is not a relative Markdown file".to_owned());
    }
    if target.relative_commonmark_target_from(&source) != "../routes/route~add-generated-doc.md" {
        return Err("Index vault relative CommonMark target changed".to_owned());
    }
    if target.commonmark_link_from(&source, "Generated [docs]")
        != "[Generated \\[docs\\]](../routes/route~add-generated-doc.md)"
    {
        return Err("Index vault CommonMark link rendering changed".to_owned());
    }
    if target.vault_root_wikilink_target("SIM-Index") != "SIM-Index/routes/route~add-generated-doc"
    {
        return Err("Index vault wikilink target changed".to_owned());
    }
    if target.vault_root_wikilink("SIM-Index", "Route | docs")
        != "[[SIM-Index/routes/route~add-generated-doc|Route \\| docs]]"
    {
        return Err("Index vault wikilink rendering changed".to_owned());
    }
    if escape_markdown_label(r"a [b]\c") != r"a \[b\]\\c"
        || escape_wikilink_label(r"a | [b]\c") != r"a \| \[b\]\\c"
        || yaml_property_value("a\"b\nc") != "\"a\\\"b\\nc\""
        || logseq_property_value("a\\b\nc\td") != r"a\\b\nc\td"
    {
        return Err("Index vault escaping helpers changed".to_owned());
    }

    Ok(())
}

fn profile_names() -> Vec<&'static str> {
    let mut names = PROFILE_ALIASES
        .iter()
        .map(|alias| alias.friendly_name)
        .chain(PROFILES.iter().map(|profile| profile.id))
        .collect::<Vec<_>>();
    names.sort();
    names
}
