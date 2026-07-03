//! Studio Site graph builder for the SIM Atelier.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::io::{
    cache_path, check_cache, editable_roots, is_meta_workspace, normalize_roots, write_cache,
};

const SCHEMA: &str = "sim.atelier.site.v1";
const SITE_MODEL: &str = "SUP.20";
const REALIZE_OPERATION: &str = "server:realize";
const STREAM_REALIZE_OPERATION: &str = "realize_stream_events";

/// Options for generating the Atelier Site graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtelierSiteOptions {
    /// Control-plane root for resolving the default cache path.
    pub control_root: PathBuf,
    /// Optional `repos.toml` manifest for loading editable sibling roots.
    pub repos_manifest: Option<PathBuf>,
    /// Editable source roots for the generated graph.
    pub editable_roots: Vec<String>,
    /// Optional cache path for the generated JSON graph.
    pub cache_path: Option<PathBuf>,
    /// When true, fail if the cache differs from the generated graph.
    pub check: bool,
}

impl Default for AtelierSiteOptions {
    fn default() -> Self {
        Self {
            control_root: PathBuf::from("."),
            repos_manifest: None,
            editable_roots: Vec::new(),
            cache_path: None,
            check: false,
        }
    }
}

/// Summary of an `atelier-site` run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtelierSiteReport {
    /// Generated Studio Site graph.
    pub site: AtelierSite,
    /// Cache path used for the graph, when caching is enabled.
    pub cache_path: Option<PathBuf>,
    /// Whether the run wrote a different cache payload.
    pub cache_changed: bool,
}

/// SUP placement layer used by an Atelier node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtelierLayer {
    /// L0, immediate local UI state.
    L0,
    /// L1, coroutine-local interactive control.
    L1,
    /// L2, thread-level background work.
    L2,
    /// L3, loop or pipeline work.
    L3,
    /// L4, process-level work.
    L4,
    /// L5, LAN-capable work.
    L5,
    /// L6, browser-facing shell work.
    L6,
    /// L7, remote fabric boundary.
    L7,
}

impl AtelierLayer {
    /// Returns the stable layer label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::L0 => "L0",
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::L3 => "L3",
            Self::L4 => "L4",
            Self::L5 => "L5",
            Self::L6 => "L6",
            Self::L7 => "L7",
        }
    }
}

/// Kind of development node placed in the Atelier graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtelierNodeKind {
    /// Fast buffer-editing node.
    Editor,
    /// Constellation indexing node.
    Indexer,
    /// Capability-gated rule checking node.
    Guard,
    /// Agent runner or fake runner node.
    Agent,
    /// Cargo, simdoc, and audit validation node.
    Validator,
    /// Documentation-regeneration node.
    Docs,
    /// `repos.toml` pin planning node.
    Pin,
    /// Browser-facing Atelier shell node.
    Shell,
}

