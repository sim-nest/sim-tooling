//! Semantic discovery and permanent enforcement of the platform membrane.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

// Kept private: the public interchange vocabulary is owned by sim-index-core;
// this scanner emits its stable labels and does not expose a parallel API.
#[derive(Clone, Copy)]
enum HostBindingKind {
    Call,
    Dependency,
    AbiDeclaration,
    ForeignImplementation,
    ArtifactImport,
    Subprocess,
}
impl HostBindingKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Dependency => "dependency",
            Self::AbiDeclaration => "abi-declaration",
            Self::ForeignImplementation => "foreign-implementation",
            Self::ArtifactImport => "artifact-import",
            Self::Subprocess => "subprocess",
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum HostSourceRole {
    Pure,
    Capsule,
    Bootstrap,
    Tool,
    Test,
}
impl HostSourceRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::Capsule => "capsule",
            Self::Bootstrap => "bootstrap",
            Self::Tool => "tool",
            Self::Test => "test",
        }
    }
}

#[derive(Serialize)]
struct Ledger {
    schema: &'static str,
    generated_by: &'static str,
    semantic_files: usize,
    totals: BTreeMap<&'static str, usize>,
    structural_totals: BTreeMap<&'static str, usize>,
    membrane_violations: Vec<String>,
    facts: Vec<Fact>,
    index_facts: Vec<IndexFact>,
}

#[derive(Clone, Serialize)]
struct Fact {
    span_digest: String,
    repository: String,
    package: String,
    target: String,
    module: String,
    file: String,
    line: usize,
    binding_kind: &'static str,
    role: &'static str,
    test_member: bool,
    provider: String,
    evidence: String,
    owner_phase: &'static str,
    normalization_move: &'static str,
}

#[derive(Serialize)]
struct IndexFact {
    anchor: String,
    kind: &'static str,
    role: &'static str,
    provider: String,
    service: String,
    evidence: String,
    fact_class: &'static str,
    product_reachable: bool,
}

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let mut repos = Vec::new();
    let mut out = None;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--repo" => {
                let value = args.get(index + 1).ok_or("--repo requires name=path")?;
                let (name, path) = value.split_once('=').ok_or("--repo requires name=path")?;
                repos.push((name.to_owned(), PathBuf::from(path)));
                index += 2;
            }
            "--out" => {
                out = Some(PathBuf::from(
                    args.get(index + 1).ok_or("--out requires a path")?,
                ));
                index += 2;
            }
            other => return Err(format!("unknown platform-inventory argument: {other}")),
        }
    }
    if repos.is_empty() {
        return Err("platform-inventory requires at least one --repo".into());
    }
    let out = out.ok_or("platform-inventory requires --out")?;
    let ledger = scan(&repos)?;
    let rendered = serde_json::to_string_pretty(&ledger).map_err(|e| e.to_string())? + "\n";
    if out.as_os_str() == "-" {
        print!("{rendered}");
    } else {
        fs::write(out, rendered).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn scan(repos: &[(String, PathBuf)]) -> Result<Ledger, String> {
    let mut facts = Vec::new();
    let mut semantic_files = 0;
    for (repo, root) in repos {
        let mut files = Vec::new();
        collect_files(root, &mut files)?;
        files.sort();
        for path in files {
            let Some(language) = language(&path) else {
                continue;
            };
            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(_) => continue,
            };
            semantic_files += 1;
            scan_file(repo, root, &path, language, &text, &mut facts);
        }
    }
    facts.sort_by(|a, b| {
        (&a.repository, &a.file, a.line, a.binding_kind, &a.evidence).cmp(&(
            &b.repository,
            &b.file,
            b.line,
            b.binding_kind,
            &b.evidence,
        ))
    });
    facts.dedup_by(|a, b| a.span_digest == b.span_digest);
    let mut totals = BTreeMap::new();
    for fact in &facts {
        *totals.entry(fact.binding_kind).or_insert(0) += 1;
    }
    let mut structural_totals = BTreeMap::new();
    for fact in &facts {
        *structural_totals.entry(fact_class(fact.role)).or_insert(0) += 1;
    }
    let membrane_violations = validate_membrane(&facts);
    let index_facts = facts
        .iter()
        .map(|fact| IndexFact {
            anchor: format!("anchor/host/{}/{}", fact.repository, fact.span_digest),
            kind: fact.binding_kind,
            role: fact.role,
            provider: fact.provider.clone(),
            service: service(&fact.provider).to_owned(),
            evidence: fact.evidence.clone(),
            fact_class: fact_class(fact.role),
            product_reachable: false,
        })
        .collect();
    Ok(Ledger {
        schema: "sim.platform-membrane/v1",
        generated_by: "sim-tooling platform-inventory v2",
        semantic_files,
        totals,
        structural_totals,
        membrane_violations,
        facts,
        index_facts,
    })
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            if matches!(
                path.file_name().and_then(|v| v.to_str()),
                Some(".git" | "target" | ".sim" | "docs")
            ) {
                continue;
            }
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn language(path: &Path) -> Option<&'static str> {
    // A lockfile is a resolved build input, not a target dependency edge. Its
    // transitive package names cannot establish product reachability.
    if path.file_name().and_then(|value| value.to_str()) == Some("Cargo.lock") {
        return None;
    }
    if let Some("Cargo.toml" | "Cargo.lock" | "package.json" | "Package.swift" | "CMakeLists.txt") =
        path.file_name().and_then(|v| v.to_str())
    {
        return Some("manifest");
    }
    match path.extension().and_then(|v| v.to_str())? {
        "rs" => Some("rust"),
        "kt" | "kts" => Some("kotlin"),
        "swift" => Some("swift"),
        "js" | "mjs" | "cjs" | "ts" => Some("javascript"),
        "lua" => Some("lua"),
        "c" | "h" | "cc" | "cpp" | "cxx" | "m" | "mm" => Some("native"),
        "sh" | "bash" => Some("shell"),
        "toml" | "json" => Some("manifest"),
        _ => None,
    }
}

