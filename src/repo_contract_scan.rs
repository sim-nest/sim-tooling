//! Package and browse-card scanning for the repo-contract task.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};

use crate::repo_contract_cut::CONTRACT_CUT_PATH;

pub(crate) fn card_index(repo: &Path, package_groups: &BTreeMap<String, String>) -> Vec<Value> {
    let mut cards = BTreeMap::<String, Value>::new();
    insert_card(
        &mut cards,
        "browse/catalog",
        "browse-root",
        "root browse catalog",
        None,
    );
    insert_card(
        &mut cards,
        "registry/catalog",
        "browse-registry",
        "registry catalog browse card",
        None,
    );

    scan_reflection_cards(repo, &mut cards);
    scan_surface_cards(repo, &mut cards);
    scan_static_card_keys(repo, &mut cards, package_groups);
    for recipe in recipe_books(repo, package_groups) {
        if let Some(id) = recipe["card_id"].as_str() {
            let summary = recipe["summary"].as_str().unwrap_or("cookbook recipe");
            insert_card(
                &mut cards,
                id,
                "cookbook-recipe",
                summary,
                recipe["package"].as_str(),
            );
        }
    }
    for citizen in citizen_classes(repo) {
        if let Some(symbol) = citizen["symbol"].as_str() {
            let id = format!("citizen/{symbol}");
            insert_card(
                &mut cards,
                &id,
                "citizen-class",
                "citizen class constructor surface",
                citizen["crate"].as_str(),
            );
        }
    }

    cards.into_values().collect()
}

pub(crate) fn citizen_classes(repo: &Path) -> Vec<Value> {
    let path = repo.join("docs/generated/citizens.md");
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        if !line.starts_with("| `") || line.contains("---") {
            continue;
        }
        let cells = markdown_cells(line);
        if cells.len() < 4 || cells[0] == "Symbol" {
            continue;
        }
        out.push(json!({
            "symbol": trim_code(&cells[0]),
            "version": cells[1].trim().parse::<u64>().unwrap_or(0),
            "arity": cells[2].trim().parse::<u64>().unwrap_or(0),
            "crate": trim_code(&cells[3]),
        }));
    }
    out
}

pub(crate) fn non_citizen_exemptions(repo: &Path) -> Vec<Value> {
    let root_package = root_package_name(repo);
    let mut files = rust_files(repo);
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let rel = rel_path(repo, &path);
        let mut lines = text.lines().enumerate().peekable();
        while let Some((index, line)) = lines.next() {
            if !line.contains("non_citizen(") {
                continue;
            }
            let mut attr = line.to_owned();
            while !attr.contains(")]") {
                let Some((_, next)) = lines.next() else {
                    break;
                };
                attr.push('\n');
                attr.push_str(next);
            }
            out.push(json!({
                "file": rel,
                "line": index + 1,
                "crate": crate_for_path(&path, &root_package),
                "kind": attr_value(&attr, "kind").unwrap_or_else(|| "unknown".to_owned()),
                "descriptor": attr_value(&attr, "descriptor").unwrap_or_else(|| "unknown".to_owned()),
                "reason": attr_value(&attr, "reason").unwrap_or_else(|| "unspecified".to_owned()),
            }));
        }
    }
    out.sort_by(|left, right| {
        left["file"]
            .as_str()
            .cmp(&right["file"].as_str())
            .then(left["line"].as_u64().cmp(&right["line"].as_u64()))
    });
    out
}

pub(crate) fn recipe_books(repo: &Path, package_groups: &BTreeMap<String, String>) -> Vec<Value> {
    let root_package = root_package_name(repo);
    let mut sources = Vec::new();
    let root_book = repo.join("recipes/book.toml");
    if root_book.is_file() {
        sources.push((root_book, repo.to_path_buf(), root_package.clone()));
    }

    let mut paths = Vec::new();
    collect_named_files(&repo.join("crates"), "book.toml", &mut paths);
    paths.sort();
    for path in paths {
        if !path.to_string_lossy().contains("/recipes/") {
            continue;
        }
        let Some(root) = path.parent().and_then(Path::parent) else {
            continue;
        };
        let root = root.to_path_buf();
        let package = crate_for_path(&path, &root_package);
        sources.push((path, root, package));
    }

    sources.sort_by(|left, right| left.0.cmp(&right.0));
    sources
        .into_iter()
        .filter_map(|(path, root, package)| {
            recipe_book_entry(repo, &path, &root, &package, package_groups)
        })
        .collect()
}

