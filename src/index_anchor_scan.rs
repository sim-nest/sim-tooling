//! Discovery of source anchors for SIM Index fragments.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use serde_json::Value;
use sim_index_core::{AnchorId, DiscoveredAnchor, SubjectId};

use crate::{
    index_fragment::{
        is_test_source, package_rust_files, rel_path, repo_name, slug_ident, slug_path, subject_id,
    },
    repo_contract::PackageContract,
};

/// Discovers deterministic anchors for source and generated contract facts.
pub(crate) fn discovered(
    repo: &Path,
    packages: &[PackageContract],
    cards: &[Value],
) -> Vec<DiscoveredAnchor> {
    let repo_subject = subject_id("repo", &repo_name(repo));
    let doc_set_subject = subject_id("doc-set", &format!("{}/generated", repo_name(repo)));
    let mut anchors = BTreeMap::new();

    insert_anchor(
        &mut anchors,
        "anchor/repo",
        &repo_name(repo),
        &repo_subject,
        "repo",
    );
    insert_generated_doc_anchors(&mut anchors, &doc_set_subject);
    insert_card_anchors(&mut anchors, cards, &repo_subject);

    for package in packages {
        let crate_subject = subject_id("crate", &package.name);
        insert_anchor(
            &mut anchors,
            "anchor/crate",
            &package.name,
            &crate_subject,
            "crate",
        );
        for lib in runtime_lib_names(repo, package) {
            let lib_subject = subject_id("runtime-lib", &format!("{}/{lib}", package.name));
            insert_anchor(
                &mut anchors,
                "anchor/runtime-lib",
                &format!("{}/{lib}", package.name),
                &lib_subject,
                "runtime-lib",
            );
        }
        insert_cli_anchors(&mut anchors, repo, package, &crate_subject);
        insert_rustdoc_anchors(&mut anchors, repo, package, &crate_subject);
        insert_export_anchors(&mut anchors, repo, package, &crate_subject);
    }

    anchors.into_values().collect()
}

fn insert_generated_doc_anchors(
    anchors: &mut BTreeMap<String, DiscoveredAnchor>,
    subject: &SubjectId,
) {
    for name in [
        "provenance",
        "repo-contract",
        "rustdoc-index",
        "card-index",
        "feature-map",
        "sim-index-fragment",
    ] {
        insert_anchor(
            anchors,
            "anchor/doc/generated",
            name,
            subject,
            "doc-section",
        );
    }
}

fn insert_card_anchors(
    anchors: &mut BTreeMap<String, DiscoveredAnchor>,
    cards: &[Value],
    repo_subject: &SubjectId,
) {
    for card in cards {
        let Some(id) = card["id"].as_str() else {
            continue;
        };
        let subject = card["owner"]
            .as_str()
            .filter(|owner| *owner != "workspace")
            .map(|owner| subject_id("crate", owner))
            .unwrap_or_else(|| repo_subject.clone());
        let kind = card["kind"].as_str().unwrap_or("card");
        insert_anchor(anchors, "anchor/card", id, &subject, kind);
    }
}

fn insert_cli_anchors(
    anchors: &mut BTreeMap<String, DiscoveredAnchor>,
    repo: &Path,
    package: &PackageContract,
    subject: &SubjectId,
) {
    if package.target_kinds.iter().any(|kind| kind == "bin") {
        insert_anchor(anchors, "anchor/cli", &package.name, subject, "cli-verb");
    }
    for verb in cli_verbs(repo, package) {
        insert_anchor(anchors, "anchor/cli", &verb, subject, "cli-verb");
    }
}

fn insert_export_anchors(
    anchors: &mut BTreeMap<String, DiscoveredAnchor>,
    repo: &Path,
    package: &PackageContract,
    subject: &SubjectId,
) {
    let mut exports = BTreeSet::new();
    for path in package_rust_files(repo, package) {
        let rel = rel_path(repo, &path);
        if is_test_source(&rel) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let text = non_test_source_text(&text);
        for symbol in quoted_values(&text) {
            if is_export_symbol(&symbol) {
                exports.insert(symbol);
            }
        }
    }
    for export in exports {
        insert_anchor(
            anchors,
            "anchor/export",
            &format!("{}/{}", package.name, export),
            subject,
            "export",
        );
    }
}

