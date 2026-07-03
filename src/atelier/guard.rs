//! Guideline Firewall rule catalog and runner.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::{guard_scan::scan_repo, index_manifest::read_repos_manifest, io::display_io};

/// Severity reported by a Guideline Firewall rule or finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuidelineSeverity {
    /// Informational finding.
    Info,
    /// Warning finding; the runner reports it without failing normal operation.
    Warning,
    /// Error finding; `atelier-guard --check` fails when any error is present.
    Error,
}

impl GuidelineSeverity {
    /// Returns the stable lowercase severity label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Machine-readable Guideline Firewall rule metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidelineRule {
    /// Stable rule id.
    pub id: &'static str,
    /// Human-facing rule title.
    pub title: &'static str,
    /// Repository contract or red-line source that owns the rule.
    pub source_contract: &'static str,
    /// Default severity for findings produced by this rule.
    pub severity: GuidelineSeverity,
    /// Scope of files, manifests, or repositories inspected by the rule.
    pub scope: &'static str,
    /// Short description of how the runner detects the rule.
    pub detection: &'static str,
    /// Optional quick-fix text for common violations.
    pub quick_fix: Option<&'static str>,
    /// Guard capability gated by this rule.
    pub gated_capability: &'static str,
}

/// One Guideline Firewall finding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidelineFinding {
    /// Stable rule id.
    pub rule_id: String,
    /// Rule title copied into the finding for standalone reports.
    pub title: String,
    /// Severity for this specific finding.
    pub severity: GuidelineSeverity,
    /// Repository contract or red-line source that owns the rule.
    pub source_contract: String,
    /// Repository and path location.
    pub location: String,
    /// Evidence explaining why the finding was emitted.
    pub evidence: String,
    /// Guard capability gated by this finding.
    pub gated_capability: String,
    /// Optional quick-fix text.
    pub quick_fix: Option<String>,
}

/// Options for `atelier-guard`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtelierGuardOptions {
    /// Control-plane root used to resolve `repos.toml` and sibling checkouts.
    pub control_root: PathBuf,
    /// Optional manifest path. Defaults to `repos.toml` under `control_root`.
    pub repos_manifest: Option<PathBuf>,
    /// Optional repository filter.
    pub repo_filter: Option<String>,
    /// Emit JSON instead of text.
    pub json: bool,
    /// Fail the command when error findings are present.
    pub check: bool,
}

impl Default for AtelierGuardOptions {
    fn default() -> Self {
        Self {
            control_root: PathBuf::from("."),
            repos_manifest: None,
            repo_filter: None,
            json: false,
            check: false,
        }
    }
}

/// Summary of an `atelier-guard` run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtelierGuardReport {
    /// Rule catalog used for the run.
    pub rules: Vec<GuidelineRule>,
    /// Findings emitted by the run.
    pub findings: Vec<GuidelineFinding>,
}

impl AtelierGuardReport {
    /// Counts error findings.
    pub fn error_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == GuidelineSeverity::Error)
            .count()
    }

    /// Counts warning findings.
    pub fn warning_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == GuidelineSeverity::Warning)
            .count()
    }
}

/// Runs the Guideline Firewall rule engine.
pub fn atelier_guard(options: AtelierGuardOptions) -> Result<AtelierGuardReport, String> {
    let manifest_path = options
        .repos_manifest
        .clone()
        .unwrap_or_else(|| options.control_root.join("repos.toml"));
    let mut repos = read_repos_manifest(&options.control_root, &manifest_path)?;
    if let Some(filter) = &options.repo_filter {
        repos.retain(|repo| repo.name == *filter);
    }
    let rules = guideline_rules();
    let mut findings = Vec::new();
    for repo in &repos {
        findings.extend(scan_repo(repo, &rules)?);
    }
    Ok(AtelierGuardReport { rules, findings })
}

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let mut options = AtelierGuardOptions {
        control_root: std::env::current_dir().map_err(display_io)?,
        ..AtelierGuardOptions::default()
    };
    let mut args = args.into_iter().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--control-root" => options.control_root = next_path(&mut args, "--control-root")?,
            "--repos-manifest" => {
                options.repos_manifest = Some(next_path(&mut args, "--repos-manifest")?);
            }
            "--repo" => options.repo_filter = Some(next_string(&mut args, "--repo")?),
            "--json" => options.json = true,
            "--check" => options.check = true,
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown atelier-guard option: {other}")),
        }
    }

    let report = atelier_guard(options.clone())?;
    if options.json {
        println!("{}", pretty_json(&report_json(&report))?);
    } else {
        print_text_report(&report);
    }
    if options.check && report.error_count() > 0 {
        return Err(format!(
            "atelier-guard found {} error(s)",
            report.error_count()
        ));
    }
    Ok(())
}