fn scan_file(
    repo: &str,
    root: &Path,
    path: &Path,
    language: &str,
    text: &str,
    out: &mut Vec<Fact>,
) {
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let package = package_name(&rel);
    let target = target_name(&rel, language);
    let mut effective_target = target.clone();
    let mut rust_aliases: Vec<(String, &'static str)> = Vec::new();
    let mut cfg_test_depth = None;
    let mut depth = 0usize;
    let mut pending_test = false;
    for (offset, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if language == "manifest" && line.starts_with("[target.") {
            effective_target = line.trim_matches(&['[', ']'][..]).to_owned();
        }
        if language == "rust"
            && line.starts_with("use std::")
            && let Some((path, alias)) = line.trim_end_matches(';').split_once(" as ")
        {
            let provider = if path.contains("::fs") {
                Some("os/filesystem")
            } else if path.contains("::net") {
                Some("os/network")
            } else if path.contains("::process") {
                Some("os/process")
            } else {
                None
            };
            if let Some(provider) = provider {
                rust_aliases.push((format!("{}::", alias.trim()), provider));
            }
        }
        if language == "rust" && line.starts_with("#[cfg") && line.contains("test") {
            pending_test = true;
        }
        if pending_test
            && (line.starts_with("mod ") || line.contains(" mod "))
            && line.contains('{')
        {
            cfg_test_depth = Some(depth + raw.matches('{').count());
            pending_test = false;
        }
        let test_member = cfg_test_depth.is_some()
            || matches!(effective_target.as_str(), "test" | "example" | "bench");
        if !is_prose(line) {
            for (needle, kind, provider, evidence) in patterns(language) {
                if line.contains(needle) {
                    push_fact(
                        out,
                        repo,
                        &package,
                        &effective_target,
                        &rel,
                        offset + 1,
                        line,
                        kind,
                        role(repo, &package, &effective_target, &rel, test_member),
                        test_member,
                        provider,
                        evidence,
                    );
                }
            }
            for (alias, provider) in &rust_aliases {
                if line.contains(alias) && !line.starts_with("use ") {
                    push_fact(
                        out,
                        repo,
                        &package,
                        &effective_target,
                        &rel,
                        offset + 1,
                        line,
                        HostBindingKind::Call,
                        role(repo, &package, &effective_target, &rel, test_member),
                        test_member,
                        provider,
                        "resolved aliased or re-exported host call",
                    );
                }
            }
        }
        depth = depth
            .saturating_add(raw.matches('{').count())
            .saturating_sub(raw.matches('}').count());
        if cfg_test_depth.is_some_and(|start| depth < start) {
            cfg_test_depth = None;
        }
    }
}

fn patterns(language: &str) -> Vec<(&'static str, HostBindingKind, &'static str, &'static str)> {
    let mut rows = vec![
        (
            "std::process::Command",
            HostBindingKind::Subprocess,
            "os/process",
            "resolved process spawn",
        ),
        (
            "tokio::process::Command",
            HostBindingKind::Subprocess,
            "os/process",
            "resolved async process spawn",
        ),
        (
            "std::fs::",
            HostBindingKind::Call,
            "os/filesystem",
            "resolved filesystem call",
        ),
        (
            "std::net::",
            HostBindingKind::Call,
            "os/network",
            "resolved network call",
        ),
        (
            "std::env::",
            HostBindingKind::Call,
            "os/environment",
            "resolved environment call",
        ),
        (
            "libc::",
            HostBindingKind::Call,
            "os/libc",
            "resolved libc call",
        ),
        (
            "extern \"C\"",
            HostBindingKind::AbiDeclaration,
            "abi/c",
            "foreign ABI declaration",
        ),
        (
            "#[no_mangle]",
            HostBindingKind::ForeignImplementation,
            "abi/c",
            "foreign ABI export",
        ),
        (
            "#[unsafe(no_mangle)]",
            HostBindingKind::ForeignImplementation,
            "abi/c",
            "foreign ABI export",
        ),
    ];
    rows.extend(match language {
        "kotlin" => vec![
            (
                "java.io.",
                HostBindingKind::Call,
                "jvm/io",
                "resolved JVM host call",
            ),
            (
                "java.net.",
                HostBindingKind::Call,
                "jvm/network",
                "resolved JVM host call",
            ),
        ],
        "swift" => vec![
            (
                "Foundation.",
                HostBindingKind::Call,
                "apple/foundation",
                "resolved Foundation call",
            ),
            (
                "@_silgen_name",
                HostBindingKind::AbiDeclaration,
                "abi/c",
                "Swift foreign declaration",
            ),
        ],
        "javascript" => vec![
            (
                "require(\"fs\")",
                HostBindingKind::Dependency,
                "node/filesystem",
                "resolved Node import",
            ),
            (
                "child_process",
                HostBindingKind::Subprocess,
                "node/process",
                "resolved Node process import",
            ),
        ],
        "lua" => vec![
            (
                "os.execute",
                HostBindingKind::Subprocess,
                "os/process",
                "resolved Lua process call",
            ),
            (
                "io.open",
                HostBindingKind::Call,
                "os/filesystem",
                "resolved Lua filesystem call",
            ),
        ],
        "native" => vec![
            (
                "#include <",
                HostBindingKind::Dependency,
                "abi/native",
                "native header import",
            ),
            (
                "dlopen(",
                HostBindingKind::Call,
                "os/dynamic-loader",
                "resolved dynamic loader call",
            ),
        ],
        "shell" => vec![
            (
                "curl ",
                HostBindingKind::Call,
                "os/network",
                "shell network command",
            ),
            (
                "ssh ",
                HostBindingKind::Call,
                "os/network",
                "shell remote command",
            ),
        ],
        "manifest" => vec![
            (
                "target.'cfg(",
                HostBindingKind::Dependency,
                "cargo/target",
                "Cargo target dependency",
            ),
            (
                "libc",
                HostBindingKind::Dependency,
                "os/libc",
                "resolved manifest dependency",
            ),
            (
                "links =",
                HostBindingKind::ArtifactImport,
                "cargo/native-artifact",
                "declared built artifact import",
            ),
            (
                "tokio =",
                HostBindingKind::Dependency,
                "cargo/tokio-host",
                "resolved host-capable dependency",
            ),
            (
                "reqwest",
                HostBindingKind::Dependency,
                "cargo/http-client",
                "resolved host-capable dependency or alias",
            ),
            (
                "name = \"libloading\"",
                HostBindingKind::Dependency,
                "cargo/dynamic-loader",
                "resolved transitive host-capable lock dependency",
            ),
            (
                "name = \"cc\"",
                HostBindingKind::Dependency,
                "cargo/native-build",
                "resolved transitive native build dependency",
            ),
        ],
        _ => Vec::new(),
    });
    rows
}

#[allow(clippy::too_many_arguments)]
fn push_fact(
    out: &mut Vec<Fact>,
    repo: &str,
    package: &str,
    target: &str,
    file: &str,
    line: usize,
    source: &str,
    kind: HostBindingKind,
    role: HostSourceRole,
    test_member: bool,
    provider: &str,
    evidence: &str,
) {
    let move_name = normalization(kind, provider);
    let phase = if matches!(
        role,
        HostSourceRole::Tool | HostSourceRole::Test | HostSourceRole::Capsule
    ) {
        "resolved"
    } else {
        owner_phase(kind, provider)
    };
    let identity = format!("{repo}\0{file}\0{line}\0{}\0{source}", kind.as_str());
    let digest = format!("{:x}", Sha256::digest(identity.as_bytes()));
    out.push(Fact {
        span_digest: digest,
        repository: repo.to_owned(),
        package: package.to_owned(),
        target: target.to_owned(),
        module: file.trim_end_matches(".rs").replace('/', "::"),
        file: file.to_owned(),
        line,
        binding_kind: kind.as_str(),
        role: role.as_str(),
        test_member,
        provider: provider.to_owned(),
        evidence: evidence.to_owned(),
        owner_phase: phase,
        normalization_move: move_name,
    });
}

fn role(repo: &str, package: &str, target: &str, file: &str, test: bool) -> HostSourceRole {
    if test {
        HostSourceRole::Test
    } else if repo == "sim-tooling"
        || target == "build"
        || package == "xtask"
        || file.starts_with("bin/")
        || file.starts_with("xtask/")
    {
        HostSourceRole::Tool
    } else if package.contains("host") || package.contains("platform") {
        HostSourceRole::Capsule
    } else if package == "sim-run" {
        HostSourceRole::Bootstrap
    } else {
        HostSourceRole::Pure
    }
}

fn fact_class(role: &str) -> &'static str {
    match role {
        "tool" => "host-tool",
        "capsule" => "platform-capsule",
        "bootstrap" => "platform-bootstrap",
        "test" => "test-evidence",
        "pure" => "pure-source",
        _ => unreachable!("host source roles are closed"),
    }
}

