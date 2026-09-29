// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `check-license`: the three-layer MPL-2.0 license gate for one repository.
//!
//! Layer 1 requires the root `LICENSE` to be the canonical MPL-2.0 text.
//! Layer 3 requires the file notice on every touched first-party source file of
//! a bound type. Untouched notice gaps and layer 2 (license text inside each
//! publishable package) are reported without failing until `--strict`.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::content_digest::content_digest;

mod exclusions;

use exclusions::Exclusions;

/// SHA-256 of the canonical MPL-2.0 `LICENSE` shipped by every SIM repository.
const CANONICAL_LICENSE_SHA256: &str =
    "1f256ecad192880510e84ad60474eab7589218784b9a50bc7ceee34c2b91f1d5";

/// File extensions whose notice is currently required. Other file types join
/// under the `LICENSE-HEADERS-1` sweep.
const BOUND_EXTENSIONS: &[&str] = &["rs"];

/// Lines searched for a notice or a generated marker, after any shebang.
const HEAD_LINES: usize = 12;

const NOTICE: &str = "SPDX-License-Identifier: MPL-2.0";
const SPDX: &str = "SPDX-License-Identifier:";
const GENERATED_MARKER: &str = "@generated";

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = parse_args(&args)?;
    let report = check_license(&options)?;
    print!("{}", report.render(&options));
    let failures = report.failures(&options);
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "check-license: {}",
            failures.join("\ncheck-license: ")
        ))
    }
}

#[derive(Debug)]
pub(crate) struct Options {
    pub(crate) root: PathBuf,
    pub(crate) base: String,
    pub(crate) strict: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Report {
    pub(crate) license_problem: Option<String>,
    pub(crate) touched_missing: Vec<String>,
    pub(crate) untouched_missing: Vec<String>,
    pub(crate) other_license: Vec<String>,
    pub(crate) excluded: usize,
    pub(crate) checked: usize,
    pub(crate) packages_without_license: Vec<String>,
}

impl Report {
    pub(crate) fn failures(&self, options: &Options) -> Vec<String> {
        let mut failures = Vec::new();
        if let Some(problem) = &self.license_problem {
            failures.push(problem.clone());
        }
        failures.extend(
            self.touched_missing
                .iter()
                .map(|path| format!("{path}: touched file lacks the MPL-2.0 notice")),
        );
        if options.strict {
            failures.extend(
                self.untouched_missing
                    .iter()
                    .map(|path| format!("{path}: file lacks the MPL-2.0 notice")),
            );
            failures.extend(
                self.packages_without_license
                    .iter()
                    .map(|path| format!("{path}: package ships no license text")),
            );
        }
        failures
    }

