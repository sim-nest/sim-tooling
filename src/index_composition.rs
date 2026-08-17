//! Source-backed checking for authored reuse and composition relations.

use std::{collections::BTreeMap, fs, path::Path};

use quote::ToTokens;
use sim_index_core::{IndexDoc, SubjectId};
use syn::visit::Visit;

use crate::{
    index_author::AuthoredOverlay,
    index_fragment::{is_test_source, package_rust_files, rel_path},
    repo_contract::PackageContract,
};

pub(crate) fn check_authored_composition(
    repo: &Path,
    packages: &[PackageContract],
    doc: &IndexDoc,
    overlay: &AuthoredOverlay,
) -> Result<(), String> {
    let owners = overlay
        .features()
        .map(|(feature, owner)| (feature.to_owned(), owner.clone()))
        .chain(
            doc.features
                .iter()
                .map(|feature| (feature.id.to_string(), feature.subject.clone())),
        )
        .collect::<BTreeMap<_, _>>();
    let package_by_subject = packages
        .iter()
        .map(|package| (format!("crate/{}", package.name), package))
        .collect::<BTreeMap<_, _>>();

    for (feature, relation, target) in overlay
        .relations()
        .filter(|(_, relation, _)| matches!(*relation, "reuses" | "composes"))
    {
        let claiming_owner = owners.get(feature).ok_or_else(|| {
            format!("authored {relation} claim {feature} has no resolved owner package")
        })?;
        let target_owner = resolve_target_owner(target, &owners)?;
        if claiming_owner == &target_owner {
            continue;
        }
        let claiming_package =
            package_by_subject
                .get(claiming_owner.as_str())
                .ok_or_else(|| {
                    claim_error(
                        feature,
                        relation,
                        target,
                        claiming_owner,
                        &target_owner,
                        "claiming owner is not a repository package",
                    )
                })?;
        let target_package = subject_package(&target_owner).ok_or_else(|| {
            claim_error(
                feature,
                relation,
                target,
                claiming_owner,
                &target_owner,
                "target owner is not a crate package",
            )
        })?;
        let dependency = claiming_package
            .source_dependencies
            .iter()
            .find(|dependency| dependency.package == target_package);
        let delegated = overlay
            .relations()
            .any(|(from, rel, to)| from == feature && rel == "delegates-to" && to == target);
        if delegated {
            continue;
        }
        let Some(dependency) = dependency else {
            return Err(claim_error(
                feature,
                relation,
                target,
                claiming_owner,
                &target_owner,
                "missing normal dependency; change the claim or the code",
            ));
        };
        let facts = source_use_facts(repo, claiming_package, &dependency.crate_name);
        if facts.is_empty() {
            return Err(claim_error(
                feature,
                relation,
                target,
                claiming_owner,
                &target_owner,
                &format!(
                    "dependency {} exists, but no reachable call, impl, or type-use source fact names crate `{}`; change the claim or the code",
                    dependency.package, dependency.crate_name
                ),
            ));
        }
    }
    Ok(())
}

fn resolve_target_owner(
    target: &str,
    owners: &BTreeMap<String, SubjectId>,
) -> Result<SubjectId, String> {
    if target.starts_with("crate/") {
        return Ok(SubjectId::new(target));
    }
    owners.get(target).cloned().ok_or_else(|| {
        format!("authored composition target {target} does not resolve to a feature or crate owner")
    })
}

fn subject_package(subject: &SubjectId) -> Option<&str> {
    subject.as_str().strip_prefix("crate/")
}

fn claim_error(
    feature: &str,
    relation: &str,
    target: &str,
    owner: &SubjectId,
    target_owner: &SubjectId,
    fact: &str,
) -> String {
    format!(
        "false authored composition claim: feature {feature} ({owner}) {relation} {target} ({target_owner}); source fact: {fact}"
    )
}

#[derive(Default)]
struct UseFactVisitor<'a> {
    dependency: &'a str,
    facts: Vec<String>,
}

impl<'ast> Visit<'ast> for UseFactVisitor<'_> {
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        let tokens = item.tree.to_token_stream().to_string();
        if first_token(&tokens) == self.dependency {
            self.facts.push(format!("type-use `{}`", compact(&tokens)));
        }
        syn::visit::visit_item_use(self, item);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = call.func.as_ref()
            && path_starts_with(&path.path, self.dependency)
        {
            self.facts.push(format!(
                "call `{}`",
                compact(&call.func.to_token_stream().to_string())
            ));
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let trait_uses_dependency = item
            .trait_
            .as_ref()
            .is_some_and(|(_, path, _)| path_starts_with(path, self.dependency));
        let self_uses_dependency = type_starts_with(&item.self_ty, self.dependency);
        if trait_uses_dependency || self_uses_dependency {
            self.facts.push(format!(
                "impl `{}`",
                compact(&item.to_token_stream().to_string())
            ));
        }
        syn::visit::visit_item_impl(self, item);
    }

    fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
        if path_starts_with(&ty.path, self.dependency) {
            self.facts.push(format!(
                "type-use `{}`",
                compact(&ty.to_token_stream().to_string())
            ));
        }
        syn::visit::visit_type_path(self, ty);
    }
}