fn validate_membrane(facts: &[Fact]) -> Vec<String> {
    let mut violations = Vec::new();
    for fact in facts {
        if fact.target.is_empty() || fact.package.is_empty() || fact.provider.is_empty() {
            violations.push(format!("unclassified host fact {}", fact.span_digest));
        }
        if fact.repository == "sim-tooling" && fact.role != "tool" && !fact.test_member {
            violations.push(format!(
                "sim-tooling fact escaped host-tool isolation: {}",
                fact.file
            ));
        }
    }
    violations.sort();
    violations.dedup();
    violations
}
fn package_name(rel: &str) -> String {
    rel.split('/')
        .nth(1)
        .filter(|_| rel.starts_with("crates/"))
        .unwrap_or_else(|| {
            if rel.starts_with("src/") {
                "repo-root"
            } else {
                "workspace"
            }
        })
        .to_owned()
}
fn target_name(rel: &str, language: &str) -> String {
    if rel == "build.rs" || rel.ends_with("/build.rs") {
        "build"
    } else if rel.contains("/tests/") {
        "test"
    } else if rel.contains("/examples/") {
        "example"
    } else if rel.contains("/benches/") {
        "bench"
    } else if language == "manifest" {
        "manifest"
    } else {
        "lib"
    }
    .to_owned()
}
fn is_prose(line: &str) -> bool {
    line.is_empty()
        || line.starts_with("//")
        || line.starts_with("/*")
        || line.starts_with('*')
        || line.starts_with('#') && !line.starts_with("#[")
}
fn owner_phase(kind: HostBindingKind, provider: &str) -> &'static str {
    match (kind, provider) {
        (HostBindingKind::Subprocess, _) => "OS5.10",
        (_, "os/filesystem") => "OS5.11",
        (_, "os/network") => "OS5.12",
        (_, "os/environment") => "OS5.13",
        (HostBindingKind::AbiDeclaration | HostBindingKind::ForeignImplementation, _) => "OS5.26",
        (_, "cargo/target") => "OS5.28",
        (_, "apple/foundation") => "OS5.21",
        (_, "jvm/io" | "jvm/network") => "OS5.22",
        _ => "OS5.31",
    }
}
fn normalization(kind: HostBindingKind, provider: &str) -> &'static str {
    match kind {
        HostBindingKind::Subprocess => "replace with a typed host service",
        HostBindingKind::AbiDeclaration => "bind through the kernel ABI",
        HostBindingKind::ForeignImplementation => "move implementation behind a capsule",
        HostBindingKind::Dependency => "move dependency into the provider closure",
        HostBindingKind::ArtifactImport => "declare the artifact import",
        HostBindingKind::Call if provider == "os/filesystem" => "route through the storage service",
        HostBindingKind::Call if provider == "os/network" => "route through the network service",
        HostBindingKind::Call => "route through a declared host service",
    }
}
fn service(provider: &str) -> &'static str {
    if provider.contains("filesystem") {
        "storage"
    } else if provider.contains("network") {
        "network"
    } else if provider.contains("process") {
        "process"
    } else if provider.contains("abi") {
        "native-abi"
    } else {
        "host"
    }
}

#[cfg(test)]
#[path = "platform_inventory_tests.rs"]
mod tests;
