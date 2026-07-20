use std::path::{Path, PathBuf};

use super::super::{
    guard::{GuidelineFinding, GuidelineRule, GuidelineSeverity},
    index_manifest::RepoEntry,
};
use super::{KERNEL_BOUNDARY_NEEDLES, PRESENT_TENSE_NEEDLES, files};

pub(super) fn scan_repo(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
) -> Result<Vec<GuidelineFinding>, String> {
    let files = files::listed_files(repo)?;
    let mut findings = Vec::new();
    findings.extend(rule_ascii(repo, rules, &files)?);
    findings.extend(rule_present_tense(repo, rules, &files)?);
    findings.extend(rule_generated_docs(repo, rules)?);
    findings.extend(rule_code_free(repo, rules, &files));
    findings.extend(rule_local_path_deps(repo, rules, &files)?);
    findings.extend(rule_meta_workspace(repo, rules));
    findings.extend(rule_remote_policy(repo, rules)?);
    findings.extend(rule_file_size(repo, rules, &files)?);
    findings.extend(rule_kernel_boundary(repo, rules, &files)?);
    Ok(findings)
}

fn rule_ascii(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
    listed: &[PathBuf],
) -> Result<Vec<GuidelineFinding>, String> {
    let rule = by_id(rules, "ascii-source-markdown");
    let mut findings = Vec::new();
    for file in listed.iter().filter(|path| files::is_text_file(path)) {
        let text = files::read_repo_text(repo, file)?;
        if let Some(index) = text.find(|ch: char| !ch.is_ascii()) {
            findings.push(finding(
                rule,
                repo,
                Some(file),
                rule.severity,
                format!(
                    "non-ASCII text at line {}",
                    files::line_number(&text, index)
                ),
            ));
        }
    }
    Ok(findings)
}

fn rule_present_tense(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
    listed: &[PathBuf],
) -> Result<Vec<GuidelineFinding>, String> {
    if repo.kind != "code" {
        return Ok(Vec::new());
    }
    let rule = by_id(rules, "present-tense-public-docs");
    let mut findings = Vec::new();
    for file in listed
        .iter()
        .filter(|path| files::is_public_doc_or_comment_file(path))
    {
        let text = files::read_repo_text(repo, file)?;
        for (line_index, line) in text.lines().enumerate() {
            if file.extension().and_then(|ext| ext.to_str()) == Some("rs")
                && !line.trim_start().starts_with("//")
            {
                continue;
            }
            if let Some(needle) = PRESENT_TENSE_NEEDLES
                .iter()
                .find(|needle| files::line_contains(line, needle))
            {
                findings.push(finding(
                    rule,
                    repo,
                    Some(file),
                    rule.severity,
                    format!("line {} contains `{needle}`", line_index + 1),
                ));
                break;
            }
        }
    }
    Ok(findings)
}

fn rule_generated_docs(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
) -> Result<Vec<GuidelineFinding>, String> {
    if repo.kind != "code" || !repo.checkout_path.join("docs").is_dir() {
        return Ok(Vec::new());
    }
    let rule = by_id(rules, "generated-docs-clean");
    let dirty_docs = files::git_status_paths(&repo.checkout_path, "docs")?
        .into_iter()
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    Ok(dirty_docs
        .into_iter()
        .map(|path| {
            finding(
                rule,
                repo,
                Some(Path::new(&path)),
                rule.severity,
                "generated docs have uncommitted changes".to_owned(),
            )
        })
        .collect())
}

fn rule_code_free(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
    listed: &[PathBuf],
) -> Vec<GuidelineFinding> {
    if repo.contains_code {
        return Vec::new();
    }
    let rule = by_id(rules, "code-free-control-repos");
    let mut findings = Vec::new();
    for path in ["Cargo.toml", "src", "crates"] {
        if repo.checkout_path.join(path).exists() {
            findings.push(finding(
                rule,
                repo,
                Some(Path::new(path)),
                rule.severity,
                "code-free repo contains Rust project structure".to_owned(),
            ));
        }
    }
    for file in listed
        .iter()
        .filter(|path| files::is_source_rust_file(path))
    {
        findings.push(finding(
            rule,
            repo,
            Some(file),
            rule.severity,
            "code-free repo contains a Rust source file".to_owned(),
        ));
    }
    findings
}

fn rule_local_path_deps(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
    listed: &[PathBuf],
) -> Result<Vec<GuidelineFinding>, String> {
    if repo.kind != "code" {
        return Ok(Vec::new());
    }
    let rule = by_id(rules, "no-local-public-path-deps");
    let mut findings = Vec::new();
    for file in listed
        .iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
    {
        let text = files::read_repo_text(repo, file)?;
        for (line_index, line) in text.lines().enumerate() {
            if let Some(path) = files::dependency_path_value(line)
                && path.starts_with('/')
            {
                findings.push(finding(
                    rule,
                    repo,
                    Some(file),
                    rule.severity,
                    format!(
                        "line {} uses local path dependency `{path}`",
                        line_index + 1
                    ),
                ));
            }
        }
    }
    Ok(findings)
}

