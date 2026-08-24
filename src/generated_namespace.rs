#![allow(dead_code)]

use sim_codec_index_vault::{
    LegacyVaultBundle, LegacyVaultEntry, VaultBundle, resolve_legacy_profile, verify_legacy_v1,
    verify_v2,
};
use sim_index_vault_core::VaultProjection;
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
};

use crate::{
    generated_artifact::{ArtifactSet, GeneratedArtifact},
    index_vault_manifest::{
        LegacyVaultManifest, MANIFEST_FILE, VaultManifest, VaultManifestSeed, sha256_digest,
    },
};

#[derive(Clone, Debug)]
pub(crate) struct ManagedNamespace {
    vault_root: PathBuf,
    namespace: PathBuf,
    namespace_text: String,
    target: PathBuf,
    stage: PathBuf,
    recovery: PathBuf,
}

impl ManagedNamespace {
    pub(crate) fn migrate_v1(
        &self,
        legacy_profile: &str,
        seed: &VaultManifestSeed,
        artifacts: &ArtifactSet,
        legacy_projection: &VaultProjection,
        expected_bundle: &VaultBundle,
        expected_projection: &VaultProjection,
    ) -> Result<NamespaceDiff, String> {
        self.migrate_v1_inner(
            legacy_profile,
            seed,
            artifacts,
            legacy_projection,
            expected_bundle,
            expected_projection,
            MigrationFault::None,
        )
    }

