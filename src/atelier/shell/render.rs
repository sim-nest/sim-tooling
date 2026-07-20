use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::atelier::{
    eval,
    guard::{AtelierGuardReport, GuidelineFinding, GuidelineRule},
    index::AtelierIndexReport,
    radar::RadarReport,
    site::AtelierSiteReport,
    tools::AtelierToolsReport,
};

use super::{AtelierBackend, SCHEMA};

pub(super) fn shell_json(
    site: &AtelierSiteReport,
    index: &AtelierIndexReport,
    tools: &AtelierToolsReport,
    guard: &AtelierGuardReport,
    radar: Vec<Value>,
    backend: AtelierBackend,
    contract_native: Option<Value>,
) -> Value {
    let contract_native_enabled = contract_native.is_some();
    let mut shell = json!({
        "schema": SCHEMA,
        "startup": startup_json(index, backend),
        "site": site.site.to_json(),
        "index": {
            "cache": index.cache_file.to_string_lossy(),
            "diagnostics": index.index["diagnostics"].clone(),
        },
        "navigation": navigation_json(&index.index, guard),
        "panels": panel_json(contract_native_enabled),
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
    });
    if let Some(contract_native) = contract_native {
        shell
            .as_object_mut()
            .expect("shell JSON is an object")
            .insert("contract_native".to_owned(), contract_native);
    }
    shell
}

pub(super) fn radar_json(panel: &str, report: &RadarReport) -> Value {
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

pub(super) fn pretty_json(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|mut text| {
            text.push('\n');
            text
        })
        .map_err(|err| format!("render atelier shell json: {err}"))
}

fn startup_json(index: &AtelierIndexReport, backend: AtelierBackend) -> Value {
    let repos = index.index["repos"].as_array().cloned().unwrap_or_default();
    let mut startup = json!({
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
    });
    if backend != AtelierBackend::SourceRadar {
        startup
            .as_object_mut()
            .expect("startup JSON is an object")
            .insert("backend".to_owned(), json!(backend.as_str()));
    }
    startup
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

fn panel_json(contract_native: bool) -> Value {
    let mut panels = vec![
        {
            json!({
                "id": "rust-source",
                "title": "Rust source",
                "source": "Rust intelligence bridge",
                "editable": true,
            })
        },
        {
            json!({
                "id": "codec-prism",
                "title": "Codec Prism",
                "source": "sim-lib-view-codec",
                "editable": true,
            })
        },
        {
            json!({
                "id": "docs-recipes",
                "title": "Docs and recipes",
                "source": "README, rustdoc source, recipes/",
                "editable": true,
            })
        },
        {
            json!({
                "id": "retrieval-radar",
                "title": "Retrieval Radar",
                "source": "sim-lib-rank hints",
                "editable": false,
            })
        },
        {
            json!({
                "id": "guideline-firewall",
                "title": "Guideline Firewall",
                "source": "GuidelineRule catalog",
                "editable": false,
            })
        },
    ];
    if contract_native {
        panels.push(json!({
            "id": "contract-native",
            "title": "Contract-native",
            "source": "FORGE contract deck cache",
            "editable": false,
        }));
    }
    json!(panels)
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

fn rule_json(rule: &GuidelineRule) -> Value {
    json!({
        "id": rule.id,
        "title": rule.title,
        "severity": rule.severity.as_str(),
        "location": rule.scope,
        "quick_fix": rule.quick_fix,
        "gated_capability": rule.gated_capability,
    })
}

fn finding_json(finding: &GuidelineFinding) -> Value {
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