    fn render(&self, options: &Options) -> String {
        let mut out = format!(
            "check-license: {} file(s) checked, {} excluded, {} touched without notice, \
             {} untouched without notice, {} under another license, \
             {} package(s) without license text\n",
            self.checked,
            self.excluded,
            self.touched_missing.len(),
            self.untouched_missing.len(),
            self.other_license.len(),
            self.packages_without_license.len(),
        );
        if !options.strict && !self.untouched_missing.is_empty() {
            out.push_str(
                "check-license: untouched gaps are reported, not failed (LICENSE-HEADERS-1)\n",
            );
        }
        for path in &self.other_license {
            out.push_str(&format!("check-license: other license kept: {path}\n"));
        }
        out
    }
}

pub(crate) fn check_license(options: &Options) -> Result<Report, String> {
    let root = &options.root;
    let exclusions = Exclusions::load(root)?;
    let files = git_lines(
        root,
        &["ls-files", "--cached", "--others", "--exclude-standard"],
    )?;
    let touched = touched_files(root, &options.base)?;

    let mut report = Report {
        license_problem: license_problem(root),
        ..Report::default()
    };
    for rel in files {
        if !is_bound(&rel) {
            continue;
        }
        if exclusions.matches(&rel) {
            report.excluded += 1;
            continue;
        }
        // A deletion still listed by the index is not a file to head.
        let Ok(text) = fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        match classify(&text) {
            Notice::Present => {}
            Notice::Generated => {
                report.excluded += 1;
                continue;
            }
            Notice::OtherLicense => report.other_license.push(rel.clone()),
            Notice::Missing if touched.contains(&rel) => report.touched_missing.push(rel.clone()),
            Notice::Missing => report.untouched_missing.push(rel.clone()),
        }
        report.checked += 1;
    }
    report.packages_without_license = packages_without_license(root)?;
    Ok(report)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Notice {
    Present,
    Generated,
    OtherLicense,
    Missing,
}

pub(crate) fn classify(text: &str) -> Notice {
    let mut lines = text.lines().peekable();
    if lines.peek().is_some_and(|line| line.starts_with("#!")) {
        lines.next();
    }
    let head: Vec<&str> = lines.take(HEAD_LINES).collect();
    let real_lines = lines_outside_raw_strings(&head);
    let comments = || real_lines.iter().filter_map(|line| comment_payload(line));
    if comments().any(|payload| payload == NOTICE) {
        Notice::Present
    } else if comments().any(|payload| payload.starts_with(GENERATED_MARKER)) {
        Notice::Generated
    } else if comments().any(|payload| payload.starts_with(SPDX)) {
        Notice::OtherLicense
    } else {
        Notice::Missing
    }
}

/// `head`, excluding any line that lies entirely inside an open Rust raw
/// string literal (`r"..."`, `r#"..."#`, ... up to 255 `#`s, including the
/// `b`/`br` byte-string and `c`/`cr` C-string prefix forms). A raw string
/// can span multiple lines and its content can contain arbitrary text,
/// including a line that would otherwise look exactly like a real `//`
/// comment declaring the notice or a generated marker; such a line is
/// never a real declaration.
///
/// Scoped, not a full lexer: this tracks only raw-string boundaries, which
/// is what a multi-line embedded fixture (the realistic way this arises in
/// this constellation's own source, including this module's own tests)
/// actually uses. It does not track plain string literals, char literals,
/// or block comments, so a contrived construction combining those with
/// `//`-prefixed text could still evade detection; the threat model here is
/// an honest source tree that happens to embed notice-like text as data,
/// not a deliberate, sophisticated attempt to defeat this gate.
fn lines_outside_raw_strings<'a>(head: &[&'a str]) -> Vec<&'a str> {
    let mut out = Vec::with_capacity(head.len());
    let mut open_hashes: Option<usize> = None;
    for &line in head {
        if open_hashes.is_none() {
            out.push(line);
        }
        open_hashes = raw_string_state_after(line, open_hashes);
    }
    out
}

/// The raw-string nesting state after scanning the whole of `line` left to
/// right, given the state entering it -- handling any number of open/close
/// boundaries within the one line, including a raw string that both opens
/// and closes on it (possibly followed by another that opens again).
fn raw_string_state_after(line: &str, mut state: Option<usize>) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut pos = 0;
    loop {
        state = match state {
            Some(hashes) => match raw_string_close_at_or_after(bytes, pos, hashes) {
                Some(end) => {
                    pos = end;
                    None
                }
                None => return Some(hashes),
            },
            None => match raw_string_open_at_or_after(bytes, pos) {
                Some((hashes, content_start)) => {
                    pos = content_start;
                    Some(hashes)
                }
                None => return None,
            },
        };
    }
}

/// The `#` count and content-start byte index of the first raw-string
/// opener (`r"`, `r#"`, `br##"`, `cr"`, ...) at or after `from`, if any --
/// the introducing `r` (and an immediately preceding `b`/`c`, if present)
/// must not itself continue a longer identifier, so e.g. `variable_r"..."`
/// or `abr"..."` is not mistaken for one. Stops at the first genuine `//`
/// line-comment start: real Rust `//` consumes the rest of that line as
/// plain comment text, so nothing after it -- including text that merely
/// looks like raw-string syntax inside the comment itself -- can open one.
fn raw_string_open_at_or_after(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    for i in from..bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            return None;
        }
        if bytes[i] != b'r' {
            continue;
        }
        let prefix_start = match i.checked_sub(1) {
            Some(p) if matches!(bytes[p], b'b' | b'c') => p,
            _ => i,
        };
        if prefix_start > 0 && is_ident_byte(bytes[prefix_start - 1]) {
            continue;
        }
        let mut j = i + 1;
        let mut hashes = 0;
        while bytes.get(j) == Some(&b'#') {
            hashes += 1;
            j += 1;
        }
        if bytes.get(j) == Some(&b'"') {
            return Some((hashes, j + 1));
        }
    }
    None
}