    #[cfg(test)]
    pub(crate) fn migrate_v1_with_fault(
        &self,
        legacy_profile: &str,
        seed: &VaultManifestSeed,
        artifacts: &ArtifactSet,
        legacy_projection: &VaultProjection,
        expected_bundle: &VaultBundle,
        expected_projection: &VaultProjection,
        fault: MigrationFault,
    ) -> Result<NamespaceDiff, String> {
        self.migrate_v1_inner(
            legacy_profile,
            seed,
            artifacts,
            legacy_projection,
            expected_bundle,
            expected_projection,
            fault,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn migrate_v1_inner(
        &self,
        legacy_profile: &str,
        seed: &VaultManifestSeed,
        artifacts: &ArtifactSet,
        legacy_projection: &VaultProjection,
        expected_bundle: &VaultBundle,
        expected_projection: &VaultProjection,
        fault: MigrationFault,
    ) -> Result<NamespaceDiff, String> {
        ensure_vault_root(&self.vault_root)?;
        ensure_namespace_ancestors(&self.vault_root, &self.namespace)?;
        reject_interrupted_path("stage", &self.stage)?;
        reject_interrupted_path("recovery", &self.recovery)?;
        migration_fail(fault, MigrationFault::BeforeManifestRead)?;
        let manifest_path = self.target.join(MANIFEST_FILE);
        let bytes = read_manifest_bytes(&manifest_path)?.ok_or_else(|| {
            format!(
                "v1 migration requires ownership manifest at {}",
                manifest_path.display()
            )
        })?;
        migration_fail(fault, MigrationFault::AfterManifestRead)?;
        if VaultManifest::from_bytes(&bytes).is_ok() {
            let expected = self.expected_manifest(seed, artifacts);
            let current = self.inspect_current(&expected)?;
            let CurrentNamespace::Owned { manifest, .. } = current.current else {
                unreachable!()
            };
            if manifest == expected {
                return Ok(NamespaceDiff {
                    namespace: self.namespace_text.clone(),
                    changed_artifacts: 0,
                    unchanged_artifacts: artifacts.iter().count(),
                });
            }
            return Err("managed namespace is already v2 with a different target; migration is not a rewrite flag".into());
        }
        let legacy = LegacyVaultManifest::from_bytes(&bytes)?;
        if legacy.namespace != self.namespace_text {
            return Err(format!(
                "v1 manifest namespace is `{}`, expected `{}`",
                legacy.namespace, self.namespace_text
            ));
        }
        if legacy.profile != legacy_profile {
            return Err(format!(
                "v1 manifest profile is `{}`, requested `{legacy_profile}`",
                legacy.profile
            ));
        }
        if legacy.index_digest != seed.index_digest {
            return Err(format!(
                "v1 source graph digest is `{}`, selected source graph is `{}`",
                legacy.index_digest, seed.index_digest
            ));
        }
        let files = collect_file_digests(&self.target)?;
        validate_owned_files_v1(&legacy, &files)?;
        migration_fail(fault, MigrationFault::AfterSourceValidation)?;
        let profile = resolve_legacy_profile(legacy_profile)
            .map_err(|error| format!("resolve decode-only v1 profile: {error}"))?;
        let granularity = match legacy.granularity.as_str() {
            "compact" => sim_index_vault_core::VaultGranularity::Compact,
            "full" => sim_index_vault_core::VaultGranularity::Full,
            other => return Err(format!("unsupported v1 granularity `{other}`")),
        };
        if granularity != expected_projection.granularity() {
            return Err(format!(
                "v1 granularity `{}` disagrees with selected v2 target `{}`",
                legacy.granularity, seed.granularity
            ));
        }
        let legacy_bundle = LegacyVaultBundle {
            profile,
            granularity,
            entries: legacy
                .artifacts
                .keys()
                .map(|path| {
                    fs::read(self.target.join(path))
                        .map(|bytes| LegacyVaultEntry {
                            path: path.clone(),
                            bytes,
                        })
                        .map_err(|error| format!("read v1 managed artifact `{path}`: {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        verify_legacy_v1(&legacy_bundle, legacy_projection)
            .map_err(|error| format!("v1 semantic verification failed: {error}"))?;
        migration_fail(fault, MigrationFault::AfterLegacyVerify)?;
        let snapshot_manifest = sha256_digest(&bytes);
        let expected = self.expected_manifest(seed, artifacts);
        migration_fail(fault, MigrationFault::BeforeStageWrite)?;
        write_stage(&self.stage, artifacts, &expected)?;
        migration_fail(fault, MigrationFault::AfterStageWrite)?;
        let staged_bundle = bundle_from_root(&self.stage, expected_bundle)?;
        let staged = verify_v2(&staged_bundle, expected_projection, 64, 1024)
            .map_err(|error| format!("verify staged v2 semantics: {error}"))?;
        if !staged.is_success() {
            let _ = fs::remove_dir_all(&self.stage);
            return Err(format!(
                "staged v2 semantic verification found {} mismatch(es)",
                staged.total_mismatches
            ));
        }
        migration_fail(fault, MigrationFault::AfterStageVerify)?;
        // Re-read every source byte after staging. A concurrent edit invalidates the transaction.
        let current_bytes = read_manifest_bytes(&manifest_path)?
            .ok_or("v1 manifest disappeared during migration")?;
        let current_files = collect_file_digests(&self.target)?;
        if sha256_digest(&current_bytes) != snapshot_manifest || current_files != files {
            let _ = fs::remove_dir_all(&self.stage);
            return Err(
                "managed v1 namespace changed during migration; source bytes were preserved".into(),
            );
        }
        migration_fail(fault, MigrationFault::BeforeRecoveryRename)?;
        fs::rename(&self.target, &self.recovery).map_err(|e| {
            format!(
                "move v1 namespace to recovery {}: {e}",
                self.recovery.display()
            )
        })?;
        migration_fail(fault, MigrationFault::AfterRecoveryRename)?;
        migration_fail(fault, MigrationFault::BeforeLiveRename)?;
        if let Err(err) = fs::rename(&self.stage, &self.target) {
            let _ = fs::rename(&self.recovery, &self.target);
            return Err(format!("install staged v2 namespace: {err}"));
        }
        migration_fail(fault, MigrationFault::AfterLiveRename)?;
        migration_fail(fault, MigrationFault::BeforeManifestReadback)?;
        if let Err(err) = verify_written_namespace(&self.target, &expected) {
            let _ = fs::rename(&self.target, &self.stage);
            let _ = fs::rename(&self.recovery, &self.target);
            return Err(format!("verify installed v2 namespace: {err}"));
        }
        migration_fail(fault, MigrationFault::AfterManifestReadback)?;
        migration_fail(fault, MigrationFault::BeforeRecoveryCleanup)?;
        fs::remove_dir_all(&self.recovery).map_err(|e| {
            format!(
                "v2 installed but recovery cleanup requires attention at {}: {e}",
                self.recovery.display()
            )
        })?;
        migration_fail(fault, MigrationFault::AfterRecoveryCleanup)?;
        Ok(NamespaceDiff {
            namespace: self.namespace_text.clone(),
            changed_artifacts: artifacts.iter().count(),
            unchanged_artifacts: 0,
        })
    }

    /// Reads a caller-described bundle through the ownership-validated namespace.
    /// This is deliberately not a second namespace transaction.
    pub(crate) fn current_bundle(
        &self,
        seed: &VaultManifestSeed,
        expected_bundle: &VaultBundle,
    ) -> Result<VaultBundle, String> {
        let expected_artifacts = ArtifactSet::new(
            expected_bundle
                .entries
                .iter()
                .map(|entry| GeneratedArtifact::new(&entry.path, entry.bytes.clone()))
                .collect::<Result<Vec<_>, _>>()?,
        )?;
        let expected = self.expected_manifest(seed, &expected_artifacts);
        let inspection = self.inspect_current_inner(&expected, false, false)?;
        if !matches!(inspection.current, CurrentNamespace::Owned { .. }) {
            return Err(format!(
                "managed namespace `{}` is missing generated artifacts",
                self.namespace_text
            ));
        }
        let mut current = expected_bundle.clone();
        for entry in &mut current.entries {
            entry.bytes = fs::read(self.target.join(&entry.path))
                .map_err(|err| format!("read managed vault artifact {}: {err}", entry.path))?;
        }
        crate::index_vault::refresh_bundle_digests(&mut current);
        Ok(current)
    }

    pub(crate) fn open(
        vault_root: impl Into<PathBuf>,
        namespace: impl Into<PathBuf>,
    ) -> Result<Self, String> {
        let vault_root = validate_vault_root(vault_root.into())?;
        let namespace = validate_namespace(namespace.into())?;
        let namespace_text = slash_path(&namespace)?;
        let target = vault_root.join(&namespace);
        let parent = target
            .parent()
            .ok_or("managed namespace target must have a parent")?
            .to_path_buf();
        let leaf = target
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or("managed namespace target must end in UTF-8")?;
        let stage_name = format!(".{leaf}.sim-stage");
        let recovery_name = format!(".{leaf}.sim-recovery");
        validate_sibling_name(&stage_name)?;
        validate_sibling_name(&recovery_name)?;
        Ok(Self {
            vault_root,
            namespace,
            namespace_text,
            target,
            stage: parent.join(stage_name),
            recovery: parent.join(recovery_name),
        })
    }

    pub(crate) fn plan(&self, seed: &VaultManifestSeed, artifacts: &ArtifactSet) -> NamespacePlan {
        let _ = self.expected_manifest(seed, artifacts);
        let bytes = artifacts.iter().map(|artifact| artifact.bytes.len()).sum();
        NamespacePlan {
            namespace: self.namespace_text.clone(),
            target: self.target.clone(),
            artifact_count: artifacts.iter().count(),
            byte_count: bytes,
            manifest_path: self.target.join(MANIFEST_FILE),
        }
    }

    pub(crate) fn diff(
        &self,
        seed: &VaultManifestSeed,
        artifacts: &ArtifactSet,
    ) -> Result<NamespaceDiff, String> {
        let expected = self.expected_manifest(seed, artifacts);
        let snapshot = self.inspect_current(&expected)?;
        let CurrentNamespace::Owned { files, .. } = snapshot.current else {
            return Ok(NamespaceDiff {
                namespace: self.namespace_text.clone(),
                changed_artifacts: artifacts.iter().count(),
                unchanged_artifacts: 0,
            });
        };
        let changed_expected = expected
            .artifacts
            .iter()
            .filter(|(path, digest)| files.get(*path) != Some(*digest))
            .count();
        let removed_previous = files
            .keys()
            .filter(|path| !expected.artifacts.contains_key(*path))
            .count();
        Ok(NamespaceDiff {
            namespace: self.namespace_text.clone(),
            changed_artifacts: changed_expected + removed_previous,
            unchanged_artifacts: expected.artifacts.len() - changed_expected,
        })
    }

    pub(crate) fn check(
        &self,
        seed: &VaultManifestSeed,
        artifacts: &ArtifactSet,
    ) -> Result<NamespaceCheck, String> {
        let expected = self.expected_manifest(seed, artifacts);
        let snapshot = self.inspect_current(&expected)?;
        let CurrentNamespace::Owned {
            manifest, files, ..
        } = snapshot.current
        else {
            return Err(format!(
                "managed namespace `{}` is missing generated artifacts",
                self.namespace_text
            ));
        };
        let mut stale = Vec::new();
        for (path, digest) in &expected.artifacts {
            match files.get(path) {
                Some(current) if current == digest => {}
                Some(_) => stale.push(path.clone()),
                None => stale.push(path.clone()),
            }
        }
        for path in files.keys() {
            if !expected.artifacts.contains_key(path) {
                stale.push(path.clone());
            }
        }
        if manifest != expected || !stale.is_empty() {
            stale.sort();
            stale.dedup();
            let detail = if stale.is_empty() {
                "manifest".to_owned()
            } else {
                stale.join(", ")
            };
            return Err(format!(
                "stale managed namespace `{}`: {detail}",
                self.namespace_text
            ));
        }
        Ok(NamespaceCheck {
            namespace: self.namespace_text.clone(),
            artifact_count: files.len(),
        })
    }

    pub(crate) fn preflight(
        &self,
        seed: &VaultManifestSeed,
        artifacts: &ArtifactSet,
    ) -> Result<PendingNamespaceTransaction, String> {
        let expected = self.expected_manifest(seed, artifacts);
        let snapshot = self.inspect_current(&expected)?;
        Ok(PendingNamespaceTransaction {
            namespace: self.clone(),
            expected,
            artifacts: artifacts.clone(),
            snapshot: snapshot.current.snapshot(),
        })
    }

    fn expected_manifest(
        &self,
        seed: &VaultManifestSeed,
        artifacts: &ArtifactSet,
    ) -> VaultManifest {
        VaultManifest::for_artifacts(&self.namespace_text, seed, artifacts)
    }

    fn inspect_current(&self, expected: &VaultManifest) -> Result<CurrentInspection, String> {
        self.inspect_current_inner(expected, false, true)
    }

    fn inspect_current_inner(
        &self,
        expected: &VaultManifest,
        allow_stage: bool,
        validate_bytes: bool,
    ) -> Result<CurrentInspection, String> {
        ensure_vault_root(&self.vault_root)?;
        ensure_namespace_ancestors(&self.vault_root, &self.namespace)?;
        if !allow_stage {
            reject_interrupted_path("stage", &self.stage)?;
        }
        reject_interrupted_path("recovery", &self.recovery)?;
        let Some(metadata) = metadata_if_exists(&self.target)? else {
            return Ok(CurrentInspection {
                current: CurrentNamespace::Missing,
            });
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "managed namespace target is not a directory: {}",
                self.target.display()
            ));
        }

        let manifest_path = self.target.join(MANIFEST_FILE);
        let manifest_bytes = read_manifest_bytes(&manifest_path)?;
        let files = collect_file_digests(&self.target)?;
        let Some(manifest_bytes) = manifest_bytes else {
            if files.is_empty() {
                return Ok(CurrentInspection {
                    current: CurrentNamespace::EmptyDirectory,
                });
            }
            return Err(format!(
                "non-empty namespace `{}` has no ownership manifest",
                self.namespace_text
            ));
        };
        let manifest = VaultManifest::from_bytes(&manifest_bytes)?;
        manifest.validate_owner(expected)?;
        if validate_bytes {
            validate_owned_files(&manifest, &files)?;
        } else {
            validate_owned_paths(&manifest, &files)?;
        }
        Ok(CurrentInspection {
            current: CurrentNamespace::Owned {
                manifest_digest: sha256_digest(&manifest_bytes),
                manifest,
                files,
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MigrationFault {
    None,
    BeforeManifestRead,
    AfterManifestRead,
    AfterSourceValidation,
    AfterLegacyVerify,
    BeforeStageWrite,
    AfterStageWrite,
    AfterStageVerify,
    BeforeRecoveryRename,
    AfterRecoveryRename,
    BeforeLiveRename,
    AfterLiveRename,
    BeforeManifestReadback,
    AfterManifestReadback,
    BeforeRecoveryCleanup,
    AfterRecoveryCleanup,
}

fn migration_fail(actual: MigrationFault, point: MigrationFault) -> Result<(), String> {
    if actual == point {
        return Err(format!("injected migration failpoint: {point:?}"));
    }
    Ok(())
}

fn bundle_from_root(root: &Path, expected: &VaultBundle) -> Result<VaultBundle, String> {
    let mut bundle = expected.clone();
    for entry in &mut bundle.entries {
        entry.bytes = fs::read(root.join(&entry.path))
            .map_err(|error| format!("read staged vault artifact `{}`: {error}", entry.path))?;
    }
    crate::index_vault::refresh_bundle_digests(&mut bundle);
    Ok(bundle)
}

fn validate_owned_files_v1(
    manifest: &LegacyVaultManifest,
    files: &BTreeMap<String, String>,
) -> Result<(), String> {
    for (path, digest) in &manifest.artifacts {
        match files.get(path) {
            Some(actual) if actual == digest => {}
            Some(_) => return Err(format!("v1 managed artifact digest mismatch: `{path}`")),
            None => return Err(format!("v1 managed artifact is missing: `{path}`")),
        }
    }
    if let Some(path) = files
        .keys()
        .find(|path| !manifest.artifacts.contains_key(*path))
    {
        return Err(format!("v1 namespace contains unowned file `{path}`"));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NamespacePlan {
    pub(crate) namespace: String,
    pub(crate) target: PathBuf,
    pub(crate) manifest_path: PathBuf,
    pub(crate) artifact_count: usize,
    pub(crate) byte_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NamespaceDiff {
    pub(crate) namespace: String,
    pub(crate) changed_artifacts: usize,
    pub(crate) unchanged_artifacts: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NamespaceCheck {
    pub(crate) namespace: String,
    pub(crate) artifact_count: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingNamespaceTransaction {
    namespace: ManagedNamespace,
    expected: VaultManifest,
    artifacts: ArtifactSet,
    snapshot: NamespaceSnapshot,
}

impl PendingNamespaceTransaction {
    pub(crate) fn commit(self) -> Result<(), String> {
        self.commit_inner(CommitFault::None)
    }

    #[cfg(test)]
    pub(crate) fn commit_with_injected_recovery_failure(self) -> Result<(), String> {
        self.commit_inner(CommitFault::AfterRecoveryRename)
    }

    fn commit_inner(self, fault: CommitFault) -> Result<(), String> {
        self.namespace
            .verify_snapshot(&self.expected, &self.snapshot, false)?;
        reject_interrupted_path("stage", &self.namespace.stage)?;
        reject_interrupted_path("recovery", &self.namespace.recovery)?;
        let parent = self
            .namespace
            .target
            .parent()
            .ok_or("managed namespace target must have a parent")?;
        fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;
        write_stage(&self.namespace.stage, &self.artifacts, &self.expected)?;
        self.namespace
            .verify_snapshot(&self.expected, &self.snapshot, true)?;

        let had_target = self.namespace.target.exists();
        if had_target {
            fs::rename(&self.namespace.target, &self.namespace.recovery).map_err(|err| {
                format!(
                    "move managed namespace to recovery {}: {err}",
                    self.namespace.recovery.display()
                )
            })?;
        }
        if fault == CommitFault::AfterRecoveryRename {
            return Err(format!(
                "injected rename failure after recovery move; stage remains at {} and recovery remains at {}",
                self.namespace.stage.display(),
                self.namespace.recovery.display()
            ));
        }
        fs::rename(&self.namespace.stage, &self.namespace.target).map_err(|err| {
            format!(
                "move staged namespace into place {}: {err}",
                self.namespace.target.display()
            )
        })?;
        verify_written_namespace(&self.namespace.target, &self.expected)?;
        if had_target {
            fs::remove_dir_all(&self.namespace.recovery).map_err(|err| {
                format!(
                    "remove recovery {}: {err}",
                    self.namespace.recovery.display()
                )
            })?;
        }
        Ok(())
    }
}

impl ManagedNamespace {
    fn verify_snapshot(
        &self,
        expected: &VaultManifest,
        snapshot: &NamespaceSnapshot,
        allow_stage: bool,
    ) -> Result<(), String> {
        let current = self
            .inspect_current_inner(expected, allow_stage, true)?
            .current
            .snapshot();
        if &current != snapshot {
            return Err(format!(
                "managed namespace `{}` changed after preflight; rerun the export",
                self.namespace_text
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommitFault {
    None,
    AfterRecoveryRename,
}

#[derive(Clone, Debug)]
struct CurrentInspection {
    current: CurrentNamespace,
}

#[derive(Clone, Debug)]
enum CurrentNamespace {
    Missing,
    EmptyDirectory,
    Owned {
        manifest_digest: String,
        manifest: VaultManifest,
        files: BTreeMap<String, String>,
    },
}

impl CurrentNamespace {
    fn snapshot(&self) -> NamespaceSnapshot {
        match self {
            Self::Missing => NamespaceSnapshot::Missing,
            Self::EmptyDirectory => NamespaceSnapshot::EmptyDirectory,
            Self::Owned {
                manifest_digest,
                files,
                ..
            } => NamespaceSnapshot::Owned {
                manifest_digest: manifest_digest.clone(),
                files: files.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum NamespaceSnapshot {
    Missing,
    EmptyDirectory,
    Owned {
        manifest_digest: String,
        files: BTreeMap<String, String>,
    },
}

fn validate_vault_root(path: PathBuf) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("vault root must not be empty".to_owned());
    }
    Ok(path)
}

fn validate_namespace(path: PathBuf) -> Result<PathBuf, String> {
    let text = path
        .to_str()
        .ok_or("managed namespace must be valid UTF-8")?;
    if text.is_empty() {
        return Err("managed namespace must not be empty".to_owned());
    }
    if path.is_absolute()
        || looks_like_windows_absolute(text)
        || text.contains('\\')
        || text.split('/').any(|part| part.is_empty())
    {
        return Err(format!(
            "managed namespace must be a normalized relative path: `{text}`"
        ));
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "managed namespace must not traverse outside the vault root: `{text}`"
        ));
    }
    Ok(path)
}

fn validate_sibling_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return Err(format!(
            "managed namespace sibling name is invalid: `{name}`"
        ));
    }
    Ok(())
}

fn ensure_vault_root(root: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|err| format!("read vault root {}: {err}", root.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("vault root is not a directory: {}", root.display()));
    }
    Ok(())
}

fn ensure_namespace_ancestors(root: &Path, namespace: &Path) -> Result<(), String> {
    let mut current = root.to_path_buf();
    let mut components = namespace.components().peekable();
    while let Some(component) = components.next() {
        if components.peek().is_none() {
            break;
        }
        current.push(component.as_os_str());
        let Some(metadata) = metadata_if_exists(&current)? else {
            break;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "managed namespace ancestor is not a directory: {}",
                current.display()
            ));
        }
    }
    Ok(())
}

fn reject_interrupted_path(kind: &str, path: &Path) -> Result<(), String> {
    if metadata_if_exists(path)?.is_some() {
        return Err(format!(
            "interrupted managed namespace {kind} exists at {}; inspect it before retrying",
            path.display()
        ));
    }
    Ok(())
}

fn looks_like_windows_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn metadata_if_exists(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("read {}: {err}", path.display())),
    }
}

fn read_manifest_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let Some(metadata) = metadata_if_exists(path)? else {
        return Ok(None);
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "ownership manifest is not a file: {}",
            path.display()
        ));
    }
    fs::read(path)
        .map(Some)
        .map_err(|err| format!("read manifest {}: {err}", path.display()))
}

fn collect_file_digests(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut files = BTreeMap::new();
    let mut folded = BTreeMap::<String, String>::new();
    collect_file_digests_inner(root, root, &mut files, &mut folded)?;
    Ok(files)
}

fn collect_file_digests_inner(
    root: &Path,
    dir: &Path,
    files: &mut BTreeMap<String, String>,
    folded: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|err| format!("read {}: {err}", dir.display()))? {
        let entry = entry.map_err(|err| format!("read {}: {err}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("read {}: {err}", path.display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "managed namespace contains a symlink escape candidate: {}",
                path.display()
            ));
        }
        if file_type.is_dir() {
            collect_file_digests_inner(root, &path, files, folded)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(format!(
                "managed namespace contains a non-file entry: {}",
                path.display()
            ));
        }
        let key = relative_key(root, &path)?;
        if key == MANIFEST_FILE {
            continue;
        }
        let folded_key = key.to_lowercase();
        if let Some(previous) = folded.insert(folded_key, key.clone()) {
            return Err(format!(
                "case-fold collision between managed files `{previous}` and `{key}`"
            ));
        }
        let bytes = fs::read(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        files.insert(key, sha256_digest(&bytes));
    }
    Ok(())
}

fn relative_key(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("path escaped managed namespace: {}", path.display()))?;
    slash_path(relative)
}

fn slash_path(path: &Path) -> Result<String, String> {
    path.components()
        .map(|component| match component {
            Component::Normal(part) => part
                .to_str()
                .map(str::to_owned)
                .ok_or("path component must be valid UTF-8".to_owned()),
            _ => Err("path must be normalized".to_owned()),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}

fn validate_owned_files(
    manifest: &VaultManifest,
    files: &BTreeMap<String, String>,
) -> Result<(), String> {
    for (path, digest) in &manifest.artifacts {
        let Some(current) = files.get(path) else {
            return Err(format!("managed file `{path}` is missing"));
        };
        if current != digest {
            return Err(format!(
                "managed file `{path}` was changed outside the exporter"
            ));
        }
    }
    for path in files.keys() {
        if !manifest.artifacts.contains_key(path) {
            return Err(format!(
                "foreign file `{path}` is inside the managed namespace"
            ));
        }
    }
    Ok(())
}

fn validate_owned_paths(
    manifest: &VaultManifest,
    files: &BTreeMap<String, String>,
) -> Result<(), String> {
    for path in manifest.artifacts.keys() {
        if !files.contains_key(path) {
            return Err(format!("managed file `{path}` is missing"));
        }
    }
    for path in files.keys() {
        if !manifest.artifacts.contains_key(path) {
            return Err(format!(
                "foreign file `{path}` is inside the managed namespace"
            ));
        }
    }
    Ok(())
}

fn write_stage(
    stage: &Path,
    artifacts: &ArtifactSet,
    manifest: &VaultManifest,
) -> Result<(), String> {
    fs::create_dir_all(stage).map_err(|err| format!("create stage {}: {err}", stage.display()))?;
    for artifact in artifacts.iter() {
        write_artifact(stage, artifact)?;
    }
    fs::write(stage.join(MANIFEST_FILE), manifest.to_bytes()?)
        .map_err(|err| format!("write manifest {}: {err}", stage.display()))?;
    verify_written_namespace(stage, manifest)
}

fn write_artifact(root: &Path, artifact: &GeneratedArtifact) -> Result<(), String> {
    let path = root.join(&artifact.path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;
    }
    fs::write(&path, &artifact.bytes).map_err(|err| format!("write {}: {err}", path.display()))
}

fn verify_written_namespace(root: &Path, expected: &VaultManifest) -> Result<(), String> {
    let manifest_path = root.join(MANIFEST_FILE);
    let Some(bytes) = read_manifest_bytes(&manifest_path)? else {
        return Err(format!(
            "managed namespace manifest was not written: {}",
            manifest_path.display()
        ));
    };
    let manifest = VaultManifest::from_bytes(&bytes)?;
    if &manifest != expected {
        return Err(
            "written managed namespace manifest does not match expected manifest".to_owned(),
        );
    }
    let files = collect_file_digests(root)?;
    validate_owned_files(&manifest, &files)?;
    Ok(())
}
