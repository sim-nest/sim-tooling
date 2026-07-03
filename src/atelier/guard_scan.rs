use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use super::{
    guard::{GuidelineFinding, GuidelineRule, GuidelineSeverity},
    index_manifest::RepoEntry,
    io::display_io,
};

const PRESENT_TENSE_NEEDLES: &[&str] = &[
    "ROADMAP_",
    "REORG_",
    "Phase ",
    "previously",
    "historically",
    "formerly",
    "legacy",
    "migration",
    "migrated",
    "future",
    "planned",
    "not yet complete",
    "will be added",
    "TODO(roadmap)",
];

const KERNEL_BOUNDARY_NEEDLES: &[&str] = &[
    "parse_json",
    "parse_lisp",
    "JsonParser",
    "LispParser",
    "StandardArithmetic",
    "BigInt",
    "BigRational",
    "parallel map",
];

pub(super) fn scan_repo(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
) -> Result<Vec<GuidelineFinding>, String> {
    let files = listed_files(repo)?;
    let mut findings = Vec::new();
    findings.extend(rule_ascii(repo, rules, &files)?);
    findings.extend(rule_present_tense(repo, rules, &files)?);
    findings.extend(rule_generated_docs(repo, rules)?);
    findings.extend(rule_code_free(repo, rules, &files));
    findings.extend(rule_local_path_deps(repo, rules, &files)?);
    findings.extend(rule_meta_workspace(repo, rules));
    findings.extend(rule_no_github(repo, rules)?);
    findings.extend(rule_file_size(repo, rules, &files)?);
    findings.extend(rule_kernel_boundary(repo, rules, &files)?);
    Ok(findings)
}

fn rule_ascii(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
    files: &[PathBuf],
) -> Result<Vec<GuidelineFinding>, String> {
    let rule = by_id(rules, "ascii-source-markdown");
    let mut findings = Vec::new();
    for file in files.iter().filter(|path| is_text_file(path)) {
        let text = read_repo_text(repo, file)?;
        if let Some(index) = text.find(|ch: char| !ch.is_ascii()) {
            findings.push(finding(
                rule,
                repo,
                Some(file),
                rule.severity,
                format!("non-ASCII text at line {}", line_number(&text, index)),
            ));
        }
    }
    Ok(findings)
}

