use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use syn::{Attribute, Expr, Item, Lit, Meta};

use super::{index_manifest::RepoEntry, io::display_io};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SourceUnit {
    pub(super) id: String,
    pub(super) repo: String,
    pub(super) crate_name: Option<String>,
    pub(super) kind: String,
    pub(super) path: String,
    pub(super) line: usize,
    pub(super) text: String,
    pub(super) graph_id: Option<String>,
    pub(super) related_ids: Vec<String>,
    pub(super) panels: Vec<String>,
}

pub(super) fn collect_units(repo: &RepoEntry) -> Result<Vec<SourceUnit>, String> {
    let mut units = vec![repo_manifest_unit(repo)];
    if !repo.checkout_path.is_dir() {
        return Ok(units);
    }

    push_file_unit(
        repo,
        &mut units,
        "readme",
        &repo.checkout_path.join("README.md"),
    )?;
    collect_simdoc_units(repo, &mut units)?;
    collect_recipe_units(repo, &mut units)?;
    collect_rust_units(repo, &mut units)?;
    units.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(units)
}

fn repo_manifest_unit(repo: &RepoEntry) -> SourceUnit {
    let text = format!(
        "# Repo {name}\nkind: {kind}\nlocal_path: {local_path}\ncrates: {crates}\nsource_paths: {sources}\nvalidation_command: {validation}\ndocs_command: {docs}\npin: {pin}\nstatus: {status}\n",
        name = repo.name,
        kind = repo.kind,
        local_path = repo.local_path,
        crates = repo.crate_names.join(", "),
        sources = repo.source_paths.join(", "),
        validation = repo.validation_command,
        docs = repo.docs_command,
        pin = repo.pin,
        status = repo.status.as_str(),
    );
    SourceUnit {
        id: format!("{}/repo-manifest", repo.name),
        repo: repo.name.clone(),
        crate_name: None,
        kind: "repo-manifest".to_owned(),
        path: "repos.toml".to_owned(),
        line: 1,
        text,
        graph_id: None,
        related_ids: Vec::new(),
        panels: Vec::new(),
    }
}

fn collect_simdoc_units(repo: &RepoEntry, units: &mut Vec<SourceUnit>) -> Result<(), String> {
    for dir in [
        "docs/generated",
        "docs/agents",
        "docs/humans",
        "docs/diagrams/generated",
    ] {
        collect_text_tree(repo, units, "simdoc", &repo.checkout_path.join(dir))?;
    }
    Ok(())
}

fn collect_recipe_units(repo: &RepoEntry, units: &mut Vec<SourceUnit>) -> Result<(), String> {
    collect_text_tree(repo, units, "recipe", &repo.checkout_path.join("recipes"))
}

fn collect_text_tree(
    repo: &RepoEntry,
    units: &mut Vec<SourceUnit>,
    kind: &str,
    root: &Path,
) -> Result<(), String> {
    if !root.is_dir() {
        return Ok(());
    }
    for path in text_files(root)? {
        push_file_unit(repo, units, kind, &path)?;
    }
    Ok(())
}

fn collect_rust_units(repo: &RepoEntry, units: &mut Vec<SourceUnit>) -> Result<(), String> {
    for crate_root in crate_roots(repo) {
        let crate_name = crate_name(&crate_root).or_else(|| repo.crate_names.first().cloned());
        let src = crate_root.join("src");
        if !src.is_dir() {
            continue;
        }
        for path in rust_files(&src)? {
            push_rust_file_units(repo, units, crate_name.as_deref(), &path)?;
        }
    }
    Ok(())
}

fn crate_roots(repo: &RepoEntry) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = BTreeSet::new();
    push_crate_root(&mut roots, &mut seen, repo.checkout_path.clone());
    for source in &repo.source_paths {
        push_crate_root(&mut roots, &mut seen, repo.checkout_path.join(source));
    }
    roots
}

fn push_crate_root(roots: &mut Vec<PathBuf>, seen: &mut BTreeSet<PathBuf>, root: PathBuf) {
    if root.is_dir() && root.join("Cargo.toml").is_file() && seen.insert(root.clone()) {
        roots.push(root);
    }
}

fn push_file_unit(
    repo: &RepoEntry,
    units: &mut Vec<SourceUnit>,
    kind: &str,
    path: &Path,
) -> Result<(), String> {
    if !path.is_file() || is_meta_workspace_path(path) || !is_text_path(path) {
        return Ok(());
    }
    let text = fs::read_to_string(path).map_err(display_io)?;
    let display = display_path(repo, path);
    units.push(SourceUnit {
        id: format!("{}/{}/{}", repo.name, kind, stable_path_id(&display)),
        repo: repo.name.clone(),
        crate_name: None,
        kind: kind.to_owned(),
        path: display,
        line: 1,
        text,
        graph_id: None,
        related_ids: Vec::new(),
        panels: Vec::new(),
    });
    Ok(())
}

