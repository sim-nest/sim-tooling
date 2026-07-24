//! Export the public SIM Index into a managed Markdown vault namespace.

use std::{collections::BTreeMap, fs, path::PathBuf};

use sim_index_core::IndexDoc;

use crate::{
    generated_namespace::{ManagedNamespace, NamespaceDiff},
    index_render::load_doc,
    index_vault_graph::{VaultGranularity, VaultGraph},
    index_vault_manifest::{VaultManifestSeed, sha256_digest},
    index_vault_profile::resolve_profile,
    index_vault_render::{VaultRender, render_vault},
    index_vault_render_writer::granularity_label,
};

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = IndexExportOptions::parse(&args)?;
    let report = export(options)?;
    println!("{}", report.summary());
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IndexExportOptions {
    pub(crate) input: PathBuf,
    pub(crate) profile: String,
    pub(crate) vault_root: PathBuf,
    pub(crate) namespace: PathBuf,
    pub(crate) granularity: VaultGranularity,
    pub(crate) mode: ExportMode,
}

impl IndexExportOptions {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let program = args.first().map(String::as_str).unwrap_or("xtask");
        if !matches!(args.get(1).map(String::as_str), Some("index"))
            || !matches!(args.get(2).map(String::as_str), Some("export"))
        {
            return Err(usage(program));
        }

        let mut input = None;
        let mut profile = None;
        let mut vault_root = None;
        let mut namespace = None;
        let mut granularity = None;
        let mut plan = false;
        let mut check = false;
        let mut index = 3;
        while index < args.len() {
            match args[index].as_str() {
                "--input" => {
                    set_once_path(&mut input, args, &mut index, "--input")?;
                }
                "--profile" => {
                    set_once_string(&mut profile, args, &mut index, "--profile")?;
                }
                "--vault-root" => {
                    set_once_path(&mut vault_root, args, &mut index, "--vault-root")?;
                }
                "--namespace" => {
                    set_once_path(&mut namespace, args, &mut index, "--namespace")?;
                }
                "--granularity" => {
                    if granularity.is_some() {
                        return Err("duplicate index export argument `--granularity`".to_owned());
                    }
                    index += 1;
                    let value = args.get(index).ok_or("--granularity requires a value")?;
                    granularity = Some(parse_granularity(value)?);
                }
                "--plan" => {
                    if plan {
                        return Err("duplicate index export argument `--plan`".to_owned());
                    }
                    if check {
                        return Err(
                            "index export arguments `--plan` and `--check` are mutually exclusive"
                                .to_owned(),
                        );
                    }
                    plan = true;
                }
                "--check" => {
                    if check {
                        return Err("duplicate index export argument `--check`".to_owned());
                    }
                    if plan {
                        return Err(
                            "index export arguments `--plan` and `--check` are mutually exclusive"
                                .to_owned(),
                        );
                    }
                    check = true;
                }
                "-h" | "--help" => return Err(usage(program)),
                other => {
                    return Err(format!(
                        "unknown index export argument `{other}`; {}",
                        usage(program)
                    ));
                }
            }
            index += 1;
        }