fn rule_present_tense(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
    files: &[PathBuf],
) -> Result<Vec<GuidelineFinding>, String> {
    if repo.kind != "code" {
        return Ok(Vec::new());
    }
    let rule = by_id(rules, "present-tense-public-docs");
    let mut findings = Vec::new();
    for file in files
        .iter()
        .filter(|path| is_public_doc_or_comment_file(path))
    {
        let text = read_repo_text(repo, file)?;
        for (line_index, line) in text.lines().enumerate() {
            if file.extension().and_then(|ext| ext.to_str()) == Some("rs")
                && !line.trim_start().starts_with("//")
            {
                continue;
            }
            if let Some(needle) = PRESENT_TENSE_NEEDLES
                .iter()
                .find(|needle| line_contains(line, needle))
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
    let dirty_docs = git_status_paths(&repo.checkout_path, "docs")?
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
    files: &[PathBuf],
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
    for file in files.iter().filter(|path| is_source_rust_file(path)) {
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
    files: &[PathBuf],
) -> Result<Vec<GuidelineFinding>, String> {
    if repo.kind != "code" {
        return Ok(Vec::new());
    }
    let rule = by_id(rules, "no-local-public-path-deps");
    let mut findings = Vec::new();
    for file in files
        .iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
    {
        let text = read_repo_text(repo, file)?;
        for (line_index, line) in text.lines().enumerate() {
            if let Some(path) = dependency_path_value(line)
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
    if path_has_meta_workspace(&repo.local_path) {
        findings.push(finding(
            rule,
            repo,
            Some(Path::new(&repo.local_path)),
            rule.severity,
            "repo local_path points into .meta-workspace".to_owned(),
        ));
    }
    for source_path in &repo.source_paths {
        if path_has_meta_workspace(source_path) {
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

fn rule_no_github(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
) -> Result<Vec<GuidelineFinding>, String> {
    let rule = by_id(rules, "no-github-work");
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
    for remote in git_remotes(&repo.checkout_path)? {
        if remote.to_ascii_lowercase().contains("github.com") {
            findings.push(finding(
                rule,
                repo,
                Some(Path::new(".git/config")),
                rule.severity,
                format!("git remote references GitHub: {remote}"),
            ));
        }
    }
    Ok(findings)
}

fn rule_file_size(
    repo: &RepoEntry,
    rules: &[GuidelineRule],
    files: &[PathBuf],
) -> Result<Vec<GuidelineFinding>, String> {
    if !repo.contains_code {
        return Ok(Vec::new());
    }
    let rule = by_id(rules, "rust-file-size-policy");
    let mut findings = Vec::new();
    for file in files.iter().filter(|path| is_rust_file(path)) {
        let text = read_repo_text(repo, file)?;
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
    files: &[PathBuf],
) -> Result<Vec<GuidelineFinding>, String> {
    if repo.name != "sim-kernel" {
        return Ok(Vec::new());
    }
    let rule = by_id(rules, "kernel-boundary-warning");
    let mut findings = Vec::new();
    for file in files.iter().filter(|path| is_rust_file(path)) {
        let text = read_repo_text(repo, file)?;
        if let Some(needle) = KERNEL_BOUNDARY_NEEDLES
            .iter()
            .find(|needle| line_contains(&text, needle))
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

fn listed_files(repo: &RepoEntry) -> Result<Vec<PathBuf>, String> {
    if !repo.checkout_path.is_dir() {
        return Ok(Vec::new());
    }
    if repo.checkout_path.join(".git").is_dir() {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo.checkout_path)
            .arg("ls-files")
            .output()
            .map_err(display_io)?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .filter(|path| should_scan_path(path))
                .collect());
        }
    }
    let mut files = Vec::new();
    collect_files(&repo.checkout_path, Path::new(""), &mut files)?;
    Ok(files)
}

fn collect_files(root: &Path, relative: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root.join(relative)).map_err(display_io)? {
        let entry = entry.map_err(display_io)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if should_skip_dir(name.as_ref()) {
            continue;
        }
        let relative_path = relative.join(name.as_ref());
        if entry.file_type().map_err(display_io)?.is_dir() {
            collect_files(root, &relative_path, files)?;
        } else if should_scan_path(&relative_path) {
            files.push(relative_path);
        }
    }
    Ok(())
}

fn read_repo_text(repo: &RepoEntry, relative: &Path) -> Result<String, String> {
    fs::read_to_string(repo.checkout_path.join(relative)).map_err(display_io)
}

fn git_status_paths(root: &Path, pathspec: &str) -> Result<Vec<String>, String> {
    if !root.join(".git").is_dir() {
        return Ok(Vec::new());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("status")
        .arg("--short")
        .arg("--")
        .arg(pathspec)
        .output()
        .map_err(display_io)?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.get(3..))
        .map(str::to_owned)
        .collect())
}

fn git_remotes(root: &Path) -> Result<Vec<String>, String> {
    if !root.join(".git").is_dir() {
        return Ok(Vec::new());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("remote")
        .arg("-v")
        .output()
        .map_err(display_io)?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect())
}

fn is_text_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if matches!(name, "README" | "README.md" | "Cargo.toml") {
        return true;
    }
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("md" | "rs" | "sh" | "toml" | "txt" | "json" | "yaml" | "yml" | "example")
    )
}

fn is_public_doc_or_comment_file(path: &Path) -> bool {
    if path.file_name().and_then(|name| name.to_str()) == Some("README.md") {
        return true;
    }
    if path.starts_with("docs") && path.extension().and_then(|ext| ext.to_str()) == Some("md") {
        return true;
    }
    path.extension().and_then(|ext| ext.to_str()) == Some("rs")
}

fn is_rust_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("rs")
}

fn is_source_rust_file(path: &Path) -> bool {
    is_rust_file(path) && !path.starts_with("docs")
}

fn should_skip_dir(name: &str) -> bool {
    matches!(name, ".git" | "target" | ".sim" | ".meta-workspace")
        || (name.starts_with('.') && name != ".env.example")
}

fn should_scan_path(path: &Path) -> bool {
    if path.starts_with("target") || path.starts_with(".sim") || path.starts_with(".meta-workspace")
    {
        return false;
    }
    !path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .any(|part| part.starts_with('.') && part != ".env.example")
}

fn dependency_path_value(line: &str) -> Option<&str> {
    let (_, after) = line.split_once("path")?;
    let (_, after_equals) = after.split_once('=')?;
    let trimmed = after_equals.trim_start();
    let quote = trimmed.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &trimmed[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(&rest[..end])
}

fn path_has_meta_workspace(path: &str) -> bool {
    path.split(['/', '\\'])
        .any(|part| part == ".meta-workspace")
}

fn line_contains(line: &str, needle: &str) -> bool {
    if needle
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch == ' ')
    {
        line.contains(needle)
    } else {
        line.to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
    }
}

fn line_number(text: &str, byte_index: usize) -> usize {
    text[..byte_index]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}
