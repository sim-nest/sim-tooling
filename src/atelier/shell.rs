//! SIM Atelier shell aggregate.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};

use super::{
    eval,
    guard::{AtelierGuardOptions, AtelierGuardReport, atelier_guard},
    index::{AtelierIndexOptions, AtelierIndexReport, atelier_index},
    io::{check_cache, display_io, write_cache},
    radar::{AtelierRadarOptions, RadarQuery, RadarReport, atelier_radar},
    site::{AtelierSiteOptions, AtelierSiteReport, atelier_site},
    tools::{AtelierToolsOptions, AtelierToolsReport, atelier_tools},
};

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
    pub(super) check: bool,
}

impl Default for AtelierShellOptions {
    fn default() -> Self {
        Self {
            control_root: PathBuf::from("."),
            repos_manifest: None,
            cache_path: None,
            check: false,
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
    let mut options = AtelierShellOptions {
        control_root: std::env::current_dir().map_err(display_io)?,
        ..AtelierShellOptions::default()
    };
    let mut print = true;
    let mut args = args.into_iter().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--control-root" => options.control_root = next_path(&mut args, "--control-root")?,
            "--repos-manifest" => {
                options.repos_manifest = Some(next_path(&mut args, "--repos-manifest")?);
            }
            "--cache" => options.cache_path = Some(next_path(&mut args, "--cache")?),
            "--check" => {
                options.check = true;
                print = false;
            }
            "--refresh-only" => print = false,
            "--json" => print = true,
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown atelier-shell option: {other}")),
        }
    }