/// The byte index right after the closing delimiter (`"` followed by
/// exactly `hashes` `#`s) at or after `from`, if any.
fn raw_string_close_at_or_after(bytes: &[u8], from: usize, hashes: usize) -> Option<usize> {
    for i in from..bytes.len() {
        if bytes[i] != b'"' {
            continue;
        }
        let mut j = i + 1;
        let mut count = 0;
        while count < hashes && bytes.get(j) == Some(&b'#') {
            count += 1;
            j += 1;
        }
        if count == hashes {
            return Some(j);
        }
    }
    None
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// A line's content after its required `.rs` `//` line-comment prefix, or
/// `None` if the line is not itself a `//` comment. A marker must be a
/// real comment's own declaration -- not merely contained somewhere in a
/// longer comment, sentence, or string literal that happens to include the
/// same text (e.g. `SPDX-License-Identifier: MPL-2.0 OR MIT` naming a
/// different expression, or the notice text quoted inside a non-comment
/// line in this module's own tests).
fn comment_payload(line: &str) -> Option<&str> {
    line.trim().strip_prefix("//").map(str::trim_start)
}

fn is_bound(rel: &str) -> bool {
    Path::new(rel)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| BOUND_EXTENSIONS.contains(&ext))
}

pub(crate) fn license_problem(root: &Path) -> Option<String> {
    match fs::read(root.join("LICENSE")) {
        Err(_) => Some("LICENSE is missing".to_owned()),
        Ok(bytes) if content_digest(&bytes) != CANONICAL_LICENSE_SHA256 => {
            Some("LICENSE is not the canonical MPL-2.0 text".to_owned())
        }
        Ok(_) => None,
    }
}

/// Files changed since the merge base with `base`, staged or unstaged edits,
/// and untracked files: everything this change creates or edits.
fn touched_files(root: &Path, base: &str) -> Result<BTreeSet<String>, String> {
    let merge_base = git_lines(root, &["merge-base", base, "HEAD"])
        .map_err(|err| format!("cannot resolve --base {base}: {err}"))?;
    let merge_base = merge_base
        .first()
        .ok_or_else(|| format!("cannot resolve --base {base}: no merge base"))?;
    let mut touched = BTreeSet::new();
    touched.extend(git_lines(
        root,
        &["diff", "--name-only", "--diff-filter=d", merge_base],
    )?);
    touched.extend(git_lines(
        root,
        &["ls-files", "--others", "--exclude-standard"],
    )?);
    Ok(touched)
}

