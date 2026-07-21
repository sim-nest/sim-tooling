//! Per-repository SIM Index freshness and coverage gate.

use std::{
    fs,
    path::{Path, PathBuf},
};

use sim_codec_index::{IndexCodec, IndexForm};
use sim_index_core::{IndexDoc, check_index_doc};

use crate::index_rules::{CoverageReport, Strictness, check_coverage};

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = CheckOptions::parse(&args)?;
    let mut strictness = Strictness::load(&options.repo)?;
    if let Some(selectors) = &options.strict {
        strictness.apply_strict_selectors(selectors)?;
    }
    let report = index_check(&options.repo, &strictness)?;
    print_report(&report.coverage);
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CheckOptions {
    repo: PathBuf,
    strict: Option<String>,
}

impl CheckOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let program = args.first().map(String::as_str).unwrap_or("xtask");
        if args.get(1).map(String::as_str) != Some("index-check") {
            return Err(usage(program));
        }

        let mut repo = None;
        let mut strict = None;
        let mut index = 2;
        while index < args.len() {
            match args[index].as_str() {
                "--repo" => {
                    index += 1;
                    repo = Some(PathBuf::from(
                        args.get(index).ok_or("--repo requires a path")?.as_str(),
                    ));
                }
                "--strict" => {
                    index += 1;
                    strict = Some(
                        args.get(index)
                            .ok_or("--strict requires selector list")?
                            .to_owned(),
                    );
                }
                other => {
                    return Err(format!(
                        "unknown index-check argument `{other}`; {}",
                        usage(program)
                    ));
                }
            }
            index += 1;
        }

        Ok(Self {
            repo: repo.unwrap_or_else(|| PathBuf::from(".")),
            strict,
        })
    }
}

fn usage(program: &str) -> String {
    format!("usage: {program} index-check --repo <path> [--strict <category:value,...>]")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IndexCheckReport {
    pub(crate) coverage: CoverageReport,
}

pub(crate) fn index_check(
    repo: &Path,
    strictness: &Strictness,
) -> Result<IndexCheckReport, String> {
    let source = read_generated_fragment(repo)?;
    let doc = decode_fragment(repo, &source)?;
    check_index_doc(&doc).map_err(|err| format!("invalid index fragment: {err}"))?;
    assert_fragment_fresh(repo, &source)?;
    let coverage = check_coverage(&doc, strictness)?;
    Ok(IndexCheckReport { coverage })
}

fn read_generated_fragment(repo: &Path) -> Result<String, String> {
    let path = fragment_path(repo);
    fs::read_to_string(&path)
        .map_err(|err| format!("read generated index fragment {}: {err}", path.display()))
}

fn decode_fragment(repo: &Path, source: &str) -> Result<IndexDoc, String> {
    IndexCodec.decode(IndexForm::Sx, source).map_err(|err| {
        format!(
            "decode generated index fragment {}: {err}",
            fragment_path(repo).display()
        )
    })
}

fn assert_fragment_fresh(repo: &Path, current: &str) -> Result<(), String> {
    let artifacts = crate::repo_contract::contract_artifacts(repo)?;
    let expected = artifacts
        .files
        .get("sim-index-fragment.sx")
        .ok_or("repo-contract did not produce sim-index-fragment.sx")?;
    assert_fragment_fresh_from_sources(current, expected)
}

fn assert_fragment_fresh_from_sources(current: &str, expected: &str) -> Result<(), String> {
    if current == expected {
        Ok(())
    } else {
        Err("stale fragment: docs/generated/sim-index-fragment.sx is stale; run `cargo run -p xtask -- repo-contract --repo .`".to_owned())
    }
}

fn fragment_path(repo: &Path) -> PathBuf {
    repo.join("docs/generated/sim-index-fragment.sx")
}

fn print_report(report: &CoverageReport) {
    for item in &report.advisory_missing {
        println!("index-check: advisory {} {}", item.kind.as_str(), item.id);
    }
    println!(
        "index-check: ok (covered {}, advisory_missing {})",
        report.covered,
        report.advisory_missing.len()
    );
}

#[cfg(test)]
mod tests {
    use sim_index_core::{
        DiscoveredSpecimen, FeatureId, FeatureRecord, SpecimenId, SubjectId, SubjectRecord,
        Visibility, key::CanonicalFeatureKey,
    };

    use super::*;

    #[test]
    fn stale_fragment_comparison_fails() {
        let err = assert_fragment_fresh_from_sources("old", "new").unwrap_err();

        assert!(err.contains("stale fragment"));
    }

    #[test]
    fn non_runnable_specimen_claim_fails_before_coverage() {
        let mut doc = IndexDoc {
            schema: "sim.index".to_owned(),
            generated_by: "test".to_owned(),
            visibility: Visibility::Public,
            subjects: vec![SubjectRecord {
                id: SubjectId::new("crate/demo"),
                kind: "crate".to_owned(),
                title: "demo".to_owned(),
            }],
            anchors: Vec::new(),
            surfaces: Vec::new(),
            specimens: vec![DiscoveredSpecimen {
                id: SpecimenId::new("recipe/demo/manual"),
                subject: SubjectId::new("crate/demo"),
                kind: "recipe".to_owned(),
                path: "recipes/manual/recipe.toml".to_owned(),
                language: None,
                runnable: false,
                checked: false,
                checked_by: None,
                doc_anchor: None,
            }],
            drafts: Vec::new(),
            features: Vec::new(),
            routes: Vec::new(),
            edges: Vec::new(),
        };
        doc.features.push(FeatureRecord {
            id: FeatureId::new("feature/demo/manual"),
            key: CanonicalFeatureKey::new("crate/demo/feature-demo-manual"),
            subject: SubjectId::new("crate/demo"),
            title: "Manual".to_owned(),
            summary: "Manual specimen.".to_owned(),
            anchors: Vec::new(),
            surfaces: Vec::new(),
            specimens: vec![SpecimenId::new("recipe/demo/manual")],
            grammar_contracts: Vec::new(),
            doc_anchor: None,
        });

        let err = check_index_doc(&doc).unwrap_err().to_string();

        assert!(err.contains("non-runnable specimen"));
    }
}