impl AtelierNodeKind {
    /// Returns the stable lowercase node kind name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Editor => "editor",
            Self::Indexer => "indexer",
            Self::Guard => "guard",
            Self::Agent => "agent",
            Self::Validator => "validator",
            Self::Docs => "docs",
            Self::Pin => "pin",
            Self::Shell => "shell",
        }
    }

    /// Returns the default honest placement layer for this node kind.
    pub fn default_layer(self) -> AtelierLayer {
        match self {
            Self::Editor => AtelierLayer::L1,
            Self::Indexer => AtelierLayer::L2,
            Self::Guard => AtelierLayer::L1,
            Self::Agent => AtelierLayer::L4,
            Self::Validator => AtelierLayer::L4,
            Self::Docs => AtelierLayer::L4,
            Self::Pin => AtelierLayer::L1,
            Self::Shell => AtelierLayer::L6,
        }
    }

    /// Returns the layers accepted for this node kind.
    pub fn allowed_layers(self) -> &'static [AtelierLayer] {
        match self {
            Self::Editor => &[AtelierLayer::L0, AtelierLayer::L1],
            Self::Indexer => &[AtelierLayer::L2],
            Self::Guard => &[AtelierLayer::L1],
            Self::Agent => &[AtelierLayer::L4],
            Self::Validator => &[AtelierLayer::L4, AtelierLayer::L5],
            Self::Docs => &[AtelierLayer::L4],
            Self::Pin => &[AtelierLayer::L1],
            Self::Shell => &[AtelierLayer::L6],
        }
    }

    fn capabilities(self) -> &'static [&'static str] {
        match self {
            Self::Editor => &["atelier:edit-buffer"],
            Self::Indexer => &["atelier:index-constellation"],
            Self::Guard => &["atelier:check-guidelines"],
            Self::Agent => &["atelier:run-agent"],
            Self::Validator => &["atelier:run-validation"],
            Self::Docs => &["atelier:run-docs"],
            Self::Pin => &["atelier:plan-pins"],
            Self::Shell => &["atelier:present-shell"],
        }
    }

    fn site_assignment(self, layer: AtelierLayer) -> SiteAssignment {
        match self {
            Self::Editor => SiteAssignment::coroutine("buffer-edit"),
            Self::Indexer => SiteAssignment::thread("constellation-index"),
            Self::Guard => SiteAssignment::coroutine("guideline-guard"),
            Self::Agent => SiteAssignment::process("agent-runner", layer),
            Self::Validator => SiteAssignment::process("validation-runner", layer),
            Self::Docs => SiteAssignment::process("docs-runner", layer),
            Self::Pin => SiteAssignment::coroutine("pin-planner"),
            Self::Shell => SiteAssignment::browser("atelier-shell"),
        }
    }
}

/// One placed IDE node in the Atelier Site graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtelierNode {
    id: String,
    kind: AtelierNodeKind,
    layer: AtelierLayer,
    site: SiteAssignment,
}

impl AtelierNode {
    /// Builds one node and rejects placements outside the kind's allowed layers.
    pub fn new(kind: AtelierNodeKind, layer: AtelierLayer) -> Result<Self, String> {
        if !kind.allowed_layers().contains(&layer) {
            return Err(format!(
                "atelier node {} cannot be placed at {}",
                kind.as_str(),
                layer.as_str()
            ));
        }
        Ok(Self {
            id: kind.as_str().to_owned(),
            kind,
            layer,
            site: kind.site_assignment(layer),
        })
    }

    /// Returns the node id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the node kind.
    pub fn kind(&self) -> AtelierNodeKind {
        self.kind
    }

    /// Returns the node layer.
    pub fn layer(&self) -> AtelierLayer {
        self.layer
    }

    /// Returns the SUP Site kind label used by this node.
    pub fn site_kind(&self) -> &str {
        self.site.site_kind
    }

    /// Returns the SUP server address kind label used by this node.
    pub fn address_kind(&self) -> &str {
        self.site.address_kind
    }

    /// Returns true when the node is on a process or LAN-capable site.
    pub fn is_process_or_lan(&self) -> bool {
        matches!(self.site.host_class, HostClass::Process | HostClass::Lan)
    }

    /// Returns true when the node is placed on the browser-facing site.
    pub fn is_browser(&self) -> bool {
        self.site.host_class == HostClass::Browser
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind.as_str(),
            "layer": self.layer.as_str(),
            "allowed_layers": self.kind.allowed_layers()
                .iter()
                .map(|layer| layer.as_str())
                .collect::<Vec<_>>(),
            "capabilities": self.kind.capabilities(),
            "ports": {
                "input": format!("atelier/{}/in", self.kind.as_str()),
                "output": format!("atelier/{}/out", self.kind.as_str())
            },
            "site": self.site.to_json(),
        })
    }
}

/// The SIM Atelier Studio Site graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtelierSite {
    source_policy: SourcePolicy,
    nodes: Vec<AtelierNode>,
}

impl AtelierSite {
    /// Builds the default Atelier Site graph over the supplied editable roots.
    pub fn default_for_roots(editable_roots: Vec<String>) -> Result<Self, String> {
        let source_policy = SourcePolicy::new(editable_roots)?;
        let mut nodes = Vec::new();
        for kind in [
            AtelierNodeKind::Editor,
            AtelierNodeKind::Indexer,
            AtelierNodeKind::Guard,
            AtelierNodeKind::Agent,
            AtelierNodeKind::Validator,
            AtelierNodeKind::Docs,
            AtelierNodeKind::Pin,
            AtelierNodeKind::Shell,
        ] {
            nodes.push(AtelierNode::new(kind, kind.default_layer())?);
        }
        Ok(Self {
            source_policy,
            nodes,
        })
    }