fn guideline_rules() -> Vec<GuidelineRule> {
    vec![
        rule(
            "ascii-source-markdown",
            "ASCII-only source and Markdown",
            "R8",
            GuidelineSeverity::Error,
            "tracked source and Markdown text",
            "scan text files for non-ASCII bytes",
            Some("Rewrite the file as ASCII text."),
            "EditRepo(*)",
        ),
        rule(
            "present-tense-public-docs",
            "Public docs use present-tense product language",
            "AGENTS Documentation Rules",
            GuidelineSeverity::Warning,
            "public README, generated Markdown, and Rust comments",
            "scan public-facing text for roadmap, history, and future wording",
            Some("Move roadmap/history language to the control-plane docs (docs/future or docs/history)."),
            "EditRepo(*)",
        ),
        rule(
            "generated-docs-clean",
            "Generated docs stay generated",
            "R9",
            GuidelineSeverity::Error,
            "public repo docs/ lanes",
            "check git status for modified or untracked docs files",
            Some("Run the repo docs command instead of hand-editing docs/."),
            "RegenDocs(*)",
        ),
        rule(
            "code-free-control-repos",
            "Code-free repos stay Rust-code-free",
            "R1",
            GuidelineSeverity::Error,
            "code-free repos",
            "reject Cargo manifests, src/, crates/, and Rust files",
            Some("Move Rust code to the owning public code repo."),
            "EditRepo(*)",
        ),
        rule(
            "no-local-public-path-deps",
            "Public main branches avoid local path dependencies",
            "R5",
            GuidelineSeverity::Error,
            "public Cargo manifests",
            "scan Cargo.toml dependency tables for local path values",
            Some("Use published versions or generated meta-workspace overrides."),
            "EditRepo(*)",
        ),
        rule(
            "meta-workspace-not-source",
            ".meta-workspace is not an editable source root",
            "AGENTS generated meta-workspace rule",
            GuidelineSeverity::Error,
            "repos.toml local_path and source_paths",
            "reject manifest paths containing .meta-workspace",
            Some("Edit the owning sibling checkout and regenerate .meta-workspace."),
            "EditRepo(*)",
        ),
        rule(
            "no-github-work",
            "No GitHub remotes, publish flags, or mirror work",
            "R2 and Stage 1 invariant",
            GuidelineSeverity::Error,
            "git remotes and repos.toml publish flags",
            "inspect git remotes and publish_to_github",
            Some("Keep work on the upstream remote and leave publish_to_github false."),
            "PlanPin",
        ),
        rule(
            "rust-file-size-policy",
            "Rust file-size policy",
            "AGENTS Code Organization Policy",
            GuidelineSeverity::Warning,
            "Rust source files",
            "count lines against entrypoint and general soft/hard limits",
            Some("Split the file by responsibility before adding more logic."),
            "EditRepo(*)",
        ),
        rule(
            "kernel-boundary-warning",
            "Kernel boundary warnings",
            "AGENTS Kernel Boundary",
            GuidelineSeverity::Warning,
            "sim-kernel Rust source",
            "scan for concrete parser, number, standard behavior, and parallel-map hints",
            Some("Move concrete behavior to libraries and keep kernel metadata open."),
            "EditRepo(sim-kernel)",
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn rule(
    id: &'static str,
    title: &'static str,
    source_contract: &'static str,
    severity: GuidelineSeverity,
    scope: &'static str,
    detection: &'static str,
    quick_fix: Option<&'static str>,
    gated_capability: &'static str,
) -> GuidelineRule {
    GuidelineRule {
        id,
        title,
        source_contract,
        severity,
        scope,
        detection,
        quick_fix,
        gated_capability,
    }
}

fn report_json(report: &AtelierGuardReport) -> Value {
    json!({
        "schema": "sim.atelier.guard-report.v1",
        "rules": report.rules.iter().map(rule_json).collect::<Vec<_>>(),
        "summary": {
            "rules": report.rules.len(),
            "findings": report.findings.len(),
            "errors": report.error_count(),
            "warnings": report.warning_count(),
        },
        "findings": report.findings.iter().map(finding_json).collect::<Vec<_>>(),
    })
}

fn rule_json(rule: &GuidelineRule) -> Value {
    json!({
        "id": rule.id,
        "title": rule.title,
        "source_contract": rule.source_contract,
        "severity": rule.severity.as_str(),
        "scope": rule.scope,
        "detection": rule.detection,
        "quick_fix": rule.quick_fix,
        "gated_capability": rule.gated_capability,
    })
}

fn finding_json(finding: &GuidelineFinding) -> Value {
    json!({
        "rule_id": finding.rule_id,
        "title": finding.title,
        "severity": finding.severity.as_str(),
        "source_contract": finding.source_contract,
        "location": finding.location,
        "evidence": finding.evidence,
        "gated_capability": finding.gated_capability,
        "quick_fix": finding.quick_fix,
    })
}

fn print_text_report(report: &AtelierGuardReport) {
    println!(
        "atelier-guard: {} rule(s), {} finding(s), {} error(s), {} warning(s)",
        report.rules.len(),
        report.findings.len(),
        report.error_count(),
        report.warning_count()
    );
    for finding in &report.findings {
        println!(
            "{} {} {} capability={}",
            finding.severity.as_str(),
            finding.rule_id,
            finding.location,
            finding.gated_capability
        );
        println!("  source: {}", finding.source_contract);
        println!("  evidence: {}", finding.evidence);
        if let Some(quick_fix) = &finding.quick_fix {
            println!("  fix: {quick_fix}");
        }
    }
}

fn pretty_json(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|mut text| {
            text.push('\n');
            text
        })
        .map_err(|err| format!("render atelier guard json: {err}"))
}

fn print_usage() {
    println!(
        "usage: xtask atelier-guard [--control-root PATH] [--repos-manifest PATH] [--repo NAME] [--json] [--check]"
    );
}

fn next_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    next_string(args, flag).map(PathBuf::from)
}

fn next_string(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}