/// Publishable packages whose `.crate` would carry no license text: no
/// resolved `license-file` and no `LICENSE*` beside the manifest. The
/// repository root package is covered by the root `LICENSE`. Both `publish`
/// and `license-file` are resolved through `[workspace.package]` when the
/// package sets `key.workspace = true`, matching Cargo's own inheritance --
/// an inherited `license-file` path is relative to the workspace root, an
/// own-package one to the package's directory, exactly as Cargo resolves it.
///
/// Known accepted gap: this does not run real `cargo package --list`, so a
/// package whose `include`/`exclude` manifest fields would exclude an
/// otherwise-canonical `LICENSE` from its actual `.crate` archive is not
/// detected; this layer checks that the canonical text exists at the
/// resolved path, not that Cargo's packaging rules would actually ship it.
/// This is the same non-blocking-by-default layer as the untouched-notice
/// backlog (reported under `--strict` only), not the primary per-file gate.
fn packages_without_license(root: &Path) -> Result<Vec<String>, String> {
    let manifests = git_lines(
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "Cargo.toml",
            "*/Cargo.toml",
        ],
    )?;
    let mut missing = Vec::new();
    for rel in manifests {
        let path = root.join(&rel);
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let manifest: toml::Table = toml::from_str(&text).map_err(|err| format!("{rel}: {err}"))?;
        let Some(package) = manifest.get("package").and_then(|value| value.as_table()) else {
            continue;
        };
        let dir = path.parent().unwrap_or(root);
        // This repository's own root package (root/Cargo.toml itself, not a
        // publishable member) is covered by the root LICENSE, not this
        // layer -- checked before resolving a workspace table so a
        // workspace-only root manifest with no [package] never reaches
        // here in the first place (the `let Some(package) = ...` guard
        // above already skips it), and a root that happens to declare
        // [package] is still exempt regardless.
        if dir == root {
            continue;
        }
        // Each package's [workspace.package] comes from its OWN nearest
        // enclosing workspace, not unconditionally the checkout root: this
        // repository itself nests a separate real workspace at
        // crates/Cargo.toml, distinct from the top-level one. An inherited
        // path resolves relative to THAT workspace's own root, not the
        // outer checkout root.
        let (workspace_root, workspace_package) = nearest_workspace(dir, package, root)?;
        if !is_publishable(package, &workspace_package) {
            continue;
        }
        let declared_path = match resolve_workspace_license_file(package, &workspace_package) {
            Some(LicenseFileRef::Local(value)) => Some(dir.join(value)),
            Some(LicenseFileRef::Inherited(value)) => Some(workspace_root.join(value)),
            None => None,
        };
        let carries_license = match declared_path {
            Some(declared_path) => is_canonical_license_file(&declared_path),
            None => has_license_file(dir),
        };
        if carries_license {
            continue;
        }
        missing.push(rel);
    }
    Ok(missing)
}

/// The package's actual workspace root directory and its
/// `[workspace.package]` table.
///
/// A package's own `[package] workspace = "path"` (relative to the
/// package's own directory) explicitly overrides Cargo's normal discovery
/// and is honored first, matching Cargo's own precedence. Otherwise this
/// walks up from `dir` (inclusive) to `root` (inclusive) for the nearest
/// ancestor Cargo.toml that declares a `[workspace]` table: Cargo resolves
/// `key.workspace = true` against the workspace a package actually belongs
/// to, which is not always the checkout root -- a repository can nest an
/// independent workspace below its own top level, and an inherited path
/// resolves relative to that workspace's own root.
///
/// Returns `(root, empty table)` if there is no explicit override and no
/// enclosing directory up to and including `root` declares a `[workspace]`
/// table at all.
fn nearest_workspace(
    dir: &Path,
    package: &toml::Table,
    root: &Path,
) -> Result<(PathBuf, toml::Table), String> {
    if let Some(explicit) = package.get("workspace").and_then(|value| value.as_str()) {
        let workspace_root = dir.join(explicit);
        let workspace_package = workspace_package_table_at(&workspace_root)?;
        return Ok((workspace_root, workspace_package));
    }
    for candidate_dir in dir.ancestors() {
        if let Some(workspace_package) = workspace_package_table_if_declared(candidate_dir)? {
            return Ok((candidate_dir.to_path_buf(), workspace_package));
        }
        if candidate_dir == root {
            break;
        }
    }
    Ok((root.to_path_buf(), toml::Table::new()))
}

/// `dir`'s own `Cargo.toml`'s `[workspace.package]` table, or empty if that
/// manifest is unreadable or declares no `[workspace]` table -- used for an
/// explicit `[package] workspace = "..."` override, where the target is
/// trusted to be a real workspace even if this session cannot read it.
fn workspace_package_table_at(dir: &Path) -> Result<toml::Table, String> {
    Ok(workspace_package_table_if_declared(dir)?.unwrap_or_default())
}

