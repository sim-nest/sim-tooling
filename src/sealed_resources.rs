//! Materializes explicit immutable build-resource roots without ambient discovery.

use crate::build_inputs::{
    FileRecord, TreeLimit, TreeReport, TreeWriter, bounded_read, canonical_directory, digest,
    staging_path, sync_directory, write_new,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
};

const MAXIMUM_SPEC_BYTES: usize = 4 * 1024 * 1024;
const MAXIMUM_ROOTS: usize = 12;
const MAXIMUM_ENTRIES: usize = 4096;
const MAXIMUM_BYTES: usize = 256 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    schema: String,
    roots: Vec<RootSelection>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RootSelection {
    name: String,
    guest_path: String,
    maximum_entries: usize,
    maximum_bytes: usize,
    files: Vec<SelectedFile>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SelectedFile {
    source: PathBuf,
    destination: PathBuf,
    sha256: String,
    executable: bool,
}

#[derive(Serialize)]
struct Report {
    schema: &'static str,
    selection_sha256: String,
    roots: Vec<RootReport>,
}

#[derive(Serialize)]
struct RootReport {
    name: String,
    guest_path: String,
    resource_id: String,
    limit: TreeLimit,
    tree: TreeReport,
}

struct Options {
    selection: PathBuf,
    expected_selection_sha256: String,
    destination: PathBuf,
    owner_roots: Vec<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapturePlan {
    schema: String,
    roots: Vec<CaptureRoot>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureRoot {
    name: String,
    guest_path: String,
    maximum_entries: usize,
    maximum_bytes: usize,
    files: Vec<CaptureFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureFile {
    source: PathBuf,
    destination: PathBuf,
}

struct CaptureOptions {
    plan: PathBuf,
    expected_plan_sha256: String,
    selection: PathBuf,
    owner_roots: Vec<PathBuf>,
}

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    match args.get(2).map(String::as_str) {
        Some("capture") => {
            let options = parse_capture(&args)?;
            let selection = options.selection.clone();
            capture(options)?;
            println!(
                "sealed-resources: exact selection captured at {}",
                selection.display()
            );
            Ok(())
        }
        Some("materialize") => {
            let options = parse(&args)?;
            let destination = options.destination.clone();
            let report = materialize(options)?;
            println!(
                "sealed-resources: {} roots materialized at {}",
                report.roots.len(),
                destination.display()
            );
            Ok(())
        }
        _ => Err(usage(args.first().map(String::as_str).unwrap_or("xtask"))),
    }
}

fn parse_capture(args: &[String]) -> Result<CaptureOptions, String> {
    let (values, owner_roots) = parse_values(args)?;
    let required = |name: &'static str| {
        values
            .get(name)
            .cloned()
            .ok_or_else(|| format!("sealed-resources requires {name}"))
    };
    let expected_plan_sha256 = required("--expected-plan-sha256")?;
    validate_digest(&expected_plan_sha256)?;
    Ok(CaptureOptions {
        plan: PathBuf::from(required("--plan")?),
        expected_plan_sha256,
        selection: PathBuf::from(required("--selection")?),
        owner_roots,
    })
}

fn parse(args: &[String]) -> Result<Options, String> {
    let (values, owner_roots) = parse_values(args)?;
    let required = |name: &'static str| {
        values
            .get(name)
            .cloned()
            .ok_or_else(|| format!("sealed-resources requires {name}"))
    };
    let expected_selection_sha256 = required("--expected-selection-sha256")?;
    validate_digest(&expected_selection_sha256)?;
    Ok(Options {
        selection: PathBuf::from(required("--selection")?),
        expected_selection_sha256,
        destination: PathBuf::from(required("--destination")?),
        owner_roots,
    })
}

fn parse_values(args: &[String]) -> Result<(BTreeMap<&str, String>, Vec<PathBuf>), String> {
    let mut values = BTreeMap::new();
    let mut owner_roots = Vec::new();
    let mut index = 3;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} requires a value"))?;
        if flag == "--owner-root" {
            owner_roots.push(PathBuf::from(value));
        } else if values.insert(flag, value.clone()).is_some() {
            return Err(format!("sealed-resources repeats {flag}"));
        }
        index += 2;
    }
    if owner_roots.is_empty() {
        return Err("sealed-resources requires at least one --owner-root".into());
    }
    Ok((values, owner_roots))
}

