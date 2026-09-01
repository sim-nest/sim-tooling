use super::{
    guard::{GuidelineFinding, GuidelineRule},
    index_manifest::RepoEntry,
};

mod files;
mod rules;

const PRESENT_TENSE_NEEDLES: &[&str] = &[
    "ROADMAP_",
    "source_roadmap",
    "final_proof_phase",
    "predecessor_assumption",
    "landed_source",
    "previous roadmap",
    "predecessor roadmap",
    "future roadmap",
    "future phase",
    "later phase",
    "TODO(roadmap)",
];

const KERNEL_BOUNDARY_NEEDLES: &[&str] = &[
    "parse_json",
    "parse_lisp",
    "JsonParser",
    "LispParser",
    "StandardArithmetic",
    "BigInt",
    "BigRational",
    "parallel map",
];

pub(super) fn scan_repo(
    repo: &RepoEntry,
    guard_rules: &[GuidelineRule],
) -> Result<Vec<GuidelineFinding>, String> {
    rules::scan_repo(repo, guard_rules)
}
