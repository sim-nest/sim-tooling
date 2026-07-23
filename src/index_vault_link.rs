use std::path::{Path, PathBuf};

use sim_codec_json::json_escape;
use sim_index_core::shape::is_index_id;

use crate::index_vault_graph::{VaultEndpoint, VaultNodeKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MetadataStyle {
    YamlProperties,
    LogseqProperties,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LinkTargetStyle {
    RelativeCommonMark,
    VaultRootWikilink,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutlineStyle {
    CommonMarkHeadings,
    IndentedBlocks,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NoteRef {
    endpoint: VaultEndpoint,
    path: PathBuf,
}

impl NoteRef {
    pub(crate) fn new(endpoint: VaultEndpoint) -> Result<Self, String> {
        let path =
            PathBuf::from(note_directory(endpoint.kind)).join(id_to_note_filename(&endpoint.id)?);
        Ok(Self { endpoint, path })
    }

    pub(crate) fn endpoint(&self) -> &VaultEndpoint {
        &self.endpoint
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn path_str(&self) -> &str {
        self.path
            .to_str()
            .expect("vault note paths are generated as UTF-8")
    }

    pub(crate) fn relative_commonmark_target_from(&self, source: &NoteRef) -> String {
        relative_path(
            source.path.parent().unwrap_or_else(|| Path::new("")),
            &self.path,
        )
    }

    pub(crate) fn commonmark_link_from(&self, source: &NoteRef, label: &str) -> String {
        format!(
            "[{}]({})",
            escape_markdown_label(label),
            self.relative_commonmark_target_from(source)
        )
    }

    pub(crate) fn vault_root_wikilink_target(&self, root_namespace: &str) -> String {
        let target = self.path.with_extension("");
        let target = target
            .to_str()
            .expect("vault note paths are generated as UTF-8");
        format!("{}/{}", root_namespace.trim_matches('/'), target)
    }

    pub(crate) fn vault_root_wikilink(&self, root_namespace: &str, label: &str) -> String {
        format!(
            "[[{}|{}]]",
            self.vault_root_wikilink_target(root_namespace),
            escape_wikilink_label(label)
        )
    }
}

pub(crate) fn id_to_note_filename(id: &str) -> Result<String, String> {
    if !is_index_id(id) {
        return Err(format!("vault note id is not a valid Index id: `{id}`"));
    }
    if id.contains('~') {
        return Err(format!("vault note id cannot contain `~`: `{id}`"));
    }
    Ok(format!("{}.md", id.replace('/', "~")))
}

pub(crate) fn id_from_note_filename(filename: &str) -> Result<String, String> {
    let Some(stem) = filename.strip_suffix(".md") else {
        return Err(format!("vault note filename must end in .md: `{filename}`"));
    };
    if stem.is_empty() || stem.contains('/') || stem.contains('\\') {
        return Err(format!(
            "vault note filename is not normalized: `{filename}`"
        ));
    }
    let id = stem.replace('~', "/");
    if !is_index_id(&id) {
        return Err(format!(
            "vault note filename does not decode to an Index id: `{filename}`"
        ));
    }
    Ok(id)
}

pub(crate) fn escape_markdown_label(label: &str) -> String {
    label
        .chars()
        .flat_map(|ch| match ch {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '[' => "\\[".chars().collect::<Vec<_>>(),
            ']' => "\\]".chars().collect::<Vec<_>>(),
            '\r' | '\n' => " ".chars().collect::<Vec<_>>(),
            other => vec![other],
        })
        .collect()
}

pub(crate) fn escape_wikilink_label(label: &str) -> String {
    label
        .chars()
        .flat_map(|ch| match ch {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '|' => "\\|".chars().collect::<Vec<_>>(),
            '[' => "\\[".chars().collect::<Vec<_>>(),
            ']' => "\\]".chars().collect::<Vec<_>>(),
            '\r' | '\n' => " ".chars().collect::<Vec<_>>(),
            other => vec![other],
        })
        .collect()
}

pub(crate) fn yaml_property_value(value: &str) -> String {
    format!("\"{}\"", json_escape(value))
}

pub(crate) fn logseq_property_value(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            other => vec![other],
        })
        .collect()
}

fn note_directory(kind: VaultNodeKind) -> &'static str {
    match kind {
        VaultNodeKind::Anchor => "anchors",
        VaultNodeKind::Draft => "drafts",
        VaultNodeKind::Feature => "features",
        VaultNodeKind::Route => "routes",
        VaultNodeKind::Specimen => "specimens",
        VaultNodeKind::Subject => "subjects",
        VaultNodeKind::Surface => "surfaces",
    }
}

fn relative_path(from_dir: &Path, target: &Path) -> String {
    let from = components(from_dir);
    let target = components(target);
    let common = from
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = Vec::new();
    parts.extend(std::iter::repeat_n("..".to_owned(), from.len() - common));
    parts.extend(target[common..].iter().cloned());
    if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    }
}

fn components(path: &Path) -> Vec<String> {
    path.iter()
        .map(|part| {
            part.to_str()
                .expect("vault note paths are UTF-8")
                .to_owned()
        })
        .collect()
}
