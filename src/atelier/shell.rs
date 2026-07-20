//! SIM Atelier shell aggregate.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    guard::{AtelierGuardOptions, atelier_guard},
    index::{AtelierIndexOptions, atelier_index},
    io::{check_cache, write_cache},
    radar::{AtelierRadarOptions, RadarQuery, atelier_radar},
    site::{AtelierSiteOptions, atelier_site},
    tools::{AtelierToolsOptions, atelier_tools},
};

mod cli;
mod contract_native;
mod render;

const SCHEMA: &str = "sim.atelier.shell.v1";
const DEFAULT_CACHE: &str = ".sim/atelier/shell.json";
const DEFAULT_SITE_CACHE: &str = ".sim/atelier/site.json";
const DEFAULT_INDEX_CACHE_DIR: &str = ".sim/atelier/index";
const DEFAULT_TOOLS_CACHE: &str = ".sim/atelier/tools.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AtelierShellOptions {
    pub(super) control_root: PathBuf,
    pub(super) repos_manifest: Option<PathBuf>,
    pub(super) cache_path: Option<PathBuf>,
    pub(super) backend: AtelierBackend,
    pub(super) check: bool,
}

impl Default for AtelierShellOptions {
    fn default() -> Self {
        Self {
            control_root: PathBuf::from("."),
            repos_manifest: None,
            cache_path: None,
            backend: AtelierBackend::SourceRadar,
            check: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum AtelierBackend {
    #[default]
    SourceRadar,
    ContractNative,
}

impl AtelierBackend {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "source-radar" => Some(Self::SourceRadar),
            "contract-native" => Some(Self::ContractNative),
            _ => None,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::SourceRadar => "source-radar",
            Self::ContractNative => "contract-native",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct AtelierShellReport {
    pub(super) shell: Value,
    pub(super) cache_file: PathBuf,
    pub(super) cache_changed: bool,
}

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    cli::run(args)
}

pub(super) fn atelier_shell(options: AtelierShellOptions) -> Result<AtelierShellReport, String> {
    let manifest_path = options
        .repos_manifest
        .clone()
        .unwrap_or_else(|| options.control_root.join("repos.toml"));
    let site = atelier_site(AtelierSiteOptions {
        control_root: options.control_root.clone(),
        repos_manifest: Some(manifest_path.clone()),
        cache_path: Some(options.control_root.join(DEFAULT_SITE_CACHE)),
        check: options.check,
        editable_roots: Vec::new(),
    })?;
    let index = atelier_index(AtelierIndexOptions {
        control_root: options.control_root.clone(),
        repos_manifest: Some(manifest_path.clone()),
        cache_dir: Some(options.control_root.join(DEFAULT_INDEX_CACHE_DIR)),
        check: options.check,
        ..AtelierIndexOptions::default()
    })?;
    let tools = atelier_tools(AtelierToolsOptions {
        control_root: options.control_root.clone(),
        repos_manifest: Some(manifest_path.clone()),
        cache_path: Some(options.control_root.join(DEFAULT_TOOLS_CACHE)),
        check: options.check,
        ..AtelierToolsOptions::default()
    })?;
    let guard = atelier_guard(AtelierGuardOptions {
        control_root: options.control_root.clone(),
        repos_manifest: Some(manifest_path),
        ..AtelierGuardOptions::default()
    })?;
    let radar = radar_panels(&options.control_root, &index.cache_file)?;
    let contract_native = match options.backend {
        AtelierBackend::SourceRadar => None,
        AtelierBackend::ContractNative => Some(contract_native::report_json(&guard)),
    };
    let shell = render::shell_json(
        &site,
        &index,
        &tools,
        &guard,
        radar,
        options.backend,
        contract_native,
    );
    let content = render::pretty_json(&shell)?;
    let cache_file = options
        .cache_path
        .clone()
        .unwrap_or_else(|| options.control_root.join(DEFAULT_CACHE));
    let cache_changed = if options.check {
        check_cache(&cache_file, &content, "xtask atelier-shell")?
    } else {
        write_cache(&cache_file, &content)?
    };
    Ok(AtelierShellReport {
        shell,
        cache_file,
        cache_changed,
    })
}

fn radar_panels(control_root: &Path, index_file: &Path) -> Result<Vec<Value>, String> {
    [
        (
            "rust-source",
            "Rust source",
            Some("rust-fn"),
            None,
            None,
            None,
        ),
        (
            "codec-prism",
            "Codec Prism",
            None,
            None,
            Some("codec"),
            None,
        ),
        ("docs-recipes", "recipe", Some("recipe"), None, None, None),
        (
            "retrieval-radar",
            "ranked confidence hints",
            None,
            Some("capability"),
            None,
            None,
        ),
        (
            "guideline-firewall",
            "guard rule",
            None,
            None,
            None,
            Some("guard"),
        ),
    ]
    .into_iter()
    .map(
        |(panel, text, kind, capability, codec, agent_role)| -> Result<Value, String> {
            let report = atelier_radar(AtelierRadarOptions {
                control_root: control_root.to_path_buf(),
                index_file: Some(index_file.to_path_buf()),
                query: RadarQuery {
                    text: text.to_owned(),
                    kind: kind.map(str::to_owned),
                    capability: capability.map(str::to_owned),
                    codec: codec.map(str::to_owned),
                    agent_role: agent_role.map(str::to_owned),
                    limit: 3,
                    ..RadarQuery::default()
                },
                json: false,
            })?;
            Ok(render::radar_json(panel, &report))
        },
    )
    .collect()
}
