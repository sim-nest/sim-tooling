//! Typed Atelier tool descriptor catalog.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::{
    index_manifest::{RepoEntry, read_repos_manifest},
    io::{check_cache, display_io, write_cache},
};

const SCHEMA: &str = "sim.atelier.tools.v1";
const DEFAULT_CACHE: &str = ".sim/atelier/tools.json";

/// Options for generating the Atelier tool catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtelierToolsOptions {
    /// Control-plane root used to resolve `repos.toml` and default cache paths.
    pub control_root: PathBuf,
    /// Optional manifest path. Defaults to `repos.toml` under `control_root`.
    pub repos_manifest: Option<PathBuf>,
    /// Optional repository filter for repo-scoped descriptors.
    pub repo_filter: Option<String>,
    /// Optional cache path for the generated JSON catalog.
    pub cache_path: Option<PathBuf>,
    /// Fail when the cache differs from the generated catalog.
    pub check: bool,
}

impl Default for AtelierToolsOptions {
    fn default() -> Self {
        Self {
            control_root: PathBuf::from("."),
            repos_manifest: None,
            repo_filter: None,
            cache_path: None,
            check: false,
        }
    }
}

/// Summary of an `atelier-tools` run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtelierToolsReport {
    /// Generated tool descriptors.
    pub descriptors: Vec<AtelierToolDescriptor>,
    /// Cache path used for the descriptor catalog.
    pub cache_file: PathBuf,
    /// Whether this run wrote a different cache payload.
    pub cache_changed: bool,
}

impl AtelierToolsReport {
    /// Counts descriptors scoped to a repository.
    pub fn repo_tool_count(&self) -> usize {
        self.descriptors
            .iter()
            .filter(|descriptor| descriptor.repo.is_some())
            .count()
    }
}

/// One typed, guard-checked development tool descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtelierToolDescriptor {
    /// Stable descriptor id.
    pub id: String,
    /// Human-facing title.
    pub title: String,
    /// Tool action kind.
    pub action: AtelierToolAction,
    /// Exact command or typed command template.
    pub command: String,
    /// Guard capability required before use.
    pub guard_capability: String,
    /// DevEnvelope kind recorded as evidence.
    pub evidence_kind: String,
    /// Optional repository scope.
    pub repo: Option<String>,
    /// DevEnvelope command field value.
    pub envelope_command: String,
    /// DevEnvelope exit-status field name.
    pub envelope_exit_status_field: String,
    /// DevEnvelope log-path field value.
    pub envelope_log_path: String,
    /// Whether the tool refuses generated public doc hand edits.
    pub refuses_generated_doc_hand_edit: bool,
    /// Whether the tool requires an existing pushed upstream commit.
    pub requires_pushed_commit: bool,
}

impl AtelierToolDescriptor {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "action": self.action.as_str(),
            "command": self.command,
            "guard_capability": self.guard_capability,
            "evidence": {
                "dev_envelope_kind": self.evidence_kind,
                "command": self.envelope_command,
                "exit_status_field": self.envelope_exit_status_field,
                "log_path": self.envelope_log_path,
            },
            "repo": self.repo,
            "policy": {
                "checks_guard_capability": true,
                "refuses_meta_workspace_edits": true,
                "refuses_github_mirror_operations": true,
                "refuses_control_repo_rust_code": true,
                "refuses_generated_doc_hand_edit": self.refuses_generated_doc_hand_edit,
                "requires_pushed_upstream_commit": self.requires_pushed_commit,
            },
        })
    }
}

/// Tool action categories emitted by the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtelierToolAction {
    /// `simctl` control-plane command.
    Simctl,
    /// Repository validation command.
    Validate,
    /// Repository documentation command.
    Docs,
    /// Pin update proposal.
    PinPropose,
    /// Pin update preview.
    PinPreview,
    /// Pin update application.
    PinApply,
    /// Documentation regeneration policy entry.
    DocsRegenerate,
}

impl AtelierToolAction {
    /// Returns the stable lowercase action label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Simctl => "simctl",
            Self::Validate => "validate",
            Self::Docs => "docs",
            Self::PinPropose => "pin-propose",
            Self::PinPreview => "pin-preview",
            Self::PinApply => "pin-apply",
            Self::DocsRegenerate => "docs-regenerate",
        }
    }
}