fn capture(options: CaptureOptions) -> Result<(), String> {
    let owner_roots = options
        .owner_roots
        .iter()
        .map(|path| canonical_directory(path))
        .collect::<Result<Vec<_>, _>>()?;
    let plan_bytes = bounded_read(&options.plan)?;
    if plan_bytes.len() > MAXIMUM_SPEC_BYTES || digest(&plan_bytes) != options.expected_plan_sha256
    {
        return Err("sealed-resource capture plan digest or byte bound differs".into());
    }
    let plan: CapturePlan =
        serde_json::from_slice(&plan_bytes).map_err(|error| error.to_string())?;
    if plan.schema != "sim.sealed-resource-capture-plan/v1" {
        return Err("sealed-resource capture plan schema differs".into());
    }
    let selection = Selection {
        schema: "sim.sealed-resource-selection/v1".into(),
        roots: plan
            .roots
            .into_iter()
            .map(|root| capture_root(root, &owner_roots))
            .collect::<Result<Vec<_>, _>>()?,
    };
    validate_selection(&selection, &owner_roots)?;
    if options.selection.exists() {
        return Err("sealed-resource exact selection already exists".into());
    }
    let parent = options
        .selection
        .parent()
        .ok_or("sealed-resource selection has no parent")?;
    canonical_directory(parent)?;
    let bytes = serde_json::to_vec_pretty(&selection).map_err(|error| error.to_string())?;
    write_new(&options.selection, &bytes, false)?;
    sync_directory(parent)
}