fn recipe_book_entry(
    repo: &Path,
    path: &Path,
    root: &Path,
    package: &str,
    package_groups: &BTreeMap<String, String>,
) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    let book = toml_string_value(&text, "book").unwrap_or_else(|| package.to_owned());
    let title = toml_string_value(&text, "title").unwrap_or_else(|| book.clone());
    let summary = toml_string_value(&text, "summary").unwrap_or_default();
    let recipes = recipe_entries(repo, root, &book, package);
    Some(json!({
        "package": package,
        "group": package_groups.get(package).cloned().unwrap_or_default(),
        "book": book,
        "title": title,
        "summary": summary,
        "book_toml": rel_path(repo, path),
        "recipe_count": recipes.len(),
        "recipes": recipes,
        "card_id": format!("cookbook/{book}"),
    }))
}

pub(crate) fn input_files(repo: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for path in [
        "Cargo.toml",
        CONTRACT_CUT_PATH,
        "features.toml",
        "docs/generated/citizens.md",
    ] {
        let path = repo.join(path);
        if path.is_file() {
            files.push(path);
        }
    }
    collect_named_files(repo, "Cargo.toml", &mut files);
    collect_named_files(repo, "book.toml", &mut files);
    collect_named_files(repo, "recipe.toml", &mut files);
    files.extend(rust_files(repo));
    files.sort();
    files.dedup();
    files
}

fn scan_reflection_cards(repo: &Path, cards: &mut BTreeMap<String, Value>) {
    let path = repo.join("src/runtime/browse/reflection.rs");
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let mut namespace: Option<String> = None;
    let mut name: Option<String> = None;
    let mut summary: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("namespace:") {
            namespace = quoted_value(trimmed);
        } else if trimmed.starts_with("name:") {
            name = quoted_value(trimmed);
        } else if trimmed.starts_with("summary:") {
            summary = quoted_value(trimmed);
        } else if trimmed == "}," {
            if let (Some(namespace), Some(name)) = (namespace.take(), name.take()) {
                let id = format!("{namespace}/{name}");
                insert_card(
                    cards,
                    &id,
                    "browse-reflection",
                    summary.as_deref().unwrap_or("browse reflection subject"),
                    Some("sim"),
                );
            }
            summary = None;
        }
    }
}

fn scan_surface_cards(repo: &Path, cards: &mut BTreeMap<String, Value>) {
    let path = repo.join("src/runtime/browse/surface_cards.rs");
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let values = quoted_values(line);
        if values.len() >= 3 {
            let id = format!("{}/{}", values[0], values[1]);
            insert_card(cards, &id, "surface-card", &values[2], Some("sim"));
        }
    }
}

fn scan_static_card_keys(
    repo: &Path,
    cards: &mut BTreeMap<String, Value>,
    package_groups: &BTreeMap<String, String>,
) {
    let root_package = root_package_name(repo);
    let mut paths = Vec::new();
    collect_named_files(&repo.join("crates"), "cards.rs", &mut paths);
    paths.sort();
    for path in paths {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let package = crate_for_path(&path, &root_package);
        let group = package_groups.get(&package).map(String::as_str);
        let mut pending_key: Option<String> = None;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("key:") {
                pending_key = quoted_value(trimmed);
            } else if trimmed.starts_with("summary:")
                && let (Some(key), Some(summary)) = (pending_key.take(), quoted_value(trimmed))
            {
                insert_card(
                    cards,
                    &key,
                    "static-card",
                    &summary,
                    group.or(Some(&package)),
                );
            }
        }
    }
}

fn recipe_entries(repo: &Path, root: &Path, book: &str, package: &str) -> Vec<Value> {
    let mut paths = Vec::new();
    collect_named_files(&root.join("recipes"), "recipe.toml", &mut paths);
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let id = toml_string_value(&text, "id").unwrap_or_else(|| {
            path.parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .unwrap_or("recipe")
                .to_owned()
        });
        let chapter = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        out.push(json!({
            "id": id,
            "card_id": format!("{book}/{chapter}/{id}"),
            "package": package,
            "title": toml_string_value(&text, "title").unwrap_or_default(),
            "codec": toml_string_value(&text, "codec").unwrap_or_default(),
            "recipe_toml": rel_path(repo, &path),
        }));
    }
    out
}

fn insert_card(
    cards: &mut BTreeMap<String, Value>,
    id: &str,
    kind: &str,
    summary: &str,
    owner: Option<&str>,
) {
    cards.entry(id.to_owned()).or_insert_with(|| {
        json!({
            "id": id,
            "kind": kind,
            "summary": summary,
            "owner": owner.unwrap_or("workspace"),
        })
    });
}

fn rust_files(repo: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_ext_files(&repo.join("src"), "rs", &mut files);
    collect_ext_files(&repo.join("crates"), "rs", &mut files);
    files
}