/// Generates the typed Atelier tool catalog and optionally writes or checks it.
pub fn atelier_tools(options: AtelierToolsOptions) -> Result<AtelierToolsReport, String> {
    let manifest_path = options
        .repos_manifest
        .clone()
        .unwrap_or_else(|| options.control_root.join("repos.toml"));
    let mut repos = read_repos_manifest(&options.control_root, &manifest_path)?;
    let control_repo = control_repo_name(&repos);
    let docs_repo = docs_repo_name(&repos);
    if let Some(filter) = &options.repo_filter {
        repos.retain(|repo| repo.name == *filter);
    }
    let descriptors = tool_descriptors(&repos, control_repo.as_deref(), docs_repo.as_deref());
    validate_descriptors(&descriptors, control_repo.as_deref())?;
    let catalog = catalog_json(&options, &manifest_path, &descriptors);
    let content = pretty_json(&catalog)?;
    let cache_file = options
        .cache_path
        .clone()
        .unwrap_or_else(|| options.control_root.join(DEFAULT_CACHE));
    let cache_changed = if options.check {
        check_cache(&cache_file, &content, "xtask atelier-tools")?
    } else {
        write_cache(&cache_file, &content)?
    };
    Ok(AtelierToolsReport {
        descriptors,
        cache_file,
        cache_changed,
    })
}

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let mut options = AtelierToolsOptions {
        control_root: std::env::current_dir().map_err(display_io)?,
        ..AtelierToolsOptions::default()
    };
    let mut print = true;
    let mut args = args.into_iter().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--control-root" => {
                options.control_root = next_path(&mut args, "--control-root")?;
            }
            "--repos-manifest" => {
                options.repos_manifest = Some(next_path(&mut args, "--repos-manifest")?);
            }
            "--repo" => {
                options.repo_filter = Some(next_string(&mut args, "--repo")?);
            }
            "--cache" => {
                options.cache_path = Some(next_path(&mut args, "--cache")?);
            }
            "--json" => {
                print = true;
            }
            "--check" => {
                options.check = true;
                print = false;
            }
            "--refresh-only" => {
                print = false;
            }
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown atelier-tools option: {other}")),
        }
    }

    let manifest_path = options
        .repos_manifest
        .clone()
        .unwrap_or_else(|| options.control_root.join("repos.toml"));
    let print_options = options.clone();
    let report = atelier_tools(options)?;
    if print {
        print!(
            "{}",
            pretty_json(&catalog_json(
                &print_options,
                &manifest_path,
                &report.descriptors,
            ))?
        );
    }
    let status = if report.cache_changed {
        "updated"
    } else {
        "current"
    };
    eprintln!(
        "atelier-tools: {} descriptor(s), cache {status}: {}",
        report.descriptors.len(),
        report.cache_file.display()
    );
    Ok(())
}

fn control_repo_name(repos: &[RepoEntry]) -> Option<String> {
    repos
        .iter()
        .find(|repo| repo.kind == "private")
        .map(|repo| repo.name.clone())
}

fn docs_repo_name(repos: &[RepoEntry]) -> Option<String> {
    repos
        .iter()
        .find(|repo| repo.kind == "frontpage")
        .map(|repo| repo.name.clone())
}

fn tool_descriptors(
    repos: &[RepoEntry],
    control_repo: Option<&str>,
    docs_repo: Option<&str>,
) -> Vec<AtelierToolDescriptor> {
    let mut descriptors = simctl_descriptors(control_repo, docs_repo);
    for repo in repos {
        if !repo.validation_command.is_empty() {
            descriptors.push(validation_descriptor(repo));
        }
        if !repo.docs_command.is_empty() {
            descriptors.push(docs_descriptor(repo));
            descriptors.push(docs_regeneration_descriptor(repo));
        }
        descriptors.extend(pin_descriptors(repo));
    }
    descriptors.sort_by(|left, right| left.id.cmp(&right.id));
    descriptors
}

fn simctl_descriptors(
    control_repo: Option<&str>,
    docs_repo: Option<&str>,
) -> Vec<AtelierToolDescriptor> {
    let control_repo = control_repo.unwrap_or("control-plane");
    let docs_repo = docs_repo.unwrap_or(control_repo);
    [
        ("clone", "Clone or update sibling repos"),
        ("meta-build", "Regenerate the validation meta workspace"),
        ("audit", "Run the private-data audit"),
        ("no-github-check", "Assert no GitHub work is enabled"),
        ("site", "Regenerate the public front page"),
        ("repos", "List the constellation manifest"),
        ("atelier-site", "Emit the Atelier Site graph"),
        ("atelier-index", "Refresh the Atelier index"),
        ("atelier-radar", "Query ranked Atelier hints"),
        ("atelier-guard", "Run the Guideline Firewall"),
    ]
    .into_iter()
    .map(|(command, title)| {
        let guard_capability = if command == "site" {
            format!("RegenDocs({docs_repo})")
        } else {
            format!("RunValidation({control_repo})")
        };
        descriptor(
            format!("simctl/{command}"),
            title,
            AtelierToolAction::Simctl,
            format!("sh bin/simctl {command}"),
            guard_capability,
            "control",
            Some(control_repo.to_owned()),
        )
    })
    .collect()
}

fn validation_descriptor(repo: &RepoEntry) -> AtelierToolDescriptor {
    descriptor(
        format!("validation/{}", repo.name),
        format!("Validate {}", repo.name),
        AtelierToolAction::Validate,
        repo.validation_command.clone(),
        format!("RunValidation({})", repo.name),
        "validate",
        Some(repo.name.clone()),
    )
}