fn capture_root(root: CaptureRoot, owner_roots: &[PathBuf]) -> Result<RootSelection, String> {
    if root.maximum_entries == 0
        || root.maximum_entries > MAXIMUM_ENTRIES
        || root.maximum_bytes == 0
        || root.maximum_bytes > MAXIMUM_BYTES
        || root.files.is_empty()
        || root.files.len() >= root.maximum_entries
    {
        return Err("sealed-resource capture root exceeds its hard limit".into());
    }
    let mut captured_bytes = 0usize;
    let files = root
        .files
        .into_iter()
        .map(|file| {
            let source = file
                .source
                .canonicalize()
                .map_err(|error| format!("{}: {error}", file.source.display()))?;
            if !source.is_file() || !owner_roots.iter().any(|owner| source.starts_with(owner)) {
                return Err("sealed-resource capture file has no selected owner".into());
            }
            let metadata = fs::metadata(&source).map_err(|error| error.to_string())?;
            let length = usize::try_from(metadata.len())
                .map_err(|_| "sealed-resource file length exceeds address space")?;
            if length == 0 {
                return Err("sealed-resource capture file is empty".into());
            }
            captured_bytes = captured_bytes
                .checked_add(length)
                .filter(|bytes| *bytes <= root.maximum_bytes)
                .ok_or("sealed-resource capture root exceeds its byte limit")?;
            Ok(SelectedFile {
                sha256: digest_file(&source, length)?,
                source,
                destination: file.destination,
                executable: metadata.permissions().mode() & 0o111 != 0,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(RootSelection {
        name: root.name,
        guest_path: root.guest_path,
        maximum_entries: root.maximum_entries,
        maximum_bytes: root.maximum_bytes,
        files,
    })
}

fn digest_file(path: &Path, expected_length: usize) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if !opened.is_file()
        || usize::try_from(opened.len()).ok() != Some(expected_length)
        || expected_length > MAXIMUM_BYTES
    {
        return Err("sealed-resource capture file changed or exceeds its bound".into());
    }
    let mut hash = Sha256::new();
    let mut read = 0usize;
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        read = read
            .checked_add(count)
            .filter(|bytes| *bytes <= expected_length)
            .ok_or("sealed-resource capture file grew while read")?;
        hash.update(&buffer[..count]);
    }
    if read != expected_length {
        return Err("sealed-resource capture file changed while read".into());
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn materialize(options: Options) -> Result<Report, String> {
    let owner_roots = options
        .owner_roots
        .iter()
        .map(|path| canonical_directory(path))
        .collect::<Result<Vec<_>, _>>()?;
    let selection_bytes = bounded_read(&options.selection)?;
    if selection_bytes.len() > MAXIMUM_SPEC_BYTES {
        return Err("sealed-resource selection exceeds its finite byte bound".into());
    }
    let selection_sha256 = digest(&selection_bytes);
    if selection_sha256 != options.expected_selection_sha256 {
        return Err("sealed-resource selection digest differs".into());
    }
    let selection: Selection =
        serde_json::from_slice(&selection_bytes).map_err(|error| error.to_string())?;
    validate_selection(&selection, &owner_roots)?;
    if options.destination.exists() {
        return Err("sealed-resources destination already exists".into());
    }
    let parent = options
        .destination
        .parent()
        .ok_or("sealed-resources destination has no parent")?;
    let parent = canonical_directory(parent)?;
    let staging = staging_path(&parent, &options.destination)?;
    fs::create_dir(&staging).map_err(|error| format!("{}: {error}", staging.display()))?;
    let result = materialize_into(&staging, selection, &owner_roots, selection_sha256);
    let report = result.map_err(|error| {
        format!(
            "{error}; incomplete sealed resources retained at {}",
            staging.display()
        )
    })?;
    fs::rename(&staging, &options.destination).map_err(|error| {
        format!(
            "publish {} as {}: {error}",
            staging.display(),
            options.destination.display()
        )
    })?;
    sync_directory(&parent)?;
    Ok(report)
}

fn validate_selection(selection: &Selection, owner_roots: &[PathBuf]) -> Result<(), String> {
    if selection.schema != "sim.sealed-resource-selection/v1"
        || selection.roots.is_empty()
        || selection.roots.len() > MAXIMUM_ROOTS
    {
        return Err("sealed-resource selection schema or root count differs".into());
    }
    let mut names = BTreeSet::new();
    let mut guests = BTreeSet::new();
    let mut sources = BTreeSet::new();
    for root in &selection.roots {
        validate_name(&root.name)?;
        validate_guest(&root.guest_path)?;
        if !names.insert(root.name.as_str()) || !guests.insert(root.guest_path.as_str()) {
            return Err("sealed-resource root name or guest path is duplicated".into());
        }
        if root.maximum_entries == 0
            || root.maximum_entries > MAXIMUM_ENTRIES
            || root.maximum_bytes == 0
            || root.maximum_bytes > MAXIMUM_BYTES
            || root.files.is_empty()
            || root.files.len() >= root.maximum_entries
        {
            return Err("sealed-resource root exceeds its hard limit".into());
        }
        let mut destinations = BTreeSet::new();
        for file in &root.files {
            validate_relative(&file.destination)?;
            validate_digest(&file.sha256)?;
            let source = file
                .source
                .canonicalize()
                .map_err(|error| format!("{}: {error}", file.source.display()))?;
            if !source.is_file()
                || !owner_roots.iter().any(|owner| source.starts_with(owner))
                || !sources.insert(source)
                || !destinations.insert(file.destination.as_path())
            {
                return Err("sealed-resource file is unowned, duplicated or non-regular".into());
            }
        }
    }
    Ok(())
}

fn materialize_into(
    staging: &Path,
    selection: Selection,
    owner_roots: &[PathBuf],
    selection_sha256: String,
) -> Result<Report, String> {
    let mut roots = Vec::with_capacity(selection.roots.len());
    for selected in selection.roots {
        let limit = TreeLimit {
            entries: selected.maximum_entries,
            bytes: selected.maximum_bytes,
        };
        let mut writer = TreeWriter::create(staging.join(&selected.name), limit)?;
        let expected = selected
            .files
            .iter()
            .map(|file| {
                Ok((
                    slash(&file.destination)?,
                    (file.sha256.clone(), file.executable),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        for file in &selected.files {
            writer.copy_tree(&file.source, &file.destination, owner_roots)?;
        }
        let tree = writer.finish()?;
        compare_files(&tree.files, &expected)?;
        let resource_id = format!("build-input/{}/sha256/{}", selected.name, tree.identity);
        roots.push(RootReport {
            name: selected.name,
            guest_path: selected.guest_path,
            resource_id,
            limit,
            tree,
        });
    }
    roots.sort_by(|left, right| left.guest_path.cmp(&right.guest_path));
    let report = Report {
        schema: "sim.sealed-resource-materialization/v1",
        selection_sha256,
        roots,
    };
    let bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    write_new(&staging.join("materialization.json"), &bytes, false)?;
    sync_directory(staging)?;
    Ok(report)
}

fn compare_files(
    actual: &[FileRecord],
    expected: &BTreeMap<String, (String, bool)>,
) -> Result<(), String> {
    if actual.len() != expected.len()
        || actual
            .iter()
            .any(|file| expected.get(&file.path) != Some(&(file.sha256.clone(), file.executable)))
    {
        return Err("sealed-resource source content or executable mode differs".into());
    }
    Ok(())
}

fn validate_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err("sealed-resource name is not canonical".into());
    }
    Ok(())
}

fn validate_guest(value: &str) -> Result<(), String> {
    let Some(name) = value.strip_prefix('/') else {
        return Err("sealed-resource guest path is not absolute".into());
    };
    validate_name(name)?;
    if matches!(
        value,
        "/source" | "/target" | "/work" | "/proc" | "/dev" | "/sys" | "/run"
    ) {
        return Err("sealed-resource guest path is reserved".into());
    }
    Ok(())
}

fn validate_relative(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.components().count() > 32
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("sealed-resource destination is not bounded relative".into());
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("sealed-resource digest is not lowercase SHA-256".into());
    }
    Ok(())
}

fn slash(path: &Path) -> Result<String, String> {
    path.to_str()
        .filter(|value| value.len() <= 4096 && !value.contains('\\'))
        .map(str::to_owned)
        .ok_or_else(|| "sealed-resource destination is not bounded UTF-8".into())
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} sealed-resources <capture --plan <plan.json> --expected-plan-sha256 <hex> --selection <new-selection.json>|materialize --selection <selection.json> --expected-selection-sha256 <hex> --destination <new-path>> --owner-root <path>..."
    )
}

#[cfg(test)]
mod tests;