    /// Returns the placed nodes.
    pub fn nodes(&self) -> &[AtelierNode] {
        &self.nodes
    }

    /// Returns the editable roots, excluding generated meta-workspace roots.
    pub fn editable_roots(&self) -> &[String] {
        &self.source_policy.editable_roots
    }

    /// Renders the graph as stable JSON.
    pub fn to_json(&self) -> Value {
        json!({
            "schema": SCHEMA,
            "site_model": SITE_MODEL,
            "realize_surface": {
                "operation": REALIZE_OPERATION,
                "stream_operation": STREAM_REALIZE_OPERATION
            },
            "source_policy": self.source_policy.to_json(),
            "nodes": self.nodes.iter().map(AtelierNode::to_json).collect::<Vec<_>>(),
        })
    }

    /// Renders the graph as pretty JSON with a trailing newline.
    pub fn to_pretty_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(&self.to_json())
            .map(|mut text| {
                text.push('\n');
                text
            })
            .map_err(|err| format!("render atelier site json: {err}"))
    }
}

/// Generates the Atelier Site graph and optionally writes or checks its cache.
pub fn atelier_site(options: AtelierSiteOptions) -> Result<AtelierSiteReport, String> {
    let editable_roots = editable_roots(&options)?;
    let site = AtelierSite::default_for_roots(editable_roots)?;
    let cache_path = Some(cache_path(&options));
    let json = site.to_pretty_json()?;
    let cache_changed = match &cache_path {
        Some(path) if options.check => check_cache(path, &json, "xtask atelier-site")?,
        Some(path) => write_cache(path, &json)?,
        None => false,
    };
    Ok(AtelierSiteReport {
        site,
        cache_path,
        cache_changed,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourcePolicy {
    editable_roots: Vec<String>,
}

impl SourcePolicy {
    fn new(editable_roots: Vec<String>) -> Result<Self, String> {
        let mut roots = normalize_roots(editable_roots);
        if roots.is_empty() {
            roots.push(".".to_owned());
        }
        if let Some(root) = roots.iter().find(|root| is_meta_workspace(root)) {
            return Err(format!(
                ".meta-workspace cannot be an editable Atelier root: {root}"
            ));
        }
        Ok(Self {
            editable_roots: roots,
        })
    }

    fn to_json(&self) -> Value {
        json!({
            "editable_roots": self.editable_roots,
            "generated_roots": [".meta-workspace/"],
            "editable_roots_include_meta_workspace": false,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SiteAssignment {
    site_kind: &'static str,
    address_kind: &'static str,
    host_class: HostClass,
    target: &'static str,
}

impl SiteAssignment {
    fn coroutine(target: &'static str) -> Self {
        Self {
            site_kind: "coroutine",
            address_kind: "coroutine",
            host_class: HostClass::Local,
            target,
        }
    }

    fn thread(target: &'static str) -> Self {
        Self {
            site_kind: "local",
            address_kind: "in-process",
            host_class: HostClass::Thread,
            target,
        }
    }

    fn process(target: &'static str, layer: AtelierLayer) -> Self {
        let (address_kind, host_class) = if layer == AtelierLayer::L5 {
            ("tcp", HostClass::Lan)
        } else {
            ("agent", HostClass::Process)
        };
        Self {
            site_kind: "fabric",
            address_kind,
            host_class,
            target,
        }
    }

    fn browser(target: &'static str) -> Self {
        Self {
            site_kind: "fabric",
            address_kind: "http",
            host_class: HostClass::Browser,
            target,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "site_kind": self.site_kind,
            "server_address_kind": self.address_kind,
            "host_class": self.host_class.as_str(),
            "target": self.target,
            "realize": REALIZE_OPERATION,
            "stream_realize": STREAM_REALIZE_OPERATION,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostClass {
    Local,
    Thread,
    Process,
    Lan,
    Browser,
}

impl HostClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Thread => "thread",
            Self::Process => "process",
            Self::Lan => "lan",
            Self::Browser => "browser",
        }
    }
}
