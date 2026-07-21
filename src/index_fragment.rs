//! SIM Index fragment generation from repo-contract scan facts.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;
use sim_codec_index::{IndexCodec, IndexForm};
use sim_index_core::{IndexDoc, IndexEdge, SubjectId, SubjectRecord, check_index_doc};
use sim_kernel::EncodePosition;

use crate::repo_contract::{GENERATOR, PackageContract};

/// Builds the generated `sim-index-fragment.sx` contract artifact.
pub(crate) fn artifact(
    repo: &Path,
    packages: &[PackageContract],
    cards: &[Value],
) -> Result<String, String> {
    let doc = index_doc(repo, packages, cards)?;
    IndexCodec
        .encode(&doc, EncodePosition::Data, IndexForm::Sx)
        .map_err(|err| format!("encode sim-index-fragment.sx: {err}"))
}

pub(crate) fn index_doc(
    repo: &Path,
    packages: &[PackageContract],
    cards: &[Value],
) -> Result<IndexDoc, String> {
    let (subjects, edges) = package_subjects(repo, packages);
    let anchors = crate::index_anchor_scan::discovered(repo, packages, cards);
    let discovered = crate::index_surface_scan::discovered(repo, packages, &anchors);
    let mut doc = IndexDoc::public(GENERATOR);
    doc.subjects = merge_subjects(subjects, discovered.subjects);
    doc.anchors = anchors.into_iter().chain(discovered.anchors).collect();
    doc.surfaces = discovered.surfaces;
    doc.drafts = discovered.drafts;
    doc.edges = edges;
    check_index_doc(&doc).map_err(|err| format!("invalid generated index fragment: {err}"))?;
    Ok(doc)
}

fn merge_subjects(left: Vec<SubjectRecord>, right: Vec<SubjectRecord>) -> Vec<SubjectRecord> {
    let mut subjects = BTreeMap::new();
    for subject in left.into_iter().chain(right) {
        subjects.entry(subject.id.to_string()).or_insert(subject);
    }
    subjects.into_values().collect()
}

pub(crate) fn package_subjects(
    repo: &Path,
    packages: &[PackageContract],
) -> (Vec<SubjectRecord>, Vec<IndexEdge>) {
    let repo_name = repo_name(repo);
    let repo_id = subject_id("repo", &repo_name);
    let doc_set_id = subject_id("doc-set", &format!("{repo_name}/generated"));
    let mut subjects = BTreeMap::new();
    let mut edges = BTreeSet::new();

    insert_subject(&mut subjects, repo_id.clone(), "repo", &repo_name);
    insert_subject(
        &mut subjects,
        doc_set_id.clone(),
        "doc-set",
        &format!("{repo_name} generated docs"),
    );
    insert_edge(&mut edges, &repo_id, "contains", &doc_set_id);

    for package in packages {
        let crate_id = subject_id("crate", &package.name);
        insert_subject(&mut subjects, crate_id.clone(), "crate", &package.name);
        insert_edge(&mut edges, &repo_id, "contains", &crate_id);

        for runtime_lib in runtime_libs(repo, package) {
            let lib_id = subject_id("runtime-lib", &format!("{}/{runtime_lib}", package.name));
            insert_subject(&mut subjects, lib_id.clone(), "runtime-lib", &runtime_lib);
            insert_edge(&mut edges, &crate_id, "contains", &lib_id);
        }

        if let Some(language) = codec_language(package) {
            let language_id = subject_id("language", &language);
            let grammar_id = subject_id("grammar", &language);
            insert_subject(&mut subjects, language_id.clone(), "language", &language);
            insert_subject(
                &mut subjects,
                grammar_id.clone(),
                "grammar",
                &format!("{language} grammar"),
            );
            insert_edge(&mut edges, &language_id, "contains", &grammar_id);
        }
    }

    let edges = edges
        .into_iter()
        .map(|(from, rel, to)| IndexEdge::new(from, rel, to))
        .collect();
    (subjects.into_values().collect(), edges)
}