fn docs_descriptor(repo: &RepoEntry) -> AtelierToolDescriptor {
    descriptor(
        format!("docs/{}", repo.name),
        format!("Regenerate docs for {}", repo.name),
        AtelierToolAction::Docs,
        repo.docs_command.clone(),
        format!("RegenDocs({})", repo.name),
        "docs",
        Some(repo.name.clone()),
    )
}

fn docs_regeneration_descriptor(repo: &RepoEntry) -> AtelierToolDescriptor {
    let mut descriptor = descriptor(
        format!("docs-regeneration/{}", repo.name),
        format!("Apply generated-doc policy for {}", repo.name),
        AtelierToolAction::DocsRegenerate,
        repo.docs_command.clone(),
        format!("RegenDocs({})", repo.name),
        "docs",
        Some(repo.name.clone()),
    );
    descriptor.refuses_generated_doc_hand_edit = repo.contains_code;
    descriptor
}

fn pin_descriptors(repo: &RepoEntry) -> Vec<AtelierToolDescriptor> {
    [
        (AtelierToolAction::PinPropose, "Propose"),
        (AtelierToolAction::PinPreview, "Preview"),
        (AtelierToolAction::PinApply, "Apply"),
    ]
    .into_iter()
    .map(|(action, title)| {
        let mut descriptor = descriptor(
            format!("pin/{}/{}", action.as_str(), repo.name),
            format!("{title} {} pin update", repo.name),
            action,
            format!(
                "{} repos.toml {} commit <pushed-upstream-commit>",
                action.as_str(),
                repo.name
            ),
            "PlanPin".to_owned(),
            "pin",
            Some(repo.name.clone()),
        );
        descriptor.requires_pushed_commit = true;
        descriptor
    })
    .collect()
}

fn descriptor(
    id: impl Into<String>,
    title: impl Into<String>,
    action: AtelierToolAction,
    command: impl Into<String>,
    guard_capability: impl Into<String>,
    evidence_kind: impl Into<String>,
    repo: Option<String>,
) -> AtelierToolDescriptor {
    let id = id.into();
    let command = command.into();
    AtelierToolDescriptor {
        title: title.into(),
        action,
        command: command.clone(),
        guard_capability: guard_capability.into(),
        evidence_kind: evidence_kind.into(),
        repo,
        envelope_command: command,
        envelope_exit_status_field: "exit-status".to_owned(),
        envelope_log_path: format!(".sim/atelier/logs/{}.log", id.replace('/', "-")),
        id,
        refuses_generated_doc_hand_edit: false,
        requires_pushed_commit: false,
    }
}

fn validate_descriptors(
    descriptors: &[AtelierToolDescriptor],
    control_repo: Option<&str>,
) -> Result<(), String> {
    for descriptor in descriptors {
        let command = descriptor.command.to_ascii_lowercase();
        if command.contains("--mirror")
            || command.contains("publish_to_github = true")
            || command.contains("git remote add github")
        {
            return Err(format!(
                "{} references a forbidden mirror or GitHub operation",
                descriptor.id
            ));
        }
        if descriptor.command.contains(".meta-workspace")
            && descriptor.action != AtelierToolAction::Simctl
        {
            return Err(format!(
                "{} references .meta-workspace as a tool command",
                descriptor.id
            ));
        }
        if control_repo.is_some()
            && descriptor.repo.as_deref() == control_repo
            && descriptor.command.contains("cargo")
            && !matches!(
                descriptor.action,
                AtelierToolAction::Validate | AtelierToolAction::Docs
            )
        {
            return Err(format!(
                "{} would run Rust tooling from the control-plane repo",
                descriptor.id
            ));
        }
    }
    Ok(())
}

fn catalog_json(
    options: &AtelierToolsOptions,
    manifest_path: &std::path::Path,
    descriptors: &[AtelierToolDescriptor],
) -> Value {
    json!({
        "schema": SCHEMA,
        "source_policy": {
            "repos_manifest": manifest_display(&options.control_root, manifest_path),
            "generated_roots": [".meta-workspace/"],
            "editable_roots_include_meta_workspace": false,
            "github_mirror_operations_allowed": false,
            "control_repo_rust_code_allowed": false,
        },
        "summary": {
            "descriptors": descriptors.len(),
            "repo_scoped": descriptors.iter().filter(|descriptor| descriptor.repo.is_some()).count(),
        },
        "descriptors": descriptors.iter().map(AtelierToolDescriptor::to_json).collect::<Vec<_>>(),
    })
}

fn manifest_display(control_root: &std::path::Path, manifest_path: &std::path::Path) -> String {
    manifest_path
        .strip_prefix(control_root)
        .unwrap_or(manifest_path)
        .display()
        .to_string()
}

fn pretty_json(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|mut text| {
            text.push('\n');
            text
        })
        .map_err(|err| format!("render atelier tools json: {err}"))
}

fn print_usage() {
    println!(
        "usage: xtask atelier-tools [--control-root PATH] [--repos-manifest PATH] [--repo NAME] [--cache PATH] [--json] [--check] [--refresh-only]"
    );
}

fn next_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    next_string(args, flag).map(PathBuf::from)
}

fn next_string(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}
