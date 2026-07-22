use std::{
    fs, io,
    path::{Path, PathBuf},
};

use syn::{Attribute, Type};

use super::display_io;

pub(super) fn package_name(manifest: &Path) -> Result<String, String> {
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

pub(super) fn domain_from_package(name: &str) -> String {
    name.strip_prefix("sim-lib-")
        .or_else(|| name.strip_prefix("sim-codec-"))
        .or_else(|| name.strip_prefix("sim-"))
        .unwrap_or(name)
        .replace('_', "-")
}

pub(super) fn rust_files(root: &Path) -> io::Result<Vec<PathBuf>> {
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

pub(super) fn find_repo_root(start: &Path) -> Result<PathBuf, String> {
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

pub(super) fn is_test_path(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "tests")
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "tests.rs" || name.ends_with("_tests.rs"))
}

pub(super) fn has_attr(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}

pub(super) fn derives_citizen(attrs: &[Attribute]) -> bool {
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

pub(super) fn impl_self_type_name(ty: &Type) -> Option<String> {
    let Type::Path(path) = ty else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}
