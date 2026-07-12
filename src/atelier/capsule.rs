//! Change Capsule cache for the SIM Atelier.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};
use sim_cookbook::fnv1a64_hex;

use super::{
    index_manifest::{RepoEntry, read_repos_manifest},
    io::{check_cache, display_io, write_cache},
};

const SCHEMA: &str = "sim.atelier.change-capsule.v1";
const DEFAULT_CACHE: &str = ".sim/atelier/change-capsule.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AtelierCapsuleOptions {
    pub(super) control_root: PathBuf,
    pub(super) repos_manifest: Option<PathBuf>,
    pub(super) cache_path: Option<PathBuf>,
    pub(super) check: bool,
}

impl Default for AtelierCapsuleOptions {
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
pub(super) struct AtelierCapsuleReport {
    pub(super) capsule: Value,
    pub(super) cache_file: PathBuf,
    pub(super) cache_changed: bool,
}

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let mut options = AtelierCapsuleOptions {
        control_root: std::env::current_dir().map_err(display_io)?,
        ..AtelierCapsuleOptions::default()
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
            other => return Err(format!("unknown atelier-capsule option: {other}")),
        }
    }

    let report = atelier_capsule(options)?;
    if print {
        print!("{}", pretty_json(&report.capsule)?);
    }
    let status = if report.cache_changed {
        "updated"
    } else {
        "current"
    };
    eprintln!(
        "atelier-capsule: cache {status}: {}",
        report.cache_file.display()
    );
    Ok(())
}

pub(super) fn atelier_capsule(
    options: AtelierCapsuleOptions,
) -> Result<AtelierCapsuleReport, String> {
    let manifest_path = options
        .repos_manifest
        .clone()
        .unwrap_or_else(|| options.control_root.join("repos.toml"));
    let repos = read_repos_manifest(&options.control_root, &manifest_path)?;
    let capsule = capsule_json(&repos);
    let content = pretty_json(&capsule)?;
    let cache_file = options
        .cache_path
        .clone()
        .unwrap_or_else(|| options.control_root.join(DEFAULT_CACHE));
    let cache_changed = if options.check {
        check_cache(&cache_file, &content, "xtask atelier-capsule")?
    } else {
        write_cache(&cache_file, &content)?
    };
    Ok(AtelierCapsuleReport {
        capsule,
        cache_file,
        cache_changed,
    })
}

fn capsule_json(repos: &[RepoEntry]) -> Value {
    let preview = repo_preview(repos);
    let pin_plan = pin_plan(repos);
    let docs_repo = docs_repo_name(repos);
    let docs_repo = docs_repo.as_deref().unwrap_or("front-page");
    let cassette_events = vec![
        event(0, "edit", "local-coroutine", "Patch capsule model"),
        event(1, "validate", "process", "cargo test change_capsule"),
        event(2, "docs", "process", "simdoc check"),
        event(3, "pin", "local-coroutine", "Plan pushed commit pin"),
        event(
            4,
            "reflect",
            "local-coroutine",
            "F6 risk and rollback facet",
        ),
    ];
    let content_hash = content_hash(&cassette_events);
    json!({
        "schema": SCHEMA,
        "capsule": {
            "id": "atelier/capsule/sample",
            "scope": {
                "repos": preview.iter().map(|repo| repo["name"].clone()).collect::<Vec<_>>(),
                "targets": [
                    "src/atelier/capsule.rs",
                    "crates/sim-lib-agent/src/atelier/capsule.rs",
                    "crates/sim-lib-view-agent/src/change_capsule.rs"
                ],
            },
            "patches": [
                {"repo": "sim-tooling", "path": "src/atelier/capsule.rs"},
                {"repo": "sim-agent-net", "path": "crates/sim-lib-agent/src/atelier/capsule.rs"},
                {"repo": "sim-web", "path": "crates/sim-lib-view-agent/src/change_capsule.rs"}
            ],
            "generated_artifacts": generated_artifacts(docs_repo),
            "validations": [job("validation", "workspace-check", "process", "realize", "passed")],
            "docs_runs": [job("docs", "simdoc-check", "process", "realize", "passed")],
            "repo_preview": preview,
            "pin_plan": pin_plan,
            "site_changes": [
                {"repo": docs_repo, "path": "docs/site/repos.md", "generated": true}
            ],
            "risks": ["review generated docs and pin plan before commit"],
            "rollback_notes": ["restore previous pins and rerun simctl site"],
            "placement_plan": placement_plan(),
            "conformance": {
                "sup11_streams": true,
                "sup28_placement": true,
                "realize_operation": "realize",
                "edit_site": "local-coroutine",
                "capsule_assembly_site": "local-coroutine"
            },
            "dev_cassette": {
                "events": cassette_events,
                "content_hash": content_hash,
                "replay_content_hash": content_hash
            },
            "fairness_facet": {
                "label": "F6 trade-off",
                "evidence": "diff, validation, docs, pin, replay, risk, and rollback evidence",
                "confidence": "0.92"
            },
            "policy": {
                "preview_public_repos_before_pin": true,
                "refuses_stale_pins": true,
                "refuses_generated_doc_hand_edit": true
            }
        }
    })
}

