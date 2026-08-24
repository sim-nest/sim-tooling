//! Export the canonical SIM Index through the public vault projection and codec.

use crate::{
    generated_artifact::{ArtifactSet, GeneratedArtifact},
    generated_namespace::{ManagedNamespace, NamespaceDiff},
    index_render::load_doc,
    index_vault_manifest::{VaultManifestSeed, sha256_digest},
};
use sha2::{Digest, Sha256};
use sim_codec_index_vault::{
    VaultBundle, VaultEncoder, VaultVerification, resolve_profile, verify_v2,
};
use sim_index_core::IndexRow;
use sim_index_vault_core::{VaultGranularity, VaultProjection};
use sim_kernel::{ContentId, Symbol};
use std::{collections::BTreeMap, fs, path::PathBuf};

const MAX_MISMATCHES: usize = 64;
const MAX_MISMATCH_VALUE_BYTES: usize = 1024;

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    println!("{}", export(IndexExportOptions::parse(&args)?)?.summary());
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
        if args.get(1).map(String::as_str) != Some("index")
            || args.get(2).map(String::as_str) != Some("export")
        {
            return Err(usage(program));
        }
        let (mut input, mut profile, mut vault_root, mut namespace, mut granularity, mut mode) =
            (None, None, None, None, None, None);
        let mut i = 3;
        while i < args.len() {
            match args[i].as_str() {
                "--input" => set_once_path(&mut input, args, &mut i, "--input")?,
                "--profile" => set_once_string(&mut profile, args, &mut i, "--profile")?,
                "--vault-root" => set_once_path(&mut vault_root, args, &mut i, "--vault-root")?,
                "--namespace" => set_once_path(&mut namespace, args, &mut i, "--namespace")?,
                "--granularity" => {
                    if granularity.is_some() {
                        return Err("duplicate index export argument `--granularity`".into());
                    }
                    i += 1;
                    granularity = Some(parse_granularity(
                        args.get(i).ok_or("--granularity requires a value")?,
                    )?);
                }
                "--plan" => set_mode(&mut mode, ExportMode::Plan)?,
                "--verify" => set_mode(&mut mode, ExportMode::Verify)?,
                "--check" => set_mode(&mut mode, ExportMode::Check)?,
                "-h" | "--help" => return Err(usage(program)),
                other => {
                    return Err(format!(
                        "unknown index export argument `{other}`; {}",
                        usage(program)
                    ));
                }
            }
            i += 1;
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
            mode: mode.unwrap_or(ExportMode::Write),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExportMode {
    Plan,
    Verify,
    Check,
    Write,
}
impl ExportMode {
    fn label(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Verify => "verify",
            Self::Check => "check",
            Self::Write => "write",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IndexExportReport {
    pub(crate) mode: ExportMode,
    pub(crate) profile_id: String,
    pub(crate) projection_identity: String,
    pub(crate) namespace: String,
    pub(crate) granularity: String,
    pub(crate) family_counts: BTreeMap<String, usize>,
    pub(crate) note_count: usize,
    pub(crate) artifact_count: usize,
    pub(crate) byte_count: usize,
    pub(crate) changed_artifacts: usize,
    pub(crate) unchanged_artifacts: usize,
    pub(crate) bundle_root: String,
    pub(crate) verified: bool,
    pub(crate) mismatch_count: usize,
    pub(crate) target: PathBuf,
}
impl IndexExportReport {
    pub(crate) fn summary(&self) -> String {
        let families = self
            .family_counts
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "index export {}: profile={} projection={} namespace={} granularity={} families={} notes={} artifacts={} bytes={} changed={} unchanged={} bundle_root={} verified={} mismatches={} target={}",
            self.mode.label(),
            self.profile_id,
            self.projection_identity,
            self.namespace,
            self.granularity,
            families,
            self.note_count,
            self.artifact_count,
            self.byte_count,
            self.changed_artifacts,
            self.unchanged_artifacts,
            self.bundle_root,
            self.verified,
            self.mismatch_count,
            self.target.display()
        )
    }
}

pub(crate) fn export(options: IndexExportOptions) -> Result<IndexExportReport, String> {
    let input_bytes =
        fs::read(&options.input).map_err(|e| format!("read {}: {e}", options.input.display()))?;
    let doc = load_doc(&options.input)?; // sim-codec-index is the sole index.sx decoder.
    let projection = VaultProjection::from_complete(&doc, options.granularity)
        .map_err(|e| format!("project complete Index: {e}"))?;
    let profile =
        resolve_profile(&options.profile).map_err(|e| format!("resolve vault profile: {e}"))?;
    let bundle = VaultEncoder::new(profile)
        .encode(&projection)
        .map_err(|e| format!("encode vault bundle: {e}"))?;
    let artifacts = artifacts_from_bundle(&bundle)?; // independent path and case-fold gate.
    let family_counts = family_counts(&bundle);
    let seed = VaultManifestSeed::new(
        profile.id.as_str(),
        granularity_label(options.granularity),
        sha256_digest(&input_bytes),
        manifest_coverage(&bundle, &family_counts),
    )?;
    let namespace = ManagedNamespace::open(options.vault_root, options.namespace)?;
    let plan = namespace.plan(&seed, &artifacts);
    let mut verification = None;
    let diff = match options.mode {
        ExportMode::Plan => NamespaceDiff {
            namespace: plan.namespace.clone(),
            changed_artifacts: plan.artifact_count,
            unchanged_artifacts: 0,
        },
        ExportMode::Verify => {
            let current = namespace.current_bundle(&seed, &bundle)?;
            verification = Some(semantic_verify(&current, &projection)?);
            NamespaceDiff {
                namespace: plan.namespace.clone(),
                changed_artifacts: 0,
                unchanged_artifacts: plan.artifact_count,
            }
        }
        ExportMode::Check => {
            let diff = namespace.diff(&seed, &artifacts)?;
            namespace.check(&seed, &artifacts)?;
            let current = namespace.current_bundle(&seed, &bundle)?;
            verification = Some(semantic_verify(&current, &projection)?);
            diff
        }
        ExportMode::Write => {
            let diff = namespace.diff(&seed, &artifacts)?;
            if diff.changed_artifacts != 0 {
                namespace.preflight(&seed, &artifacts)?.commit()?;
            }
            diff
        }
    };
    let verified = verification
        .as_ref()
        .is_some_and(VaultVerification::is_success);
    let mismatch_count = verification.as_ref().map_or(0, |v| v.total_mismatches);
    Ok(IndexExportReport {
        mode: options.mode,
        profile_id: profile.id.as_str().into(),
        projection_identity: content_text(&bundle.projection_digest),
        namespace: diff.namespace,
        granularity: granularity_label(bundle.granularity).into(),
        family_counts,
        note_count: bundle
            .entries
            .iter()
            .filter(|e| e.note_kind.is_some())
            .count(),
        artifact_count: plan.artifact_count,
        byte_count: plan.byte_count,
        changed_artifacts: diff.changed_artifacts,
        unchanged_artifacts: diff.unchanged_artifacts,
        bundle_root: content_text(&bundle.bundle_root),
        verified,
        mismatch_count,
        target: plan.target,
    })
}

pub(crate) fn semantic_verify(
    bundle: &VaultBundle,
    projection: &VaultProjection,
) -> Result<VaultVerification, String> {
    let result = verify_v2(bundle, projection, MAX_MISMATCHES, MAX_MISMATCH_VALUE_BYTES)
        .map_err(|e| format!("verify vault semantics: {e}"))?;
    if result.is_success() {
        Ok(result)
    } else {
        Err(format!(
            "vault semantic verification failed with {} mismatch(es)",
            result.total_mismatches
        ))
    }
}
fn artifacts_from_bundle(bundle: &VaultBundle) -> Result<ArtifactSet, String> {
    ArtifactSet::new(
        bundle
            .entries
            .iter()
            .map(|e| GeneratedArtifact::new(&e.path, e.bytes.clone()))
            .collect::<Result<Vec<_>, _>>()?,
    )
}
fn family_counts(bundle: &VaultBundle) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for e in &bundle.entries {
        for (k, v) in &e.claim_families {
            *counts.entry(k.clone()).or_default() += v;
        }
    }
    counts
}
fn manifest_coverage(
    bundle: &VaultBundle,
    families: &BTreeMap<String, usize>,
) -> BTreeMap<String, u64> {
    let mut r = families
        .iter()
        .map(|(k, v)| (k.clone(), *v as u64))
        .collect::<BTreeMap<_, _>>();
    r.insert(
        "notes".into(),
        bundle
            .entries
            .iter()
            .filter(|e| e.note_kind.is_some())
            .count() as u64,
    );
    r.insert("artifacts".into(), bundle.entries.len() as u64);
    r
}
fn granularity_label(v: VaultGranularity) -> &'static str {
    match v {
        VaultGranularity::Compact => "compact",
        VaultGranularity::Full => "full",
    }
}
fn content_text(id: &ContentId) -> String {
    format!(
        "sha256:{}",
        id.bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}
pub(crate) fn refresh_bundle_digests(bundle: &mut VaultBundle) {
    for e in &mut bundle.entries {
        e.content_digest = content_id(b"sim.index-vault.content.v2\0", &e.bytes);
    }
    let mut h = Sha256::new();
    h.update(b"sim.index-vault.bundle.v2\0");
    for e in &bundle.entries {
        h.update(e.path.as_bytes());
        h.update([0]);
        h.update(e.content_digest.bytes);
    }
    bundle.bundle_root = finish(h);
}
fn content_id(domain: &[u8], bytes: &[u8]) -> ContentId {
    let mut h = Sha256::new();
    h.update(domain);
    h.update(bytes);
    finish(h)
}
fn finish(h: Sha256) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256"), h.finalize().into())
}
fn set_mode(slot: &mut Option<ExportMode>, value: ExportMode) -> Result<(), String> {
    if slot.is_some() {
        return Err("index export mode flags are mutually exclusive".into());
    }
    *slot = Some(value);
    Ok(())
}
fn set_once_path(
    slot: &mut Option<PathBuf>,
    args: &[String],
    i: &mut usize,
    flag: &str,
) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("duplicate index export argument `{flag}`"));
    }
    *i += 1;
    let v = args
        .get(*i)
        .ok_or_else(|| format!("{flag} requires a path"))?;
    if v.trim().is_empty() {
        return Err(format!("{flag} requires a non-empty path"));
    }
    *slot = Some(PathBuf::from(v));
    Ok(())
}
fn set_once_string(
    slot: &mut Option<String>,
    args: &[String],
    i: &mut usize,
    flag: &str,
) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("duplicate index export argument `{flag}`"));
    }
    *i += 1;
    let v = args
        .get(*i)
        .ok_or_else(|| format!("{flag} requires a value"))?;
    if v.trim().is_empty() {
        return Err(format!("{flag} requires a non-empty value"));
    }
    *slot = Some(v.clone());
    Ok(())
}
fn parse_granularity(v: &str) -> Result<VaultGranularity, String> {
    match v {
        "compact" => Ok(VaultGranularity::Compact),
        "full" => Ok(VaultGranularity::Full),
        other => Err(format!(
            "unknown Index vault granularity `{other}`; expected compact or full"
        )),
    }
}
fn usage(p: &str) -> String {
    format!(
        "usage: {p} index export --input <index.sx> --profile <profile> --vault-root <dir> [--namespace <relative-path>] [--granularity compact|full] [--plan|--verify|--check]"
    )
}

// Ownership guard: tooling may dispatch over public rows, but never enumerate IndexDoc fields.
const _: fn(&IndexRow) = |_| {};