fn insert_rustdoc_anchors(
    anchors: &mut BTreeMap<String, DiscoveredAnchor>,
    repo: &Path,
    package: &PackageContract,
    subject: &SubjectId,
) {
    let mut items = BTreeSet::new();
    for path in package_rust_files(repo, package) {
        let rel = rel_path(repo, &path);
        if is_test_source(&rel) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(file) = syn::parse_file(&text) else {
            continue;
        };
        collect_public_items(&file.items, "", &mut items);
        if rel.ends_with("/lib.rs") || rel == "src/lib.rs" {
            items.insert("crate-root".to_owned());
        }
    }
    for item in items {
        insert_anchor(
            anchors,
            "anchor/rustdoc",
            &format!("{}/{}", package.name, item),
            subject,
            "rustdoc-item",
        );
    }
}

fn collect_public_items(items: &[syn::Item], prefix: &str, out: &mut BTreeSet<String>) {
    for item in items {
        match item {
            syn::Item::Const(item) if is_public(&item.vis) => {
                out.insert(join_path(prefix, &item.ident.to_string()));
            }
            syn::Item::Enum(item) if is_public(&item.vis) => {
                out.insert(join_path(prefix, &item.ident.to_string()));
            }
            syn::Item::Fn(item) if is_public(&item.vis) => {
                out.insert(join_path(prefix, &item.sig.ident.to_string()));
            }
            syn::Item::Mod(item) if is_public(&item.vis) => {
                let name = join_path(prefix, &item.ident.to_string());
                out.insert(name.clone());
                if let Some((_, nested)) = &item.content {
                    collect_public_items(nested, &name, out);
                }
            }
            syn::Item::Static(item) if is_public(&item.vis) => {
                out.insert(join_path(prefix, &item.ident.to_string()));
            }
            syn::Item::Struct(item) if is_public(&item.vis) => {
                out.insert(join_path(prefix, &item.ident.to_string()));
            }
            syn::Item::Trait(item) if is_public(&item.vis) => {
                out.insert(join_path(prefix, &item.ident.to_string()));
            }
            syn::Item::Type(item) if is_public(&item.vis) => {
                out.insert(join_path(prefix, &item.ident.to_string()));
            }
            _ => {}
        }
    }
}

fn is_public(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

fn join_path(prefix: &str, name: &str) -> String {
    let name = slug_ident(name);
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    }
}

fn cli_verbs(repo: &Path, package: &PackageContract) -> BTreeSet<String> {
    let mut verbs = BTreeSet::new();
    for path in package_rust_files(repo, package) {
        let rel = rel_path(repo, &path);
        if is_test_source(&rel) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let text = non_test_source_text(&text);
        for symbol in quoted_values(&text) {
            if let Some(verb) = symbol.strip_prefix("cli/main/") {
                if !is_simple_symbol_tail(verb) {
                    continue;
                }
                let verb = slug_path(verb);
                if !verb.is_empty() {
                    verbs.insert(verb);
                }
            }
        }
    }
    verbs
}

fn runtime_lib_names(repo: &Path, package: &PackageContract) -> Vec<String> {
    let mut libs = BTreeSet::new();
    for path in package_rust_files(repo, package) {
        let rel = rel_path(repo, &path);
        if is_test_source(&rel) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let text = non_test_source_text(&text);
        if let Ok(file) = syn::parse_file(&text) {
            collect_lib_impls(&file.items, &mut libs);
        }
    }
    libs.into_iter().collect()
}

fn collect_lib_impls(items: &[syn::Item], libs: &mut BTreeSet<String>) {
    for item in items {
        match item {
            syn::Item::Impl(item_impl) => {
                let Some((_, trait_path, _)) = &item_impl.trait_ else {
                    continue;
                };
                if trait_path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "Lib")
                    && let Some(name) = impl_type_ident(&item_impl.self_ty)
                {
                    libs.insert(slug_ident(&name));
                }
            }
            syn::Item::Mod(module) => {
                if let Some((_, nested)) = &module.content {
                    collect_lib_impls(nested, libs);
                }
            }
            _ => {}
        }
    }
}

fn impl_type_ident(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    if path.qself.is_some() {
        return None;
    }
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn is_export_symbol(symbol: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "agent/",
        "agent:",
        "bridge/",
        "bridge:",
        "browse/",
        "card/",
        "chat/",
        "chat:",
        "citizen/",
        "cli/main/",
        "codec/",
        "codec:",
        "doc/",
        "grammar/",
        "mcp/",
        "mcp:",
        "model/",
        "model:",
        "packet/",
        "packet:",
        "registry/",
        "runtime/",
        "server/",
        "shape/",
        "shape:",
        "site/",
        "site:",
        "surface/",
        "surface:",
        "view/",
        "view:",
        "wire/",
        "wire:",
    ];

    PREFIXES
        .iter()
        .find_map(|prefix| symbol.strip_prefix(prefix))
        .is_some_and(is_simple_symbol_tail)
}