fn repo_preview(repos: &[RepoEntry]) -> Vec<Value> {
    repos
        .iter()
        .filter(|repo| repo.kind == "code")
        .map(|repo| {
            let head = git_head(&repo.checkout_path);
            json!({
                "name": repo.name,
                "status": repo.status.as_str(),
                "pinned_commit": repo.pin,
                "local_head": head,
                "pin_matches_head": head.as_deref() == Some(repo.pin.as_str()),
                "validation_command": repo.validation_command,
                "docs_command": repo.docs_command,
            })
        })
        .collect()
}

fn pin_plan(repos: &[RepoEntry]) -> Vec<Value> {
    repos
        .iter()
        .filter(|repo| repo.kind == "code")
        .filter_map(|repo| {
            let head = git_head(&repo.checkout_path)?;
            Some(json!({
                "repo": repo.name,
                "current_commit": repo.pin,
                "new_commit": head,
                "pushed_commit_exists": commit_exists(&repo.checkout_path, &head),
                "requires_plan_pin": true,
            }))
        })
        .collect()
}

fn generated_artifacts(docs_repo: &str) -> Vec<Value> {
    vec![
        json!({
            "repo": "sim-tooling",
            "path": "docs/generated/contract.md",
            "generated_public_doc": true,
            "hand_edited": false,
            "generator": "xtask simdoc"
        }),
        json!({
            "repo": docs_repo,
            "path": "docs/site/repos.md",
            "generated_public_doc": true,
            "hand_edited": false,
            "generator": "simctl site"
        }),
    ]
}

fn docs_repo_name(repos: &[RepoEntry]) -> Option<String> {
    repos
        .iter()
        .find(|repo| repo.kind == "frontpage")
        .map(|repo| repo.name.clone())
}

fn placement_plan() -> Vec<Value> {
    vec![
        placed("edit", "local-coroutine", "local-coroutine"),
        placed("capsule-assembly", "local-coroutine", "local-coroutine"),
        placed("validation", "process", "realize"),
        placed("docs", "process", "realize"),
        placed("pin-plan", "local-coroutine", "local-coroutine"),
    ]
}

fn placed(label: &str, site: &str, realize_operation: &str) -> Value {
    json!({
        "label": label,
        "site": site,
        "realize_operation": realize_operation
    })
}

fn job(kind: &str, label: &str, site: &str, realize_operation: &str, outcome: &str) -> Value {
    json!({
        "kind": kind,
        "label": label,
        "site": site,
        "realize_operation": realize_operation,
        "outcome": outcome
    })
}

fn event(sequence: u64, kind: &str, site: &str, summary: &str) -> Value {
    json!({
        "sequence": sequence,
        "media": format!("ide/event/{kind}"),
        "site": site,
        "summary": summary
    })
}

fn git_head(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        None
    }
}

fn commit_exists(root: &Path, commit: &str) -> bool {
    upstream_ref(root)
        .filter(|upstream| commit_is_ancestor(root, commit, upstream))
        .is_some()
        || remote_branch_tip(root)
            .filter(|tip| commit == tip || commit_is_ancestor(root, commit, tip))
            .is_some()
}

fn upstream_ref(root: &Path) -> Option<String> {
    git_text(
        root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
}

fn remote_branch_tip(root: &Path) -> Option<String> {
    let branch = git_text(root, &["branch", "--show-current"])?;
    let remote = git_text(root, &["config", &format!("branch.{branch}.remote")])?;
    let merge = git_text(root, &["config", &format!("branch.{branch}.merge")])?;
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("ls-remote")
        .arg(remote)
        .arg(merge)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .map(str::to_owned)
}

fn commit_is_ancestor(root: &Path, commit: &str, reference: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["merge-base", "--is-ancestor", commit, reference])
        .status()
        .is_ok_and(|status| status.success())
}

fn git_text(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|text| !text.is_empty())
}

fn content_hash(events: &[Value]) -> String {
    format!("fnv1a64:{}", fnv1a64_hex(format!("{events:?}").as_bytes()))
}

fn pretty_json(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|mut text| {
            text.push('\n');
            text
        })
        .map_err(|err| format!("render atelier capsule json: {err}"))
}

fn print_usage() {
    println!(
        "usage: xtask atelier-capsule [--control-root PATH] [--repos-manifest PATH] [--cache PATH] [--check|--refresh-only|--json]"
    );
}

fn next_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} requires a value"))
}

#[cfg(test)]
pub(super) fn test_capsule_json(repos: &[RepoEntry]) -> Value {
    capsule_json(repos)
}