pub(crate) fn insert_subject(
    subjects: &mut BTreeMap<String, SubjectRecord>,
    id: SubjectId,
    kind: &str,
    title: &str,
) {
    subjects.entry(id.to_string()).or_insert(SubjectRecord {
        id,
        kind: kind.to_owned(),
        title: title.to_owned(),
    });
}

fn insert_edge(
    edges: &mut BTreeSet<(String, String, String)>,
    from: &SubjectId,
    rel: &str,
    to: &SubjectId,
) {
    edges.insert((from.to_string(), rel.to_owned(), to.to_string()));
}

pub(crate) fn subject_id(kind: &str, tail: &str) -> SubjectId {
    SubjectId::new(format!("{kind}/{}", slug_path(tail)))
}

fn runtime_libs(repo: &Path, package: &PackageContract) -> Vec<String> {
    let mut libs = BTreeSet::new();
    for path in package_rust_files(repo, package) {
        let rel = rel_path(repo, &path);
        if is_test_source(&rel) {
            continue;
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        libs.extend(runtime_lib_impls(&text));
    }
    libs.into_iter().collect()
}

fn runtime_lib_impls(text: &str) -> Vec<String> {
    let Ok(file) = syn::parse_file(text) else {
        return Vec::new();
    };
    let mut out = BTreeSet::new();
    collect_runtime_lib_impls(&file.items, &mut out);
    out.into_iter().collect()
}

fn collect_runtime_lib_impls(items: &[syn::Item], out: &mut BTreeSet<String>) {
    for item in items {
        match item {
            syn::Item::Impl(item_impl) if !has_cfg_test(&item_impl.attrs) => {
                let Some((_, trait_path, _)) = &item_impl.trait_ else {
                    continue;
                };
                if is_lib_trait(trait_path)
                    && let Some(name) = impl_type_ident(&item_impl.self_ty)
                {
                    out.insert(slug_ident(&name));
                }
            }
            syn::Item::Mod(module) if !has_cfg_test(&module.attrs) => {
                if let Some((_, nested_items)) = &module.content {
                    collect_runtime_lib_impls(nested_items, out);
                }
            }
            _ => {}
        }
    }
}

fn has_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| match &attr.meta {
        syn::Meta::List(list) => {
            list.path.is_ident("cfg") && list.tokens.to_string().contains("test")
        }
        _ => false,
    })
}

fn is_lib_trait(path: &syn::Path) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| segment.ident == "Lib")
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

pub(crate) fn package_rust_files(repo: &Path, package: &PackageContract) -> Vec<PathBuf> {
    let src = if package.root.is_empty() {
        repo.join("src")
    } else {
        repo.join(&package.root).join("src")
    };
    let mut files = Vec::new();
    collect_ext_files(&src, "rs", &mut files);
    files.sort();
    files
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

fn should_descend(path: &Path) -> bool {
    !matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | "target" | ".meta-workspace")
    )
}

pub(crate) fn is_test_source(rel: &str) -> bool {
    rel.contains("/tests/")
        || rel.ends_with("/tests.rs")
        || rel.ends_with("_tests.rs")
        || rel.contains("/test_support/")
}

pub(crate) fn codec_language(package: &PackageContract) -> Option<String> {
    package
        .name
        .strip_prefix("sim-codec-")
        .filter(|tail| !tail.is_empty())
        .map(slug_path)
}