fn push_rust_file_units(
    repo: &RepoEntry,
    units: &mut Vec<SourceUnit>,
    crate_name: Option<&str>,
    path: &Path,
) -> Result<(), String> {
    let text = fs::read_to_string(path).map_err(display_io)?;
    let parsed =
        syn::parse_file(&text).map_err(|err| format!("parse {}: {err}", path.display()))?;
    let display = display_path(repo, path);
    let module_docs = doc_attrs(&parsed.attrs);
    if !module_docs.is_empty() {
        units.push(SourceUnit {
            id: format!(
                "{}/rust/{}/module-docs",
                repo.name,
                stable_path_id(&display)
            ),
            repo: repo.name.clone(),
            crate_name: crate_name.map(str::to_owned),
            kind: "rust-doc".to_owned(),
            path: display.clone(),
            line: 1,
            text: module_docs.join("\n"),
            graph_id: None,
            related_ids: Vec::new(),
            panels: Vec::new(),
        });
    }

    for item in parsed.items {
        let Some((item_kind, name, signature, attrs, line)) = item_summary(&item) else {
            continue;
        };
        let docs = doc_attrs(attrs);
        let mut unit_text = format!("signature: {signature}\n");
        if !docs.is_empty() {
            unit_text.push_str("docs:\n");
            unit_text.push_str(&docs.join("\n"));
            unit_text.push('\n');
        }
        units.push(SourceUnit {
            id: format!(
                "{}/rust/{}/{}",
                repo.name,
                stable_path_id(&display),
                stable_path_id(&name)
            ),
            repo: repo.name.clone(),
            crate_name: crate_name.map(str::to_owned),
            kind: format!("rust-{item_kind}"),
            path: display.clone(),
            line,
            text: unit_text,
            graph_id: None,
            related_ids: Vec::new(),
            panels: Vec::new(),
        });
    }
    Ok(())
}

type ItemSummary<'a> = (&'static str, String, String, &'a [Attribute], usize);

fn item_summary(item: &Item) -> Option<ItemSummary<'_>> {
    match item {
        Item::Const(item) => Some((
            "const",
            item.ident.to_string(),
            format!("const {}", item.ident),
            &item.attrs,
            item.ident.span().start().line,
        )),
        Item::Enum(item) => Some((
            "enum",
            item.ident.to_string(),
            format!("enum {}", item.ident),
            &item.attrs,
            item.ident.span().start().line,
        )),
        Item::Fn(item) => Some((
            "fn",
            item.sig.ident.to_string(),
            format!("fn {}", item.sig.ident),
            &item.attrs,
            item.sig.ident.span().start().line,
        )),
        Item::Mod(item) => Some((
            "mod",
            item.ident.to_string(),
            format!("mod {}", item.ident),
            &item.attrs,
            item.ident.span().start().line,
        )),
        Item::Struct(item) => Some((
            "struct",
            item.ident.to_string(),
            format!("struct {}", item.ident),
            &item.attrs,
            item.ident.span().start().line,
        )),
        Item::Trait(item) => Some((
            "trait",
            item.ident.to_string(),
            format!("trait {}", item.ident),
            &item.attrs,
            item.ident.span().start().line,
        )),
        Item::Type(item) => Some((
            "type",
            item.ident.to_string(),
            format!("type {}", item.ident),
            &item.attrs,
            item.ident.span().start().line,
        )),
        _ => None,
    }
}

fn doc_attrs(attrs: &[Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter_map(|attr| {
            if !attr.path().is_ident("doc") {
                return None;
            }
            let Meta::NameValue(value) = &attr.meta else {
                return None;
            };
            let Expr::Lit(lit) = &value.value else {
                return None;
            };
            let Lit::Str(text) = &lit.lit else {
                return None;
            };
            Some(text.value().trim().to_owned())
        })
        .filter(|text| !text.is_empty())
        .collect()
}

fn text_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    walk_files(root, is_text_path)
}

fn rust_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    walk_files(root, |path| path.extension().is_some_and(|ext| ext == "rs"))
}

fn walk_files(root: &Path, accept: fn(&Path) -> bool) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        if is_meta_workspace_path(&path) || is_ignored_dir(&path) {
            continue;
        }
        if path.is_dir() {
            for entry in fs::read_dir(&path).map_err(display_io)? {
                stack.push(entry.map_err(display_io)?.path());
            }
        } else if accept(&path) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn is_text_path(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        matches!(
            ext.to_string_lossy().as_ref(),
            "md" | "toml" | "json" | "jsonl" | "txt" | "sim" | "scm" | "lisp" | "rs"
        )
    })
}

fn is_ignored_dir(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        matches!(
            name.to_string_lossy().as_ref(),
            ".git" | "target" | ".sim" | ".meta-workspace"
        )
    })
}

fn is_meta_workspace_path(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == ".meta-workspace")
}

fn display_path(repo: &RepoEntry, path: &Path) -> String {
    path.strip_prefix(&repo.checkout_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn stable_path_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn crate_name(crate_root: &Path) -> Option<String> {
    let text = fs::read_to_string(crate_root.join("Cargo.toml")).ok()?;
    let value = text.parse::<toml::Value>().ok()?;
    value
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
}
