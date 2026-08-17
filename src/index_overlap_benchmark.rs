//! Benchmark runner and statistics ownership checks for the overlap board.

use std::collections::{BTreeMap, BTreeSet};

use sim_index_core::{DeclarationRole, IndexDoc};

use super::Finding;

const RUNNER_OWNER: &str = "crate/xtask";
const STATS_OWNER: &str = "crate/sim-lib-numbers-stats";
const RUNNER_FEATURE: &str = "feature/sim-tooling/benchmark-cli";
const STATS_FEATURE: &str = "feature/sim-numbers/statistical-inference";
const OWNERSHIP_RELS: &[&str] = &["reuses", "composes", "delegates-to"];

/// Raises public declarations that establish a second benchmark runner or a
/// second implementation of benchmark summary statistics. Workload functions
/// are intentionally absent from the vocabulary: they supply work to the
/// runner and do not own sampling, comparison, or summaries.
pub(super) fn benchmark_ownership_findings(doc: &IndexDoc) -> Vec<Finding> {
    let subjects = doc
        .anchors
        .iter()
        .map(|anchor| (anchor.id.as_str(), anchor.subject.as_str()))
        .collect::<BTreeMap<_, _>>();
    let claims = doc
        .features
        .iter()
        .flat_map(|feature| {
            feature
                .anchors
                .iter()
                .map(move |anchor| (anchor.as_str(), feature.id.as_str()))
        })
        .collect::<BTreeMap<_, _>>();

    doc.declarations
        .iter()
        .filter(|declaration| declaration.role == DeclarationRole::Function)
        .filter_map(|declaration| {
            let subject = subjects.get(declaration.anchor.as_str()).copied()?;
            let symbol = declaration
                .module_path
                .rsplit("::")
                .next()
                .unwrap_or(declaration.module_path.as_str());
            let (owner, feature, kind) = if is_statistics_implementation(symbol) {
                (STATS_OWNER, STATS_FEATURE, "statistics")
            } else if is_runner_implementation(symbol) {
                (RUNNER_OWNER, RUNNER_FEATURE, "runner")
            } else {
                return None;
            };
            if subject == owner
                || claims
                    .get(declaration.anchor.as_str())
                    .is_some_and(|claim| has_explicit_owner_relation(doc, claim, feature))
            {
                return None;
            }
            Some(ownership_finding(kind, symbol, subject, owner, feature))
        })
        .collect()
}

fn ownership_finding(
    kind: &str,
    symbol: &str,
    subject: &str,
    owner: &str,
    owner_feature: &str,
) -> Finding {
    Finding {
        cluster: format!("benchmark-ownership/{kind}"),
        member: None,
        left: None,
        right: None,
        source_classification: Some("implementation".to_owned()),
        graph_relation: Some("reuses|composes|delegates-to".to_owned()),
        reason: format!("duplicate-benchmark-{kind}-owner"),
        detail: format!(
            "{subject} declares `{symbol}` outside canonical owner {owner}; move the implementation to its owner or author a checked reuse/composition/delegation path to {owner_feature}"
        ),
        strict: true,
    }
}

fn is_statistics_implementation(symbol: &str) -> bool {
    matches!(
        symbol,
        "mean"
            | "variance"
            | "median"
            | "median_absolute_deviation"
            | "bootstrap_mean_difference_interval"
    )
}

fn is_runner_implementation(symbol: &str) -> bool {
    matches!(
        symbol,
        "run_benchmark" | "benchmark_run" | "measure_benchmark" | "compare_benchmarks"
    )
}

fn has_explicit_owner_relation(doc: &IndexDoc, from: &str, owner: &str) -> bool {
    let mut pending = vec![from];
    let mut seen = BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !seen.insert(current) {
            continue;
        }
        for edge in doc.edges.iter().filter(|edge| {
            edge.from.as_str() == current && OWNERSHIP_RELS.contains(&edge.rel.as_str())
        }) {
            if edge.to.as_str() == owner {
                return true;
            }
            pending.push(edge.to.as_str());
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use sim_index_core::{
        AnchorId, DeclarationFact, DeclarationRole, DiscoveredAnchor, IndexDoc, SourceLocation,
        SubjectId, SyntaxBound,
    };

    use super::*;

    #[test]
    fn rejects_a_second_mean_but_ignores_a_workload_definition() {
        let mut doc = IndexDoc::public("benchmark-ownership");
        add_function(&mut doc, "crate/example", "mean");
        add_function(&mut doc, "crate/example", "decode_workload");

        let findings = benchmark_ownership_findings(&doc);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].reason, "duplicate-benchmark-statistics-owner");
        assert!(findings[0].detail.contains(STATS_OWNER));
    }

    #[test]
    fn accepts_the_canonical_statistics_and_runner_owners() {
        let mut doc = IndexDoc::public("benchmark-owners");
        add_function(&mut doc, STATS_OWNER, "mean");
        add_function(&mut doc, RUNNER_OWNER, "run_benchmark");

        assert!(benchmark_ownership_findings(&doc).is_empty());
    }

    fn add_function(doc: &mut IndexDoc, subject: &str, symbol: &str) {
        let anchor = AnchorId::new(format!(
            "anchor/test/{}/{}",
            subject.replace('/', "-"),
            symbol
        ));
        doc.anchors.push(DiscoveredAnchor {
            id: anchor.clone(),
            subject: SubjectId::new(subject),
            kind: "rustdoc-item".to_owned(),
        });
        doc.declarations.push(DeclarationFact {
            anchor,
            role: DeclarationRole::Function,
            module_path: symbol.to_owned(),
            generics: String::new(),
            members: Vec::new(),
            location: SourceLocation {
                file: "src/lib.rs".to_owned(),
                declaration: 0,
            },
            syntax_bound: SyntaxBound {
                max_bytes: 16_384,
                truncated: false,
            },
        });
    }
}