pub(crate) fn slug_path(input: &str) -> String {
    input
        .split('/')
        .map(slug_ident)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn slug_ident(input: &str) -> String {
    let mut out = String::new();
    let mut previous_dash = false;
    let mut previous_lower_or_digit = false;
    for byte in input.bytes() {
        let ch = match byte {
            b'a'..=b'z' | b'0'..=b'9' => {
                previous_dash = false;
                previous_lower_or_digit = true;
                byte as char
            }
            b'_' | b'.' => {
                previous_dash = false;
                previous_lower_or_digit = false;
                byte as char
            }
            b'A'..=b'Z' => {
                if previous_lower_or_digit && !previous_dash {
                    out.push('-');
                }
                previous_dash = false;
                previous_lower_or_digit = false;
                byte.to_ascii_lowercase() as char
            }
            b'-' => {
                if previous_dash {
                    continue;
                }
                previous_dash = true;
                previous_lower_or_digit = false;
                '-'
            }
            _ => {
                if previous_dash {
                    continue;
                }
                previous_dash = true;
                previous_lower_or_digit = false;
                '-'
            }
        };
        out.push(ch);
    }
    out.trim_matches('-').to_owned()
}

pub(crate) fn repo_name(repo: &Path) -> String {
    repo.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace")
        .to_owned()
}

pub(crate) fn rel_path(repo: &Path, path: &Path) -> String {
    path.strip_prefix(repo)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::json;
    use sim_codec_index::{IndexCodec, IndexForm};

    use super::*;

    #[test]
    fn package_subjects_emit_containment_edges() {
        let parent = temp_root("sim-tooling-index-fragment-parent");
        let root = parent.join("sim-tooling-index-fragment");
        fs::create_dir_all(root.join("crates/sim-codec-demo/src")).unwrap();
        fs::write(
            root.join("crates/sim-codec-demo/src/lib.rs"),
            "use sim_kernel::{Lib, LibManifest};\n\
             pub struct DemoLib;\n\
             impl Lib for DemoLib {\n\
                 fn manifest(&self) -> LibManifest { todo!() }\n\
                 fn load(&self, _: &mut sim_kernel::LoadCx, _: &mut sim_kernel::Linker<'_>) -> sim_kernel::Result<()> { todo!() }\n\
             }\n",
        )
        .unwrap();
        let package = package("sim-codec-demo", "crates/sim-codec-demo");

        let (subjects, edges) = package_subjects(&root, &[package]);
        let ids = subjects
            .iter()
            .map(|subject| subject.id.as_str())
            .collect::<BTreeSet<_>>();
        let edge_ids = edges
            .iter()
            .map(|edge| (edge.from.as_str(), edge.rel.as_str(), edge.to.as_str()))
            .collect::<BTreeSet<_>>();

        assert!(ids.contains("repo/sim-tooling-index-fragment"));
        assert!(ids.contains("crate/sim-codec-demo"));
        assert!(ids.contains("runtime-lib/sim-codec-demo/demo-lib"));
        assert!(ids.contains("language/demo"));
        assert!(ids.contains("grammar/demo"));
        assert!(ids.contains("doc-set/sim-tooling-index-fragment/generated"));
        assert!(edge_ids.contains(&(
            "repo/sim-tooling-index-fragment",
            "contains",
            "crate/sim-codec-demo"
        )));
        assert!(edge_ids.contains(&(
            "crate/sim-codec-demo",
            "contains",
            "runtime-lib/sim-codec-demo/demo-lib"
        )));
        assert!(edge_ids.contains(&("language/demo", "contains", "grammar/demo")));

        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn fragment_artifact_round_trips_through_codec_index() {
        let root = temp_root("sim-tooling-index-fragment-roundtrip");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn run() {}\n").unwrap();
        let sx = artifact(&root, &[package("xtask", "")], &[]).expect("fragment artifact");
        let doc = IndexCodec
            .decode(IndexForm::Sx, &sx)
            .expect("codec/index decodes fragment");

        assert_eq!(doc.schema, "sim.index");
        assert!(sx.contains("doc-set"));
        assert!(
            doc.edges
                .iter()
                .any(|edge| edge.rel == "contains" && edge.to == "crate/xtask")
        );

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
            target_kinds: vec!["lib".to_owned()],
            targets: vec![json!({
                "name": name,
                "kind": ["lib"],
                "crate_types": ["lib"],
                "src": if root.is_empty() { "src/lib.rs".to_owned() } else { format!("{root}/src/lib.rs") },
            })],
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