    let report = atelier_shell(options)?;
    if print {
        print!("{}", pretty_json(&report.shell)?);
    }
    let status = if report.cache_changed {
        "updated"
    } else {
        "current"
    };
    eprintln!(
        "atelier-shell: cache {status}: {}",
        report.cache_file.display()
    );
    Ok(())
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
    let shell = shell_json(&site, &index, &tools, &guard, radar);
    let content = pretty_json(&shell)?;
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

fn shell_json(
    site: &AtelierSiteReport,
    index: &AtelierIndexReport,
    tools: &AtelierToolsReport,
    guard: &AtelierGuardReport,
    radar: Vec<Value>,
) -> Value {
    json!({
        "schema": SCHEMA,
        "startup": startup_json(index),
        "site": site.site.to_json(),
        "index": {
            "cache": index.cache_file.to_string_lossy(),
            "diagnostics": index.index["diagnostics"].clone(),
        },
        "navigation": navigation_json(&index.index, guard),
        "panels": panel_json(),
        "radar": radar,
        "firewall": {
            "rules": guard.rules.iter().map(rule_json).collect::<Vec<_>>(),
            "findings": guard.findings.iter().map(finding_json).collect::<Vec<_>>(),
        },
        "tools": {
            "cache": tools.cache_file.to_string_lossy(),
            "descriptors": tools.descriptors.len(),
            "repo_descriptors": tools.repo_tool_count(),
        },
        "scenarios": eval::scenario_json(),
        "editor_policy": editor_policy_json(),
    })
}

fn startup_json(index: &AtelierIndexReport) -> Value {
    let repos = index.index["repos"].as_array().cloned().unwrap_or_default();
    json!({
        "cache": {
            "site": "current",
            "index": "current",
            "tools": "current",
        },
        "dirty_repos": repos_with_status(&repos, "dirty"),
        "missing_siblings": repos_with_statuses(
            &repos,
            &["missing", "missing-cargo-toml", "not-git"],
        ),
        "validation": repos.iter().filter_map(validation_json).collect::<Vec<_>>(),
    })
}

fn navigation_json(index: &Value, guard: &AtelierGuardReport) -> Value {
    let repos = index["repos"].as_array().cloned().unwrap_or_default();
    let units = index["units"].as_array().cloned().unwrap_or_default();
    let chunks = index["chunks"].as_array().cloned().unwrap_or_default();

    json!([
        nav_section("repo", repos.iter().filter_map(name_field).collect()),
        nav_section("crate", crates(&repos)),
        nav_section("capability", chunk_strings(&chunks, "capabilities")),
        nav_section("codec", chunk_strings(&chunks, "codecs")),
        nav_section("recipe", recipe_paths(&units)),
        nav_section("agent-role", agent_roles(&chunks)),
        nav_section(
            "guard-rule",
            guard.rules.iter().map(|rule| rule.id.to_owned()).collect(),
        ),
    ])
}

fn panel_json() -> Value {
    json!([
        {
            "id": "rust-source",
            "title": "Rust source",
            "source": "Rust intelligence bridge",
            "editable": true,
        },
        {
            "id": "codec-prism",
            "title": "Codec Prism",
            "source": "sim-lib-view-codec",
            "editable": true,
        },
        {
            "id": "docs-recipes",
            "title": "Docs and recipes",
            "source": "README, rustdoc source, recipes/",
            "editable": true,
        },
        {
            "id": "retrieval-radar",
            "title": "Retrieval Radar",
            "source": "sim-lib-rank hints",
            "editable": false,
        },
        {
            "id": "guideline-firewall",
            "title": "Guideline Firewall",
            "source": "GuidelineRule catalog",
            "editable": false,
        },
    ])
}

fn editor_policy_json() -> Value {
    json!({
        "editable_docs": [
            "README.md",
            "src/**/*.rs rustdoc",
            "recipes/**/purpose.md",
            "recipes/**/recipe.toml",
            "recipes/**/setup.siml",
        ],
        "read_only_generated_docs": [
            "docs/generated/",
            "docs/agents/",
            "docs/humans/",
            "docs/diagrams/generated/",
        ],
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
            Ok(radar_json(panel, &report))
        },
    )
    .collect()
}

fn radar_json(panel: &str, report: &RadarReport) -> Value {
    json!({
        "panel": panel,
        "stale_index": report.stale_index,
        "stale_chunk_ids": report.stale_chunk_ids,
        "hints": report.hints.iter().map(|hint| {
            json!({
                "title": hint.title,
                "confidence": hint.confidence,
                "span": {
                    "repo": hint.repo,
                    "file": hint.path,
                    "line": hint.line,
                },
                "capabilities": hint.capabilities,
                "preferred_codec": hint.preferred_codec,
            })
        }).collect::<Vec<_>>(),
    })
}

fn rule_json(rule: &super::GuidelineRule) -> Value {
    json!({
        "id": rule.id,
        "title": rule.title,
        "severity": rule.severity.as_str(),
        "location": rule.scope,
        "quick_fix": rule.quick_fix,
        "gated_capability": rule.gated_capability,
    })
}

fn finding_json(finding: &super::GuidelineFinding) -> Value {
    json!({
        "rule_id": finding.rule_id,
        "title": finding.title,
        "severity": finding.severity.as_str(),
        "location": finding.location,
        "evidence": finding.evidence,
        "quick_fix": finding.quick_fix,
        "gated_capability": finding.gated_capability,
    })
}

fn repos_with_status(repos: &[Value], status: &str) -> Vec<String> {
    repos_with_statuses(repos, &[status])
}

fn repos_with_statuses(repos: &[Value], statuses: &[&str]) -> Vec<String> {
    repos
        .iter()
        .filter(|repo| {
            repo["status"]
                .as_str()
                .is_some_and(|status| statuses.contains(&status))
        })
        .filter_map(name_field)
        .collect()
}

fn validation_json(repo: &Value) -> Option<Value> {
    let name = repo["name"].as_str()?;
    let command = repo["validation_command"].as_str().unwrap_or_default();
    if command.is_empty() {
        return None;
    }
    let status = match repo["status"].as_str().unwrap_or("missing") {
        "clean" => "ready",
        "dirty" => "needs-review",
        _ => "blocked",
    };
    Some(json!({
        "repo": name,
        "status": status,
        "command": command,
    }))
}

fn nav_section(kind: &str, items: Vec<String>) -> Value {
    json!({
        "kind": kind,
        "items": sorted(items),
    })
}

fn name_field(value: &Value) -> Option<String> {
    value["name"].as_str().map(str::to_owned)
}

fn crates(repos: &[Value]) -> Vec<String> {
    repos
        .iter()
        .flat_map(|repo| string_array(&repo["crates"]))
        .collect()
}

fn chunk_strings(chunks: &[Value], field: &str) -> Vec<String> {
    chunks
        .iter()
        .flat_map(|chunk| string_array(&chunk[field]))
        .collect()
}

fn recipe_paths(units: &[Value]) -> Vec<String> {
    units
        .iter()
        .filter(|unit| unit["kind"].as_str() == Some("recipe"))
        .filter_map(|unit| unit["path"].as_str().map(str::to_owned))
        .collect()
}

fn agent_roles(chunks: &[Value]) -> Vec<String> {
    let mut roles = Vec::new();
    for chunk in chunks {
        let text = chunk["text"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        for role in [
            "agent",
            "planner",
            "retriever",
            "validator",
            "guard",
            "editor",
            "docs",
            "pin",
        ] {
            if text.contains(role) {
                roles.push(role.to_owned());
            }
        }
    }
    roles
}

fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn sorted(items: Vec<String>) -> Vec<String> {
    items
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn pretty_json(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|mut text| {
            text.push('\n');
            text
        })
        .map_err(|err| format!("render atelier shell json: {err}"))
}

fn print_usage() {
    println!(
        "usage: xtask atelier-shell [--control-root PATH] [--repos-manifest PATH] [--cache PATH] [--check|--refresh-only|--json]"
    );
}

fn next_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} requires a value"))
}