fn source_use_facts(repo: &Path, package: &PackageContract, dependency: &str) -> Vec<String> {
    let mut facts = Vec::new();
    for path in package_rust_files(repo, package) {
        let relative = rel_path(repo, &path);
        if is_test_source(&relative) {
            continue;
        }
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(file) = syn::parse_file(&source) else {
            continue;
        };
        let mut visitor = UseFactVisitor {
            dependency,
            facts: Vec::new(),
        };
        visitor.visit_file(&file);
        facts.extend(
            visitor
                .facts
                .into_iter()
                .map(|fact| format!("{relative}: {fact}")),
        );
    }
    facts.sort();
    facts.dedup();
    facts
}

fn path_starts_with(path: &syn::Path, dependency: &str) -> bool {
    path.segments
        .first()
        .is_some_and(|segment| segment.ident == dependency)
}

fn type_starts_with(ty: &syn::Type, dependency: &str) -> bool {
    matches!(ty, syn::Type::Path(path) if path_starts_with(&path.path, dependency))
}

fn first_token(tokens: &str) -> &str {
    tokens.split_whitespace().next().unwrap_or_default()
}

fn compact(tokens: &str) -> String {
    tokens.split_whitespace().collect()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::Value;
    use sim_index_core::{IndexDoc, SubjectId, SubjectRecord};

    use super::*;
    use crate::{index_author::parse_overlay, repo_contract::SourceDependency};

    #[test]
    fn landed_javascript_json_composition_has_dependency_and_type_use() {
        let fixture = Fixture::new(
            "use sim_codec_json::JsonCodec;\npub fn codec() -> JsonCodec { todo!() }\n",
        );
        let overlay = parse_overlay(&overlay("composes")).expect("parse overlay");

        check_authored_composition(&fixture.root, &fixture.packages(), &fixture.doc(), &overlay)
            .expect("source-backed composition");
    }

    #[test]
    fn false_claim_names_feature_owner_and_missing_exact_source_fact() {
        let fixture =
            Fixture::new("use unrelated_dependency::Other;\npub fn unrelated(_: Other) {}\n");
        let overlay = parse_overlay(&overlay("reuses")).expect("parse overlay");

        let error = check_authored_composition(
            &fixture.root,
            &fixture.packages(),
            &fixture.doc(),
            &overlay,
        )
        .expect_err("an unrelated dependency must not prove composition");

        assert!(error.contains("feature/runtime/javascript-json"));
        assert!(error.contains("crate/javascript-runtime"));
        assert!(error.contains("crate/sim-codec-json"));
        assert!(error.contains("no reachable call, impl, or type-use source fact"));
        assert!(error.contains("sim_codec_json"));
    }

    #[test]
    fn explicit_checked_delegation_explains_indirection() {
        let fixture = Fixture::new("pub fn delegates() {}\n");
        let source = overlay("composes").replace(
            "composes = [\"crate/sim-codec-json\"]",
            "composes = [\"crate/sim-codec-json\"]\ndelegates_to = [\"crate/sim-codec-json\"]",
        );
        let overlay = parse_overlay(&source).expect("parse overlay");

        check_authored_composition(&fixture.root, &fixture.packages(), &fixture.doc(), &overlay)
            .expect("explicit delegation");
    }

    fn overlay(relation: &str) -> String {
        format!(
            "schema = \"sim.features\"\n\n[[feature]]\nid = \"feature/runtime/javascript-json\"\ntitle = \"JavaScript JSON\"\nsummary = \"Uses the shared JSON model.\"\nowner = \"crate/javascript-runtime\"\n{relation} = [\"crate/sim-codec-json\"]\n"
        )
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(source: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("sim-index-composition-{nonce}"));
            fs::create_dir_all(root.join("runtime/src")).expect("fixture directory");
            fs::write(root.join("runtime/src/lib.rs"), source).expect("fixture source");
            Self { root }
        }

        fn packages(&self) -> Vec<PackageContract> {
            vec![PackageContract {
                name: "javascript-runtime".to_owned(),
                crate_name: "javascript_runtime".to_owned(),
                manifest: "runtime/Cargo.toml".to_owned(),
                root: "runtime".to_owned(),
                group: "runtime".to_owned(),
                publish: "true".to_owned(),
                description: String::new(),
                target_kinds: vec!["lib".to_owned()],
                targets: Vec::<Value>::new(),
                dependencies: Vec::<Value>::new(),
                source_dependencies: vec![
                    SourceDependency {
                        package: "sim-codec-json".to_owned(),
                        crate_name: "sim_codec_json".to_owned(),
                        kind: "normal".to_owned(),
                    },
                    SourceDependency {
                        package: "unrelated-dependency".to_owned(),
                        crate_name: "unrelated_dependency".to_owned(),
                        kind: "normal".to_owned(),
                    },
                ],
                features: Vec::<Value>::new(),
                rustdoc_summary: String::new(),
            }]
        }

        fn doc(&self) -> IndexDoc {
            let mut doc = IndexDoc::public("test");
            doc.subjects.push(SubjectRecord {
                id: SubjectId::new("crate/javascript-runtime"),
                kind: "crate".to_owned(),
                title: "javascript-runtime".to_owned(),
            });
            doc
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