/// `dir`'s own `Cargo.toml`'s `[workspace.package]` table, or `None` if
/// that manifest is unreadable, malformed, or declares no `[workspace]`
/// table at all (distinct from declaring an empty one).
fn workspace_package_table_if_declared(dir: &Path) -> Result<Option<toml::Table>, String> {
    let candidate = dir.join("Cargo.toml");
    let Ok(text) = fs::read_to_string(&candidate) else {
        return Ok(None);
    };
    let manifest: toml::Table =
        toml::from_str(&text).map_err(|err| format!("{}: {err}", candidate.display()))?;
    let Some(workspace) = manifest.get("workspace").and_then(|value| value.as_table()) else {
        return Ok(None);
    };
    Ok(Some(
        workspace
            .get("package")
            .and_then(|value| value.as_table())
            .cloned()
            .unwrap_or_default(),
    ))
}

/// Cargo requires a `version` to publish (inherited or own -- either shape
/// of the key existing is enough to check here, since only its presence,
/// not its resolved value, matters). It also treats an explicit `publish =
/// false` or an empty `publish = []` registry list, resolved through
/// `publish.workspace = true` when present, as unpublishable; anything
/// else, including an absent `publish` key, is conservatively treated as
/// publishable so this layer still checks it.
fn is_publishable(package: &toml::Table, workspace: &toml::Table) -> bool {
    if !package.contains_key("version") {
        return false;
    }
    let publish = match package.get("publish") {
        Some(value) if is_workspace_inherited(value) => workspace.get("publish"),
        other => other,
    };
    match publish {
        Some(toml::Value::Boolean(false)) => false,
        Some(toml::Value::Array(registries)) => !registries.is_empty(),
        _ => true,
    }
}

fn is_workspace_inherited(value: &toml::Value) -> bool {
    value
        .as_table()
        .and_then(|table| table.get("workspace"))
        .and_then(toml::Value::as_bool)
        == Some(true)
}

enum LicenseFileRef<'a> {
    /// Relative to the declaring package's own directory.
    Local(&'a str),
    /// `license-file.workspace = true`, relative to the workspace root.
    Inherited(&'a str),
}

fn resolve_workspace_license_file<'a>(
    package: &'a toml::Table,
    workspace: &'a toml::Table,
) -> Option<LicenseFileRef<'a>> {
    match package.get("license-file") {
        Some(toml::Value::String(value)) => Some(LicenseFileRef::Local(value)),
        Some(value) if is_workspace_inherited(value) => workspace
            .get("license-file")
            .and_then(toml::Value::as_str)
            .map(LicenseFileRef::Inherited),
        _ => None,
    }
}

/// True only for a regular file whose bytes are the canonical MPL-2.0 text --
/// matching `license_problem`'s own bar for the root `LICENSE`. A declared
/// `license-file` key that names a missing, empty, or non-MPL file, or a
/// `LICENSE*`-prefixed directory entry with no real license text (or that is
/// itself a directory), does not satisfy this layer.
fn is_canonical_license_file(path: &Path) -> bool {
    fs::read(path).is_ok_and(|bytes| content_digest(&bytes) == CANONICAL_LICENSE_SHA256)
}

fn has_license_file(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("LICENSE"))
                && is_canonical_license_file(&entry.path())
        })
    })
}

fn git_lines(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|err| format!("git {}: {err}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        root: std::env::current_dir().map_err(|err| err.to_string())?,
        base: "origin/main".to_owned(),
        strict: false,
    };
    let mut args = args.iter().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repo-root" => {
                options.root = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--repo-root requires a value".to_owned())?;
            }
            "--base" => {
                options.base = args
                    .next()
                    .cloned()
                    .ok_or_else(|| "--base requires a value".to_owned())?;
            }
            "--strict" => options.strict = true,
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("unknown check-license option: {other}")),
        }
    }
    Ok(options)
}

fn usage() -> String {
    "usage: xtask check-license [--repo-root PATH] [--base REV] [--strict]".to_owned()
}

#[cfg(test)]
mod tests;