fn rule_meta_workspace(repo: &RepoEntry, rules: &[GuidelineRule]) -> Vec<GuidelineFinding> {
    let rule = by_id(rules, "meta-workspace-not-source");
    let mut findings = Vec::new();
    if files::path_has_meta_workspace(&repo.local_path) {
        findings.push(finding(
            rule,
            repo,
            Some(Path::new(&repo.local_path)),
            rule.severity,
            "repo local_path points into .meta-workspace".to_owned(),
        ));
    }
    for source_path in &repo.source_paths {
        if files::path_has_meta_workspace(source_path) {
            findings.push(finding(
                rule,
                repo,
                Some(Path::new(source_path)),
                rule.severity,
                "repo source_paths includes .meta-workspace".to_owned(),
            ));
        }
    }
    findings
}

fn rule_remote_policy(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
) -> Result<Vec<GuidelineFinding>, String> {
    let rule = by_id(rules, "remote-policy");
    let mut findings = Vec::new();
    if repo.publish_to_github {
        findings.push(finding(
            rule,
            repo,
            Some(Path::new("repos.toml")),
            rule.severity,
            "publish_to_github is true".to_owned(),
        ));
    }
    for remote in files::git_remotes(&repo.checkout_path)? {
        let lower = remote.to_ascii_lowercase();
        if repo.kind == "private" && lower.contains("github.com") {
            findings.push(finding(
                rule,
                repo,
                Some(Path::new(".git/config")),
                rule.severity,
                format!("private repo has GitHub remote: {remote}"),
            ));
        }
        if matches!(repo.kind.as_str(), "code" | "frontpage")
            && is_public_mirror_push_remote(&lower)
        {
            findings.push(finding(
                rule,
                repo,
                Some(Path::new(".git/config")),
                rule.severity,
                format!("public mirror push remote is configured: {remote}"),
            ));
        }
    }
    Ok(findings)
}

fn is_public_mirror_push_remote(remote: &str) -> bool {
    remote.ends_with("(push)")
        && remote.contains("/sim-nest/")
        && !remote.contains("github.com/sim-nest/")
}

fn rule_file_size(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
    listed: &[PathBuf],
) -> Result<Vec<GuidelineFinding>, String> {
    if !repo.contains_code {
        return Ok(Vec::new());
    }
    let rule = by_id(rules, "rust-file-size-policy");
    let mut findings = Vec::new();
    for file in listed.iter().filter(|path| files::is_rust_file(path)) {
        let text = files::read_repo_text(repo, file)?;
        let lines = text.lines().count();
        let entrypoint = file
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| matches!(name, "lib.rs" | "main.rs" | "mod.rs"));
        let (soft, hard) = if entrypoint { (150, 250) } else { (500, 700) };
        if lines > hard {
            findings.push(finding(
                rule,
                repo,
                Some(file),
                GuidelineSeverity::Error,
                format!("{lines} lines exceeds hard limit {hard}"),
            ));
        } else if lines > soft {
            findings.push(finding(
                rule,
                repo,
                Some(file),
                GuidelineSeverity::Warning,
                format!("{lines} lines exceeds soft target {soft}"),
            ));
        }
    }
    Ok(findings)
}

fn rule_kernel_boundary(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
    listed: &[PathBuf],
) -> Result<Vec<GuidelineFinding>, String> {
    if repo.name != "sim-kernel" {
        return Ok(Vec::new());
    }
    let rule = by_id(rules, "kernel-boundary-warning");
    let mut findings = Vec::new();
    for file in listed.iter().filter(|path| files::is_rust_file(path)) {
        let text = files::read_repo_text(repo, file)?;
        if let Some(needle) = KERNEL_BOUNDARY_NEEDLES
            .iter()
            .find(|needle| files::line_contains(&text, needle))
        {
            findings.push(finding(
                rule,
                repo,
                Some(file),
                rule.severity,
                format!("kernel source mentions `{needle}`"),
            ));
        }
    }
    Ok(findings)
}

fn finding(
    rule: &GuidelineRule,
    repo: &RepoEntry,
    path: Option<&Path>,
    severity: GuidelineSeverity,
    evidence: String,
) -> GuidelineFinding {
    GuidelineFinding {
        rule_id: rule.id.to_owned(),
        title: rule.title.to_owned(),
        severity,
        source_contract: rule.source_contract.to_owned(),
        location: match path {
            Some(path) => format!(
                "{}:{}",
                repo.name,
                path.to_string_lossy().replace('\\', "/")
            ),
            None => repo.name.clone(),
        },
        evidence,
        gated_capability: capability_for(rule, repo),
        quick_fix: rule.quick_fix.map(str::to_owned),
    }
}

fn capability_for(rule: &GuidelineRule, repo: &RepoEntry) -> String {
    match rule.gated_capability {
        "EditRepo(*)" => format!("EditRepo({})", repo.name),
        "RegenDocs(*)" => format!("RegenDocs({})", repo.name),
        other => other.to_owned(),
    }
}

fn by_id<'a>(rules: &'a [GuidelineRule], id: &str) -> &'a GuidelineRule {
    rules
        .iter()
        .find(|rule| rule.id == id)
        .expect("guard rule catalog is missing a rule")
}