fn collect_named_files(dir: &Path, name: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if should_descend(&path) {
                collect_named_files(&path, name, out);
            }
        } else if path.file_name().and_then(|file| file.to_str()) == Some(name) {
            out.push(path);
        }
    }
}

fn collect_ext_files(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if should_descend(&path) {
                collect_ext_files(&path, extension, out);
            }
        } else if path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
            out.push(path);
        }
    }
}

fn attr_value(attr: &str, key: &str) -> Option<String> {
    let needle = format!("{key} = ");
    let start = attr.find(&needle)? + needle.len();
    quoted_value(&attr[start..])
}

fn toml_string_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix(key)?.trim_start();
        quoted_value(rest.strip_prefix('=')?.trim_start())
    })
}

fn quoted_value(text: &str) -> Option<String> {
    quoted_values(text).into_iter().next()
}

fn quoted_values(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('"') {
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('"') else {
            break;
        };
        out.push(after_start[..end].to_owned());
        rest = &after_start[end + 1..];
    }
    out
}

fn markdown_cells(line: &str) -> Vec<String> {
    line.trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect()
}

fn trim_code(text: &str) -> String {
    text.trim().trim_matches('`').to_owned()
}

fn crate_for_path(path: &Path, root_package: &str) -> String {
    let mut parts = path.components().filter_map(|part| match part {
        std::path::Component::Normal(value) => value.to_str(),
        _ => None,
    });
    while let Some(part) = parts.next() {
        if part == "crates" {
            return parts.next().unwrap_or("sim").to_owned();
        }
    }
    root_package.to_owned()
}

fn root_package_name(repo: &Path) -> String {
    fs::read_to_string(repo.join("Cargo.toml"))
        .ok()
        .and_then(|text| toml_string_value(&text, "name"))
        .unwrap_or_else(|| {
            repo.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("workspace")
                .to_owned()
        })
}

fn should_descend(path: &Path) -> bool {
    !matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | "target" | ".meta-workspace")
    )
}

fn rel_path(repo: &Path, path: &Path) -> String {
    path.strip_prefix(repo)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        env, fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn input_files_cover_rust_sources_without_local_lockfile() {
        let root = temp_root("sim-tooling-input-files");
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::create_dir_all(root.join("crates/sim-fixture/src")).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n").unwrap();
        fs::write(root.join("Cargo.lock"), "# local ignored lockfile\n").unwrap();
        fs::write(root.join("features.toml"), "schema = \"sim.features\"\n").unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        fs::write(root.join("src/nested/tool.rs"), "").unwrap();
        fs::write(root.join("crates/sim-fixture/src/lib.rs"), "").unwrap();

        let paths = input_files(&root)
            .into_iter()
            .map(|path| {
                path.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/")
            })
            .collect::<Vec<_>>();

        assert!(paths.contains(&"Cargo.toml".to_owned()));
        assert!(paths.contains(&"features.toml".to_owned()));
        assert!(paths.contains(&"src/lib.rs".to_owned()));
        assert!(paths.contains(&"src/nested/tool.rs".to_owned()));
        assert!(paths.contains(&"crates/sim-fixture/src/lib.rs".to_owned()));
        assert!(!paths.contains(&"Cargo.lock".to_owned()));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recipe_books_include_root_layout_book() {
        let root = temp_root("sim-tooling-root-recipes");
        let recipe_dir = root
            .join("recipes")
            .join("01-basics")
            .join("exact-bool-shape");
        fs::create_dir_all(&recipe_dir).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"sim-root\"\n").unwrap();
        fs::write(
            root.join("recipes/book.toml"),
            "book = \"sim-root\"\ntitle = \"Root recipes\"\nsummary = \"Root cookbook.\"\n",
        )
        .unwrap();
        fs::write(
            recipe_dir.join("recipe.toml"),
            "id = \"exact-bool-shape\"\ntitle = \"Exact boolean shape\"\ncodec = \"rust\"\n",
        )
        .unwrap();

        let groups = BTreeMap::from([("sim-root".to_owned(), "workspace".to_owned())]);
        let books = recipe_books(&root, &groups);

        assert_eq!(books.len(), 1);
        assert_eq!(books[0]["package"], "sim-root");
        assert_eq!(books[0]["group"], "workspace");
        assert_eq!(books[0]["book_toml"], "recipes/book.toml");
        assert_eq!(books[0]["recipe_count"], 1);
        assert_eq!(
            books[0]["recipes"][0]["recipe_toml"],
            "recipes/01-basics/exact-bool-shape/recipe.toml"
        );
        assert_eq!(
            books[0]["recipes"][0]["card_id"],
            "sim-root/01-basics/exact-bool-shape"
        );

        fs::remove_dir_all(root).unwrap();
    }

    fn temp_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("{name}-{}-{stamp}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
}
