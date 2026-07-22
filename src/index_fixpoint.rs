//! Reflexive SIM Index fixpoint and global strict coverage gate.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use sim_index_core::{FeatureRecord, IndexDoc};

use crate::{
    index_author,
    index_merge::{encode_sx, merge_fragment_paths},
    index_render::load_doc,
    index_rules::{Strictness, feature_has_runnable_specimen, route_coverage_gaps},
};

const SELF_FEATURE_ID: &str = "feature/sim-index/core";
const SELF_REQUIRED_AUDIENCES: [&str; 3] = ["user", "code", "framework"];
const SELF_REQUIRED_SURFACES: [&str; 2] = ["cli/xtask", "docs/sim-tooling/generated"];

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = FixpointOptions::parse(&args)?;
    let mut strictness = Strictness::default();
    if let Some(selectors) = &options.strict {
        strictness.apply_strict_selectors(selectors)?;
    }
    let report = assert_fixpoint(
        &options.input,
        &options.fragments,
        &options.self_feature_repo,
        &strictness,
    )?;
    println!(
        "index fixpoint: committed index is current ({} fragment(s), route_gaps {})",
        report.fragments, report.route_gaps
    );
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FixpointOptions {
    input: PathBuf,
    fragments: Vec<PathBuf>,
    strict: Option<String>,
    self_feature_repo: PathBuf,
}

impl FixpointOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let program = args.first().map(String::as_str).unwrap_or("xtask");
        if !matches!(args.get(1).map(String::as_str), Some("index"))
            || !matches!(args.get(2).map(String::as_str), Some("fixpoint"))
        {
            return Err(usage(program));
        }

        let mut input = None;
        let mut fragments = Vec::new();
        let mut strict = None;
        let mut self_feature_repo = None;
        let mut index = 3;
        while index < args.len() {
            match args[index].as_str() {
                "--input" => {
                    index += 1;
                    input = Some(PathBuf::from(
                        args.get(index).ok_or("--input requires a path")?,
                    ));
                }
                "--fragment" => {
                    index += 1;
                    fragments.push(PathBuf::from(
                        args.get(index).ok_or("--fragment requires a path")?,
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
                "--self-feature-repo" => {
                    index += 1;
                    self_feature_repo = Some(PathBuf::from(
                        args.get(index)
                            .ok_or("--self-feature-repo requires a path")?,
                    ));
                }
                "-h" | "--help" => return Err(usage(program)),
                other => {
                    return Err(format!(
                        "unknown index fixpoint argument `{other}`; {}",
                        usage(program)
                    ));
                }
            }
            index += 1;
        }

        if fragments.is_empty() {
            return Err(format!(
                "index fixpoint requires at least one --fragment; {}",
                usage(program)
            ));
        }
        Ok(Self {
            input: input
                .ok_or_else(|| format!("index fixpoint requires --input; {}", usage(program)))?,
            fragments,
            strict,
            self_feature_repo: self_feature_repo.ok_or_else(|| {
                format!(
                    "index fixpoint requires --self-feature-repo; {}",
                    usage(program)
                )
            })?,
        })
    }
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} index fixpoint --input <index.sx> --fragment <path>... --self-feature-repo <path> [--strict <selectors>]"
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FixpointReport {
    fragments: usize,
    route_gaps: usize,
}

fn assert_fixpoint(
    committed: &Path,
    fragments: &[PathBuf],
    self_feature_repo: &Path,
    strictness: &Strictness,
) -> Result<FixpointReport, String> {
    let regenerated = merge_fragment_paths(fragments, true)?;
    let fresh = encode_sx(&regenerated)?;
    let committed_text = fs::read_to_string(committed)
        .map_err(|err| format!("read {}: {err}", committed.display()))?;
    if fresh != committed_text {
        return Err("index fixpoint broken: regenerated index.sx differs".to_owned());
    }

    let committed_doc = load_doc(committed)?;
    let route_gaps = route_coverage_gaps(&committed_doc);
    if strictness.requires_any_route() {
        for gap in &route_gaps {
            if strictness.requires_route(&gap.category) {
                return Err(format!("unrouted {}: {}", gap.category, gap.id));
            }
        }
    }
    assert_self_feature(&committed_doc, self_feature_repo)?;

    Ok(FixpointReport {
        fragments: fragments.len(),
        route_gaps: route_gaps.len(),
    })
}

fn assert_self_feature(doc: &IndexDoc, self_feature_repo: &Path) -> Result<(), String> {
    let feature = doc
        .features
        .iter()
        .find(|feature| feature.id.as_str() == SELF_FEATURE_ID)
        .ok_or_else(|| format!("missing self feature {SELF_FEATURE_ID}"))?;
    assert_self_audiences(self_feature_repo)?;
    assert_self_surfaces(doc, feature)?;
    if !feature_has_runnable_specimen(doc, feature) {
        return Err(format!(
            "self feature {SELF_FEATURE_ID} lacks a runnable checked specimen"
        ));
    }
    Ok(())
}

fn assert_self_audiences(self_feature_repo: &Path) -> Result<(), String> {
    let audiences = index_author::feature_audiences(self_feature_repo)?;
    let Some(audiences) = audiences.get(SELF_FEATURE_ID) else {
        return Err(format!(
            "self feature {SELF_FEATURE_ID} is missing authored audiences"
        ));
    };
    let missing = SELF_REQUIRED_AUDIENCES
        .iter()
        .filter(|audience| !audiences.contains(**audience))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "self feature {SELF_FEATURE_ID} missing audience(s): {}",
            missing.join(", ")
        ))
    }
}

fn assert_self_surfaces(doc: &IndexDoc, feature: &FeatureRecord) -> Result<(), String> {
    let claimed = feature
        .surfaces
        .iter()
        .map(|id| id.as_str())
        .collect::<BTreeSet<_>>();
    let missing = SELF_REQUIRED_SURFACES
        .iter()
        .filter(|required| !claimed_surface_matches(doc, &claimed, required))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "self feature {SELF_FEATURE_ID} missing surface(s): {}",
            missing.join(", ")
        ))
    }
}

fn claimed_surface_matches(doc: &IndexDoc, claimed: &BTreeSet<&str>, required: &str) -> bool {
    claimed.iter().any(|id| {
        if *id == required {
            return true;
        }
        let Some(surface) = doc
            .surfaces
            .iter()
            .find(|surface| surface.id.as_str() == *id)
        else {
            return false;
        };
        match required {
            "cli/xtask" => {
                surface.kind == "cli"
                    && surface.id.as_str().ends_with("cli/xtask")
                    && surface
                        .subject
                        .as_str()
                        .ends_with("sim-tooling/crate/xtask")
            }
            "docs/sim-tooling/generated" => surface.id.as_str() == required,
            _ => false,
        }
    })
}

#[cfg(test)]
#[path = "index_fixpoint_tests.rs"]
mod tests;
