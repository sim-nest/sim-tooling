use super::{
    guard::{GuidelineFinding, GuidelineRule},
    index_manifest::RepoEntry,
};

mod files;
mod rules;

const PRESENT_TENSE_NEEDLES: &[&str] = &[
    "ROADMAP_",
    "REORG_",
    "Phase ",
    "previously",
    "historically",
    "formerly",
    "legacy",
    "migration",
    "migrated",
    "future",
    "planned",
    "not yet complete",
    "will be added",
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
