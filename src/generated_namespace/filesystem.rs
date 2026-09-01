use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum NamespaceSnapshot {
    Missing,
    EmptyDirectory,
    Owned {
        manifest_digest: String,
        files: BTreeMap<String, String>,
    },
}

pub(super) fn validate_vault_root(path: PathBuf) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("vault root must not be empty".to_owned());
    }
    Ok(path)
}

pub(super) fn validate_namespace(path: PathBuf) -> Result<PathBuf, String> {
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

pub(super) fn validate_sibling_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return Err(format!(
            "managed namespace sibling name is invalid: `{name}`"
        ));
    }
    Ok(())
}

pub(super) fn ensure_vault_root(root: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|err| format!("read vault root {}: {err}", root.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("vault root is not a directory: {}", root.display()));
    }
    Ok(())
}

pub(super) fn ensure_namespace_ancestors(root: &Path, namespace: &Path) -> Result<(), String> {
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

pub(super) fn reject_interrupted_path(kind: &str, path: &Path) -> Result<(), String> {
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

pub(super) fn metadata_if_exists(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("read {}: {err}", path.display())),
    }
}

pub(super) fn read_manifest_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
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

pub(super) fn collect_file_digests(root: &Path) -> Result<BTreeMap<String, String>, String> {
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

pub(super) fn slash_path(path: &Path) -> Result<String, String> {
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

pub(super) fn validate_owned_files(
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

pub(super) fn validate_owned_paths(
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

pub(super) fn write_stage(
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

pub(super) fn verify_written_namespace(
    root: &Path,
    expected: &VaultManifest,
) -> Result<(), String> {
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
