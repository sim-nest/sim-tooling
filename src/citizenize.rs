//! The citizenize task: scan a crate or path for citizen candidates and rewrite them toward the citizen conventions.

use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

use syn::{Fields, Item, Visibility, spanned::Spanned};

mod command;
mod deps;
mod edit;
mod parser;

pub(crate) use command::run;

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
    let repo = parser::find_repo_root(&std::env::current_dir().map_err(display_io)?)?;
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
    let repo = parser::find_repo_root(&std::env::current_dir().map_err(display_io)?)?;
    let root = path.canonicalize().map_err(display_io)?;
    let manifest = root.join("Cargo.toml");
    let name = parser::package_name(&manifest)?;
    let domain = parser::domain_from_package(&name);
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
    let files = parser::rust_files(&krate.root.join("src")).map_err(display_io)?;
    let skip_by_impl = collect_skip_impls(&files)?;
    let mut report = CitizenizeReport::default();

    for file in files {
        if parser::is_test_path(&file) {
            continue;
        }
        let text = fs::read_to_string(&file).map_err(display_io)?;
        let parsed = syn::parse_file(&text).map_err(display_syn)?;
        let candidates = candidates_in_file(&parsed, &skip_by_impl);
        if candidates.is_empty() {
            continue;
        }
        report.candidates += candidates.len();
        let edited = edit::edit_file(&text, &krate.domain, &candidates);
        if edited != text {
            fs::write(&file, edited).map_err(display_io)?;
            report.files_changed += 1;
        }
    }

    if report.candidates > 0 && deps::ensure_citizen_dependencies(krate, mode)? {
        report.files_changed += 1;
    }

    Ok(report)
}

fn collect_skip_impls(files: &[PathBuf]) -> Result<BTreeSet<String>, String> {
    let mut skip = BTreeSet::new();
    for file in files {
        if parser::is_test_path(file) {
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
            if let Some(type_name) = parser::impl_self_type_name(&item.self_ty) {
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
            if parser::has_attr(&item.attrs, "cfg") || parser::has_attr(&item.attrs, "non_citizen")
            {
                return None;
            }
            if parser::derives_citizen(&item.attrs) || parser::has_attr(&item.attrs, "citizen") {
                return None;
            }
            let name = item.ident.to_string();
            if skip_by_impl.contains(&name) {
                return None;
            }
            let Fields::Named(_) = &item.fields else {
                return None;
            };
            let insert_line = item
                .attrs
                .first()
                .map(|attr| attr.span().start().line)
                .unwrap_or_else(|| item.struct_token.span.start().line);
            Some(Candidate {
                name,
                insert_line,
                has_derive: parser::has_attr(&item.attrs, "derive"),
            })
        })
        .collect()
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
    let name = parser::package_name(&manifest)?;
    let domain = parser::domain_from_package(&name);
    Ok(CrateInfo {
        root,
        manifest,
        name,
        domain,
        repo: repo.to_path_buf(),
    })
}

pub(super) fn display_io(err: io::Error) -> String {
    err.to_string()
}

fn display_syn(err: syn::Error) -> String {
    err.to_string()
}

#[cfg(test)]
mod tests;