pub(crate) fn is_simple_symbol_tail(tail: &str) -> bool {
    !tail.is_empty()
        && tail.split('/').all(|part| !part.is_empty())
        && tail.bytes().all(|byte| {
            matches!(
                byte,
                b'a'..=b'z'
                    | b'A'..=b'Z'
                    | b'0'..=b'9'
                    | b'-'
                    | b'_'
                    | b'.'
                    | b'/'
            )
        })
}

pub(crate) fn quoted_values(text: &str) -> Vec<String> {
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

pub(crate) fn non_test_source_text(text: &str) -> String {
    let mut out = String::new();
    let mut pending_cfg_test = false;
    let mut skipping_cfg_test = false;
    let mut brace_depth = 0_i32;

    for line in text.lines() {
        if skipping_cfg_test {
            brace_depth += brace_delta(line);
            if brace_depth <= 0 {
                skipping_cfg_test = false;
                brace_depth = 0;
            }
            continue;
        }

        if pending_cfg_test {
            let delta = brace_delta(line);
            if delta > 0 {
                skipping_cfg_test = true;
                brace_depth = delta;
            }
            pending_cfg_test = false;
            continue;
        }

        if line.trim_start().starts_with("#[cfg(test") {
            pending_cfg_test = true;
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }

    out
}

fn brace_delta(line: &str) -> i32 {
    let opens = line.bytes().filter(|byte| *byte == b'{').count() as i32;
    let closes = line.bytes().filter(|byte| *byte == b'}').count() as i32;
    opens - closes
}

fn insert_anchor(
    anchors: &mut BTreeMap<String, DiscoveredAnchor>,
    prefix: &str,
    tail: &str,
    subject: &SubjectId,
    kind: &str,
) {
    let tail = slug_path(tail);
    if tail.is_empty() {
        return;
    }
    let id = AnchorId::new(format!("{prefix}/{tail}"));
    anchors
        .entry(id.to_string())
        .or_insert_with(|| DiscoveredAnchor {
            id,
            subject: subject.clone(),
            kind: kind.to_owned(),
        });
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::json;

    use super::*;

    #[test]
    fn discovers_card_cli_export_and_rustdoc_anchors() {
        let root = temp_root("sim-tooling-anchor-scan");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub struct PublicThing;\n\
             struct PrivateThing;\n\
             pub fn run() {}\n\
             const HIDDEN: u8 = 0;\n\
             const CLI: &str = \"cli/main/demo\";\n\
             const EXPORT: &str = \"surface/demo\";\n\
             const DIAGNOSTIC: &str = \"agent/attach expects a slot name\";\n\
             const PATH: &str = \"docs/generated/repo-contract.json\";\n",
        )
        .unwrap();
        let package = package("sim-demo", "");
        let cards = vec![json!({
            "id": "browse/catalog",
            "kind": "browse-root",
            "owner": "workspace"
        })];

        let anchors = discovered(&root, &[package], &cards);
        let ids = anchors
            .iter()
            .map(|anchor| anchor.id.as_str())
            .collect::<BTreeSet<_>>();

        assert!(ids.contains("anchor/card/browse/catalog"));
        assert!(ids.contains("anchor/cli/demo"));
        assert!(ids.contains("anchor/export/sim-demo/surface/demo"));
        assert!(!ids.contains("anchor/export/sim-demo/agent/attach-expects-a-slot-name"));
        assert!(!ids.contains("anchor/export/sim-demo/docs/generated/repo-contract.json"));
        assert!(ids.contains("anchor/rustdoc/sim-demo/public-thing"));
        assert!(ids.contains("anchor/rustdoc/sim-demo/run"));
        assert!(!ids.contains("anchor/rustdoc/sim-demo/private-thing"));

        fs::remove_dir_all(root).unwrap();
    }

    fn package(name: &str, root: &str) -> PackageContract {
        PackageContract {
            name: name.to_owned(),
            crate_name: name.replace('-', "_"),
            manifest: if root.is_empty() {
                "Cargo.toml".to_owned()
            } else {
                format!("{root}/Cargo.toml")
            },
            root: root.to_owned(),
            group: "workspace".to_owned(),
            publish: "false".to_owned(),
            description: format!("{name} package"),
            target_kinds: vec!["lib".to_owned(), "bin".to_owned()],
            targets: Vec::new(),
            dependencies: Vec::new(),
            features: Vec::new(),
            rustdoc_summary: format!("{name} docs"),
        }
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
