//! The citizenize task: scan a crate or path for citizen candidates and rewrite them toward the citizen conventions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

use syn::{Attribute, Fields, Item, Type, Visibility, spanned::Spanned};

/// Summary of a `citizenize` run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CitizenizeReport {
    /// Number of types considered as citizen candidates.
    pub candidates: usize,
    /// Number of source files rewritten.
    pub files_changed: usize,
}

#[derive(Debug)]
struct CrateInfo {
    root: PathBuf,
    manifest: PathBuf,
    name: String,
    domain: String,
    repo: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DependencyMode {
    /// Write versioned dependencies that are legal in a public repository.
    Published,
    /// Write local path dependencies for deliberate local checkout migrations.
    LocalPaths,
}

#[derive(Clone, Debug)]
struct Candidate {
    name: String,
    insert_line: usize,
    has_derive: bool,
    field_list_lines: Vec<usize>,
}

/// Citizenizes the crate named or located by `target`, resolved within the
/// current repository, and returns the run summary.
pub fn citizenize_arg(target: &str) -> Result<CitizenizeReport, String> {
    citizenize_arg_with_mode(target, DependencyMode::Published)
}

pub(crate) fn citizenize_arg_with_mode(
    target: &str,
    mode: DependencyMode,
) -> Result<CitizenizeReport, String> {
    let repo = find_repo_root(&std::env::current_dir().map_err(display_io)?)?;
    let krate = resolve_crate(&repo, target)?;
    citizenize_crate(&krate, mode)
}

/// Citizenizes the crate rooted at `path` (the directory holding its
/// `Cargo.toml`) and returns the run summary.
pub fn citizenize_path(path: &Path) -> Result<CitizenizeReport, String> {
    citizenize_path_with_mode(path, DependencyMode::Published)
}

pub(crate) fn citizenize_path_with_mode(
    path: &Path,
    mode: DependencyMode,
) -> Result<CitizenizeReport, String> {
    let repo = find_repo_root(&std::env::current_dir().map_err(display_io)?)?;
    let root = path.canonicalize().map_err(display_io)?;
    let manifest = root.join("Cargo.toml");
    let name = package_name(&manifest)?;
    let domain = domain_from_package(&name);
    citizenize_crate(
        &CrateInfo {
            root,
            manifest,
            name,
            domain,
            repo,
        },
        mode,
    )
}

fn citizenize_crate(krate: &CrateInfo, mode: DependencyMode) -> Result<CitizenizeReport, String> {
    let files = rust_files(&krate.root.join("src")).map_err(display_io)?;
    let skip_by_impl = collect_skip_impls(&files)?;
    let mut report = CitizenizeReport::default();

    for file in files {
        if is_test_path(&file) {
            continue;
        }
        let text = fs::read_to_string(&file).map_err(display_io)?;
        let parsed = syn::parse_file(&text).map_err(display_syn)?;
        let candidates = candidates_in_file(&parsed, &skip_by_impl);
        if candidates.is_empty() {
            continue;
        }
        report.candidates += candidates.len();
        let edited = edit_file(&text, &krate.domain, &candidates);
        if edited != text {
            fs::write(&file, edited).map_err(display_io)?;
            report.files_changed += 1;
        }
    }

    if report.candidates > 0 && ensure_citizen_dependencies(krate, mode)? {
        report.files_changed += 1;
    }

    Ok(report)
}

fn collect_skip_impls(files: &[PathBuf]) -> Result<BTreeSet<String>, String> {
    let mut skip = BTreeSet::new();
    for file in files {
        if is_test_path(file) {
            continue;
        }
        let text = fs::read_to_string(file).map_err(display_io)?;
        let parsed = syn::parse_file(&text).map_err(display_syn)?;
        for item in parsed.items {
            let Item::Impl(item) = item else {
                continue;
            };
            let Some((_, path, _)) = item.trait_ else {
                continue;
            };
            let Some(trait_name) = path
                .segments
                .last()
                .map(|segment| segment.ident.to_string())
            else {
                continue;
            };
            if !matches!(
                trait_name.as_str(),
                "Callable" | "Op" | "ReadConstructor" | "Citizen"
            ) {
                continue;
            }
            if let Some(type_name) = impl_self_type_name(&item.self_ty) {
                skip.insert(type_name);
            }
        }
    }
    Ok(skip)
}

fn candidates_in_file(parsed: &syn::File, skip_by_impl: &BTreeSet<String>) -> Vec<Candidate> {
    parsed
        .items
        .iter()
        .filter_map(|item| {
            let Item::Struct(item) = item else {
                return None;
            };
            if !matches!(item.vis, Visibility::Public(_)) {
                return None;
            }
            if has_attr(&item.attrs, "cfg") || has_attr(&item.attrs, "non_citizen") {
                return None;
            }
            if derives_citizen(&item.attrs) || has_attr(&item.attrs, "citizen") {
                return None;
            }
            let name = item.ident.to_string();
            if skip_by_impl.contains(&name) {
                return None;
            }
            let Fields::Named(fields) = &item.fields else {
                return None;
            };
            let field_list_lines = fields
                .named
                .iter()
                .filter(|field| !has_attr(&field.attrs, "citizen") && is_vec_type(&field.ty))
                .filter_map(|field| field.ident.as_ref().map(|ident| ident.span().start().line))
                .collect::<Vec<_>>();
            let insert_line = item
                .attrs
                .first()
                .map(|attr| attr.span().start().line)
                .unwrap_or_else(|| item.struct_token.span.start().line);
            Some(Candidate {
                name,
                insert_line,
                has_derive: has_attr(&item.attrs, "derive"),
                field_list_lines,
            })
        })
        .collect()
}

fn edit_file(text: &str, domain: &str, candidates: &[Candidate]) -> String {
    let mut before = BTreeMap::<usize, Vec<String>>::new();
    let needs_import = !text.contains("sim_citizen_derive::Citizen");
    if needs_import {
        before
            .entry(import_line(text))
            .or_default()
            .push("use sim_citizen_derive::Citizen;".to_owned());
    }
    for candidate in candidates {
        let derive = if candidate.has_derive {
            "#[derive(Citizen)]".to_owned()
        } else {
            "#[derive(Clone, Debug, Default, PartialEq, Citizen)]".to_owned()
        };
        before.entry(candidate.insert_line).or_default().extend([
            derive,
            format!(
                "#[citizen(symbol = \"{}/{}\", version = 1)]",
                domain, candidate.name
            ),
            format!(
                "// TODO: validate citizen example fixture for {}",
                candidate.name
            ),
        ]);
        for line in &candidate.field_list_lines {
            before
                .entry(*line)
                .or_default()
                .push("#[citizen(list)]".to_owned());
        }
    }

    let mut out = String::new();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        if let Some(insertions) = before.get(&line_no) {
            let indent = leading_indent(line);
            for insertion in insertions {
                if insertion.starts_with("use ") {
                    out.push_str(insertion);
                } else {
                    out.push_str(indent);
                    out.push_str(insertion);
                }
                out.push('\n');
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn resolve_crate(repo: &Path, target: &str) -> Result<CrateInfo, String> {
    let target_path = repo.join(target);
    if target_path.join("Cargo.toml").is_file() {
        return crate_info(repo, target_path);
    }
    let crates_path = repo.join("crates").join(target);
    if crates_path.join("Cargo.toml").is_file() {
        return crate_info(repo, crates_path);
    }
    for entry in fs::read_dir(repo.join("crates")).map_err(display_io)? {
        let path = entry.map_err(display_io)?.path();
        if !path.join("Cargo.toml").is_file() {
            continue;
        }
        let info = crate_info(repo, path)?;
        if info.name == target {
            return Ok(info);
        }
    }
    Err(format!("could not resolve crate {target:?}"))
}

fn crate_info(repo: &Path, root: PathBuf) -> Result<CrateInfo, String> {
    let manifest = root.join("Cargo.toml");
    let name = package_name(&manifest)?;
    let domain = domain_from_package(&name);
    Ok(CrateInfo {
        root,
        manifest,
        name,
        domain,
        repo: repo.to_path_buf(),
    })
}

fn ensure_citizen_dependencies(krate: &CrateInfo, mode: DependencyMode) -> Result<bool, String> {
    let text = fs::read_to_string(&krate.manifest).map_err(display_io)?;
    let mut additions = Vec::new();
    if !has_dependency(&text, "sim-citizen") {
        additions.push(dependency_spec(krate, "sim-citizen", mode));
    }
    if !has_dependency(&text, "sim-citizen-derive") {
        additions.push(dependency_spec(krate, "sim-citizen-derive", mode));
    }
    if !has_dependency(&text, "sim-kernel") {
        additions.push(dependency_spec(krate, "sim-kernel", mode));
    }
    if additions.is_empty() {
        return Ok(false);
    }
    fs::write(&krate.manifest, insert_dependencies(&text, &additions)).map_err(display_io)?;
    Ok(true)
}

fn dependency_spec(krate: &CrateInfo, dep: &str, mode: DependencyMode) -> String {
    match mode {
        DependencyMode::Published => format!("{dep} = \"{}\"", published_version(dep)),
        DependencyMode::LocalPaths => {
            format!("{dep} = {{ path = \"{}\" }}", dependency_path(krate, dep))
        }
    }
}

fn published_version(dep: &str) -> &'static str {
    match dep {
        "sim-citizen" => "0.1.1",
        "sim-citizen-derive" => "0.1.0",
        "sim-kernel" => "0.1.3",
        _ => "0.1",
    }
}

fn has_dependency(manifest: &str, name: &str) -> bool {
    let mut in_dependencies = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed == "[dependencies]" {
            in_dependencies = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_dependencies = false;
        }
        if in_dependencies
            && (trimmed.starts_with(&format!("{name} ="))
                || trimmed.starts_with(&format!("{name}=")))
        {
            return true;
        }
    }
    false
}

fn insert_dependencies(manifest: &str, additions: &[String]) -> String {
    let mut out = String::new();
    let mut in_dependencies = false;
    let mut inserted = false;

    for line in manifest.lines() {
        let trimmed = line.trim();
        if in_dependencies && trimmed.starts_with('[') {
            push_additions(&mut out, additions);
            inserted = true;
            in_dependencies = false;
        }
        out.push_str(line);
        out.push('\n');
        if trimmed == "[dependencies]" {
            in_dependencies = true;
        }
    }

    if !inserted {
        if !in_dependencies {
            if !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str("[dependencies]\n");
        }
        push_additions(&mut out, additions);
    }

    out
}

fn push_additions(out: &mut String, additions: &[String]) {
    for addition in additions {
        out.push_str(addition);
        out.push('\n');
    }
}

fn dependency_path(krate: &CrateInfo, dep: &str) -> String {
    let dep_root = dependency_root(&krate.repo, dep);
    if let (Some(target_parent), Some(dep_parent)) = (krate.root.parent(), dep_root.parent())
        && target_parent == dep_parent
    {
        return format!("../{dep}");
    }
    dep_root.display().to_string()
}

fn dependency_root(repo: &Path, dep: &str) -> PathBuf {
    let local_crate_root = repo.join("crates").join(dep);
    if local_crate_root.join("Cargo.toml").is_file() {
        return local_crate_root;
    }

    let Some(parent) = repo.parent() else {
        return local_crate_root;
    };
    match dep {
        "sim-kernel" => parent.join("sim-kernel"),
        "sim-citizen" | "sim-citizen-derive" => parent.join("sim-citizen").join("crates").join(dep),
        _ => local_crate_root,
    }
}

fn package_name(manifest: &Path) -> Result<String, String> {
    let text = fs::read_to_string(manifest).map_err(display_io)?;
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        match trimmed {
            "[package]" => in_package = true,
            section if section.starts_with('[') => in_package = false,
            _ => {}
        }
        if in_package
            && trimmed.starts_with("name")
            && let Some((_, value)) = trimmed.split_once('=')
        {
            return Ok(value.trim().trim_matches('"').to_owned());
        }
    }
    Err(format!("missing package name in {}", manifest.display()))
}

fn domain_from_package(name: &str) -> String {
    name.strip_prefix("sim-lib-")
        .or_else(|| name.strip_prefix("sim-codec-"))
        .or_else(|| name.strip_prefix("sim-"))
        .unwrap_or(name)
        .replace('_', "-")
}

fn rust_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_rust_files(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn find_repo_root(start: &Path) -> Result<PathBuf, String> {
    for path in start.ancestors() {
        let manifest = path.join("Cargo.toml");
        if manifest.is_file()
            && fs::read_to_string(&manifest)
                .map_err(display_io)?
                .contains("[workspace]")
        {
            return Ok(path.to_path_buf());
        }
    }
    Err("could not find repository root".to_owned())
}

fn is_test_path(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "tests")
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "tests.rs" || name.ends_with("_tests.rs"))
}

fn has_attr(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}

fn derives_citizen(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("derive")
            && attr
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                )
                .is_ok_and(|paths| {
                    paths
                        .iter()
                        .any(|path| path.segments.last().is_some_and(|s| s.ident == "Citizen"))
                })
    })
}

fn is_vec_type(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "Vec")
}

fn impl_self_type_name(ty: &Type) -> Option<String> {
    let Type::Path(path) = ty else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn import_line(text: &str) -> usize {
    let mut line_no = 1;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//!") || trimmed.starts_with("#![") {
            line_no += 1;
            continue;
        }
        break;
    }
    line_no
}

fn leading_indent(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

fn display_io(err: io::Error) -> String {
    err.to_string()
}

fn display_syn(err: syn::Error) -> String {
    err.to_string()
}

#[cfg(test)]
mod tests;