        Ok(Self {
            input: input
                .ok_or_else(|| format!("index export requires --input; {}", usage(program)))?,
            profile: profile
                .ok_or_else(|| format!("index export requires --profile; {}", usage(program)))?,
            vault_root: vault_root
                .ok_or_else(|| format!("index export requires --vault-root; {}", usage(program)))?,
            namespace: namespace.unwrap_or_else(|| PathBuf::from("SIM-Index")),
            granularity: granularity.unwrap_or(VaultGranularity::Compact),
            mode: if plan {
                ExportMode::Plan
            } else if check {
                ExportMode::Check
            } else {
                ExportMode::Write
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExportMode {
    Plan,
    Check,
    Write,
}

impl ExportMode {
    fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Check => "check",
            Self::Write => "write",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IndexExportReport {
    pub(crate) mode: ExportMode,
    pub(crate) profile_id: String,
    pub(crate) graph_digest: String,
    pub(crate) namespace: String,
    pub(crate) granularity: String,
    pub(crate) rows: RowCounts,
    pub(crate) note_count: usize,
    pub(crate) artifact_count: usize,
    pub(crate) byte_count: usize,
    pub(crate) changed_artifacts: usize,
    pub(crate) unchanged_artifacts: usize,
    pub(crate) unresolved_links: usize,
    pub(crate) target: PathBuf,
}

impl IndexExportReport {
    pub(crate) fn summary(&self) -> String {
        format!(
            "index export {}: profile={} graph={} namespace={} granularity={} rows subjects={} anchors={} surfaces={} specimens={} drafts={} features={} routes={} edges={} notes={} artifacts={} bytes={} changed={} unchanged={} unresolved_links={} target={}",
            self.mode.label(),
            self.profile_id,
            self.graph_digest,
            self.namespace,
            self.granularity,
            self.rows.subjects,
            self.rows.anchors,
            self.rows.surfaces,
            self.rows.specimens,
            self.rows.drafts,
            self.rows.features,
            self.rows.routes,
            self.rows.edges,
            self.note_count,
            self.artifact_count,
            self.byte_count,
            self.changed_artifacts,
            self.unchanged_artifacts,
            self.unresolved_links,
            self.target.display()
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RowCounts {
    pub(crate) subjects: usize,
    pub(crate) anchors: usize,
    pub(crate) surfaces: usize,
    pub(crate) specimens: usize,
    pub(crate) drafts: usize,
    pub(crate) features: usize,
    pub(crate) routes: usize,
    pub(crate) edges: usize,
}

pub(crate) fn export(options: IndexExportOptions) -> Result<IndexExportReport, String> {
    let input_bytes = fs::read(&options.input)
        .map_err(|err| format!("read {}: {err}", options.input.display()))?;
    let graph_digest = sha256_digest(&input_bytes);
    let doc = load_doc(&options.input)?;
    let profile = resolve_profile(&options.profile)?;
    let graph = VaultGraph::from_index(&doc)?;
    let rendered = render_vault(&graph, profile, options.granularity)?;
    let seed = VaultManifestSeed::new(
        profile.id,
        granularity_label(options.granularity),
        graph_digest.clone(),
        manifest_coverage(&doc, &rendered),
    )?;
    let namespace = ManagedNamespace::open(options.vault_root, options.namespace)?;

    let plan = namespace.plan(&seed, &rendered.artifacts);
    let diff = match options.mode {
        ExportMode::Plan => NamespaceDiff {
            namespace: plan.namespace.clone(),
            changed_artifacts: plan.artifact_count,
            unchanged_artifacts: 0,
        },
        ExportMode::Check => {
            let diff = namespace.diff(&seed, &rendered.artifacts)?;
            namespace.check(&seed, &rendered.artifacts)?;
            diff
        }
        ExportMode::Write => {
            let diff = namespace.diff(&seed, &rendered.artifacts)?;
            if diff.changed_artifacts != 0 {
                namespace.preflight(&seed, &rendered.artifacts)?.commit()?;
            }
            diff
        }
    };

    Ok(IndexExportReport {
        mode: options.mode,
        profile_id: profile.id.to_owned(),
        graph_digest,
        namespace: diff.namespace,
        granularity: granularity_label(options.granularity).to_owned(),
        rows: row_counts(&doc),
        note_count: markdown_artifacts(&rendered),
        artifact_count: plan.artifact_count,
        byte_count: plan.byte_count,
        changed_artifacts: diff.changed_artifacts,
        unchanged_artifacts: diff.unchanged_artifacts,
        unresolved_links: rendered.unresolved_links().len(),
        target: plan.target,
    })
}

fn set_once_path(
    slot: &mut Option<PathBuf>,
    args: &[String],
    index: &mut usize,
    flag: &str,
) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("duplicate index export argument `{flag}`"));
    }
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| format!("{flag} requires a path"))?;
    if value.trim().is_empty() {
        return Err(format!("{flag} requires a non-empty path"));
    }
    *slot = Some(PathBuf::from(value));
    Ok(())
}

fn set_once_string(
    slot: &mut Option<String>,
    args: &[String],
    index: &mut usize,
    flag: &str,
) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("duplicate index export argument `{flag}`"));
    }
    *index += 1;
    let value = args
        .get(*index)
        .ok_or_else(|| format!("{flag} requires a value"))?;
    if value.trim().is_empty() {
        return Err(format!("{flag} requires a non-empty value"));
    }
    *slot = Some(value.to_owned());
    Ok(())
}

fn parse_granularity(value: &str) -> Result<VaultGranularity, String> {
    match value {
        "compact" => Ok(VaultGranularity::Compact),
        "full" => Ok(VaultGranularity::Full),
        other => Err(format!(
            "unknown Index vault granularity `{other}`; expected compact or full"
        )),
    }
}

fn row_counts(doc: &IndexDoc) -> RowCounts {
    RowCounts {
        subjects: doc.subjects.len(),
        anchors: doc.anchors.len(),
        surfaces: doc.surfaces.len(),
        specimens: doc.specimens.len(),
        drafts: doc.drafts.len(),
        features: doc.features.len(),
        routes: doc.routes.len(),
        edges: doc.edges.len(),
    }
}

fn manifest_coverage(doc: &IndexDoc, rendered: &VaultRender) -> BTreeMap<String, u64> {
    let rows = row_counts(doc);
    BTreeMap::from([
        ("subjects".to_owned(), rows.subjects as u64),
        ("anchors".to_owned(), rows.anchors as u64),
        ("surfaces".to_owned(), rows.surfaces as u64),
        ("specimens".to_owned(), rows.specimens as u64),
        ("drafts".to_owned(), rows.drafts as u64),
        ("features".to_owned(), rows.features as u64),
        ("routes".to_owned(), rows.routes as u64),
        ("edges".to_owned(), rows.edges as u64),
        ("notes".to_owned(), markdown_artifacts(rendered) as u64),
        (
            "artifacts".to_owned(),
            rendered.artifacts.iter().count() as u64,
        ),
        (
            "represented_rows".to_owned(),
            rendered.coverage().represented_rows() as u64,
        ),
        (
            "represented_relations".to_owned(),
            rendered.coverage().represented_relations() as u64,
        ),
        (
            "link_targets".to_owned(),
            rendered.coverage().link_targets() as u64,
        ),
        (
            "unresolved_links".to_owned(),
            rendered.unresolved_links().len() as u64,
        ),
    ])
}

fn markdown_artifacts(rendered: &VaultRender) -> usize {
    rendered
        .artifacts
        .iter()
        .filter(|artifact| artifact.path.extension().is_some_and(|ext| ext == "md"))
        .count()
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} index export --input <index.sx> --profile <profile> --vault-root <dir> [--namespace <relative-path>] [--granularity compact|full] [--plan|--check]"
    )
}
