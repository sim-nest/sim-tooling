//! Materializes one exact Cargo unit graph into finite regular-file inputs.

use self::graph::{
    Package, ResolverPackage, local_patch_packages, resolver_local_packages,
    resolver_only_package_ids, selected_package_ids, selected_packages,
};
use self::limits::LimitsReport;
use self::manifest::{attach_resolver_package_to_workspace, narrow_workspace_manifest};
use serde::Serialize;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;

mod derive;
mod finalize;
mod graph;
mod limits;
mod manifest;
mod registry;
mod selected_view;

struct Options {
    metadata: PathBuf,
    unit_graph: PathBuf,
    workspace: PathBuf,
    vendor_root: PathBuf,
    lock: PathBuf,
    config: PathBuf,
    destination: PathBuf,
    owner_roots: Vec<PathBuf>,
    expected: BTreeMap<&'static str, String>,
    source_limit: TreeLimit,
    vendor_limit: TreeLimit,
    input_limit: TreeLimit,
}

#[derive(Clone, Copy, Serialize)]
pub(crate) struct TreeLimit {
    pub(crate) entries: usize,
    pub(crate) bytes: usize,
}

#[derive(Serialize)]
struct Report {
    schema: &'static str,
    inputs: BTreeMap<&'static str, String>,
    unit_graph_units: usize,
    local_packages: Vec<String>,
    resolver_only_local_packages: Vec<String>,
    registry_packages: Vec<String>,
    resolver_only_registry_packages: Vec<String>,
    limits: LimitsReport,
    source: TreeReport,
    vendor: TreeReport,
}

#[derive(Serialize)]
pub(crate) struct TreeReport {
    pub(crate) entries: usize,
    pub(crate) bytes: usize,
    pub(crate) identity: String,
    pub(crate) files: Vec<FileRecord>,
}

#[derive(Serialize)]
pub(crate) struct FileRecord {
    pub(crate) path: String,
    pub(crate) bytes: usize,
    pub(crate) sha256: String,
    pub(crate) executable: bool,
}

pub(crate) struct TreeWriter {
    root: PathBuf,
    limit: TreeLimit,
    entries: usize,
    bytes: usize,
    files: Vec<FileRecord>,
}

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    if args.get(2).map(String::as_str) == Some("select") {
        return selected_view::run(&args);
    }
    if args.get(2).map(String::as_str) == Some("derive") {
        return derive::run(&args);
    }
    if args.get(2).map(String::as_str) == Some("finalize") {
        return finalize::run(&args);
    }
    if args.get(2).map(String::as_str) != Some("materialize") {
        return Err(usage(args.first().map(String::as_str).unwrap_or("xtask")));
    }
    let options = parse(&args)?;
    let destination = options.destination.clone();
    let report = materialize(options)?;
    println!(
        "build-inputs: {} local + {} registry packages; source {}/{} bytes, vendor {}/{} bytes; {}",
        report.local_packages.len(),
        report.registry_packages.len(),
        report.source.entries,
        report.source.bytes,
        report.vendor.entries,
        report.vendor.bytes,
        destination.display()
    );
    Ok(())
}

fn parse(args: &[String]) -> Result<Options, String> {
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
            return Err(format!("build-inputs repeats {flag}"));
        }
        index += 2;
    }
    let required = |name: &'static str| {
        values
            .get(name)
            .cloned()
            .ok_or_else(|| format!("build-inputs requires {name}"))
    };
    if owner_roots.is_empty() {
        return Err("build-inputs requires at least one --owner-root".into());
    }
    let number = |name: &'static str, hard: usize| -> Result<usize, String> {
        let value = required(name)?
            .parse::<usize>()
            .map_err(|_| format!("{name} is not a canonical positive integer"))?;
        if value == 0 || value > hard {
            return Err(format!("{name} exceeds its hard bound"));
        }
        Ok(value)
    };
    let mut expected = BTreeMap::new();
    for (key, flag) in [
        ("metadata", "--expected-metadata-sha256"),
        ("unit-graph", "--expected-unit-graph-sha256"),
        ("workspace-manifest", "--expected-workspace-manifest-sha256"),
        ("lock", "--expected-lock-sha256"),
        ("config", "--expected-config-sha256"),
    ] {
        let digest = required(flag)?;
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{flag} is not a SHA-256 digest"));
        }
        expected.insert(key, digest.to_ascii_lowercase());
    }
    Ok(Options {
        metadata: PathBuf::from(required("--metadata")?),
        unit_graph: PathBuf::from(required("--unit-graph")?),
        workspace: PathBuf::from(required("--workspace")?),
        vendor_root: PathBuf::from(required("--vendor-root")?),
        lock: PathBuf::from(required("--lock")?),
        config: PathBuf::from(required("--config")?),
        destination: PathBuf::from(required("--destination")?),
        owner_roots,
        expected,
        source_limit: TreeLimit {
            entries: number("--max-source-entries", 4096)?,
            bytes: number("--max-source-bytes", 256 * 1024 * 1024)?,
        },
        vendor_limit: TreeLimit {
            entries: number("--max-vendor-entries", 4096)?,
            bytes: number("--max-vendor-bytes", 256 * 1024 * 1024)?,
        },
        input_limit: TreeLimit {
            entries: number("--max-input-entries", 8192)?,
            bytes: number("--max-input-bytes", 512 * 1024 * 1024)?,
        },
    })
}

fn materialize(options: Options) -> Result<Report, String> {
    let workspace = canonical_directory(&options.workspace)?;
    let vendor_root = canonical_directory(&options.vendor_root)?;
    let owner_roots = options
        .owner_roots
        .iter()
        .map(|path| canonical_directory(path))
        .collect::<Result<Vec<_>, _>>()?;
    let metadata_bytes = bounded_read(&options.metadata)?;
    let graph_bytes = bounded_read(&options.unit_graph)?;
    let manifest_path = workspace.join("Cargo.toml");
    let manifest_bytes = bounded_read(&manifest_path)?;
    let lock_bytes = bounded_read(&options.lock)?;
    let config_bytes = bounded_read(&options.config)?;
    let inputs = BTreeMap::from([
        ("metadata", digest(&metadata_bytes)),
        ("unit-graph", digest(&graph_bytes)),
        ("workspace-manifest", digest(&manifest_bytes)),
        ("lock", digest(&lock_bytes)),
        ("config", digest(&config_bytes)),
    ]);
    if inputs != options.expected {
        return Err("build-inputs exact input digests differ".into());
    }
    let graph: Json = serde_json::from_slice(&graph_bytes).map_err(|error| error.to_string())?;
    let metadata: Json =
        serde_json::from_slice(&metadata_bytes).map_err(|error| error.to_string())?;
    let selected = selected_package_ids(&graph)?;
    let packages = selected_packages(&metadata, &selected, &graph)?;
    let resolver_only = resolver_only_package_ids(&metadata, &selected)?;
    let resolver_packages = resolver_local_packages(&metadata, &resolver_only)?;
    let local = packages
        .iter()
        .filter(|package| !package.registry)
        .collect::<Vec<_>>();
    let registry = packages
        .iter()
        .filter(|package| package.registry)
        .collect::<Vec<_>>();
    let local_names = local
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut resolver_by_name = resolver_packages
        .into_iter()
        .filter(|package| !local_names.contains(package.name.as_str()))
        .map(|package| (package.name.clone(), package))
        .collect::<BTreeMap<_, _>>();
    for package in local_patch_packages(&workspace, &manifest_bytes)? {
        if !local_names.contains(package.name.as_str()) {
            resolver_by_name.insert(package.name.clone(), package);
        }
    }
    let resolver_packages = resolver_by_name.into_values().collect::<Vec<_>>();
    let resolver_registry_packages = registry::locked_resolver_packages(
        &lock_bytes,
        &vendor_root,
        &registry,
        &resolver_only,
        &resolver_packages,
    )?;
    if local.is_empty() || registry.is_empty() {
        return Err("build-inputs selected closure lacks local or registry packages".into());
    }
    if options.destination.exists() {
        return Err("build-inputs destination already exists".into());
    }
    let parent = options
        .destination
        .parent()
        .ok_or("build-inputs destination has no parent")?;
    let parent = canonical_directory(parent)?;
    let staging = staging_path(&parent, &options.destination)?;
    fs::create_dir(&staging).map_err(|error| format!("{}: {error}", staging.display()))?;
    let result = materialize_into(MaterializeInputs {
        staging: &staging,
        workspace: &workspace,
        vendor_root: &vendor_root,
        owner_roots: &owner_roots,
        manifest_bytes: &manifest_bytes,
        lock_bytes: &lock_bytes,
        config_bytes: &config_bytes,
        graph_units: graph
            .get("units")
            .and_then(Json::as_array)
            .ok_or("unit graph has no units")?
            .len(),
        packages: &packages,
        resolver_packages: &resolver_packages,
        resolver_registry_packages: &resolver_registry_packages,
        source_limit: options.source_limit,
        vendor_limit: options.vendor_limit,
        input_limit: options.input_limit,
        inputs,
    });
    let report = result.map_err(|error| {
        format!(
            "{error}; incomplete materialization retained at {}",
            staging.display()
        )
    })?;
    fs::rename(&staging, &options.destination).map_err(|error| {
        format!(
            "{} -> {}: {error}",
            staging.display(),
            options.destination.display()
        )
    })?;
    sync_directory(&parent)?;
    Ok(report)
}

struct MaterializeInputs<'a> {
    staging: &'a Path,
    workspace: &'a Path,
    vendor_root: &'a Path,
    owner_roots: &'a [PathBuf],
    manifest_bytes: &'a [u8],
    lock_bytes: &'a [u8],
    config_bytes: &'a [u8],
    graph_units: usize,
    packages: &'a [Package],
    resolver_packages: &'a [ResolverPackage],
    resolver_registry_packages: &'a [ResolverPackage],
    source_limit: TreeLimit,
    vendor_limit: TreeLimit,
    input_limit: TreeLimit,
    inputs: BTreeMap<&'static str, String>,
}

fn materialize_into(input: MaterializeInputs<'_>) -> Result<Report, String> {
    let source_root = input.staging.join("source");
    let vendor_root = input.staging.join("vendor");
    let mut source = TreeWriter::create(source_root, input.source_limit)?;
    let mut vendor = TreeWriter::create(vendor_root, input.vendor_limit)?;
    let local_names = input
        .packages
        .iter()
        .filter(|package| !package.registry)
        .map(|package| package.name.clone())
        .collect::<BTreeSet<_>>();
    let resolver_names = input
        .resolver_packages
        .iter()
        .map(|package| package.name.clone())
        .collect::<BTreeSet<_>>();
    let manifest_names = local_names.union(&resolver_names).cloned().collect();
    let root_manifest =
        narrow_workspace_manifest(input.manifest_bytes, &local_names, &manifest_names)?;
    source.write_file(Path::new("Cargo.toml"), &root_manifest, false)?;
    source.write_file(Path::new("Cargo.lock"), input.lock_bytes, false)?;
    source.write_file(Path::new(".cargo/config.toml"), input.config_bytes, false)?;
    let mut local_packages = Vec::new();
    let mut registry_packages = Vec::new();
    let mut resolver_only_local_packages = Vec::new();
    let mut resolver_only_registry_packages = Vec::new();
    let mut allowed = vec![input.workspace.to_owned()];
    allowed.extend_from_slice(input.owner_roots);
    for package in input.resolver_packages {
        let package_root = package
            .manifest
            .parent()
            .ok_or("resolver package manifest has no parent")?;
        let relative = package_root.strip_prefix(input.workspace).map_err(|_| {
            format!(
                "resolver package escaped selected workspace: {}",
                package.id
            )
        })?;
        let manifest = bounded_read(&package.manifest)?;
        let manifest = attach_resolver_package_to_workspace(&manifest)?;
        source.write_file(&relative.join("Cargo.toml"), &manifest, false)?;
        for target in &package.target_entries {
            let target_relative = target
                .strip_prefix(package_root)
                .map_err(|_| format!("resolver target escaped selected package: {}", package.id))?;
            source.copy_tree(target, &relative.join(target_relative), &allowed)?;
        }
        resolver_only_local_packages.push(package.id.clone());
    }
    for package in input.resolver_registry_packages {
        registry::copy_resolver(&mut vendor, package, input.vendor_root, input.lock_bytes)?;
        resolver_only_registry_packages.push(package.id.clone());
    }
    for package in input.packages {
        if package.registry {
            registry::copy_selected(&mut vendor, package, input.vendor_root, input.lock_bytes)?;
            registry_packages.push(package.id.clone());
        } else {
            let package_root = package
                .manifest
                .parent()
                .ok_or("local package manifest has no parent")?;
            let relative = package_root
                .strip_prefix(input.workspace)
                .map_err(|_| format!("local package escaped selected workspace: {}", package.id))?;
            source.copy_tree(&package.manifest, &relative.join("Cargo.toml"), &allowed)?;
            let mut copied_directories = BTreeSet::new();
            for target in &package.targets {
                let target_relative = target.source.strip_prefix(package_root).map_err(|_| {
                    format!("local target escaped selected package: {}", package.id)
                })?;
                if target.custom_build {
                    source.copy_tree(&target.source, &relative.join(target_relative), &allowed)?;
                    continue;
                }
                let directory = target_relative.parent().ok_or_else(|| {
                    format!("local target has no source directory: {}", package.id)
                })?;
                if copied_directories.insert(directory.to_owned()) {
                    source.copy_tree(
                        &package_root.join(directory),
                        &relative.join(directory),
                        &allowed,
                    )?;
                }
            }
            local_packages.push(package.id.clone());
        }
    }
    local_packages.sort();
    registry_packages.sort();
    let source = source.finish()?;
    let vendor = vendor.finish()?;
    if source.entries + vendor.entries > input.input_limit.entries
        || source.bytes + vendor.bytes > input.input_limit.bytes
    {
        return Err("materialized input trees exceed their combined finite limit".into());
    }
    let report = Report {
        schema: "sim.build-input-materialization/v1",
        inputs: input.inputs,
        unit_graph_units: input.graph_units,
        local_packages,
        resolver_only_local_packages,
        registry_packages,
        resolver_only_registry_packages,
        limits: LimitsReport {
            source: input.source_limit,
            vendor: input.vendor_limit,
            combined_input: input.input_limit,
        },
        source,
        vendor,
    };
    let bytes = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    write_new(&input.staging.join("materialization.json"), &bytes, false)?;
    sync_directory(input.staging)?;
    Ok(report)
}

impl TreeWriter {
    pub(crate) fn create(root: PathBuf, limit: TreeLimit) -> Result<Self, String> {
        fs::create_dir(&root).map_err(|error| format!("{}: {error}", root.display()))?;
        Ok(Self {
            root,
            limit,
            entries: 1,
            bytes: 0,
            files: Vec::new(),
        })
    }

    pub(crate) fn copy_tree(
        &mut self,
        source: &Path,
        destination: &Path,
        allowed: &[PathBuf],
    ) -> Result<(), String> {
        if let Some(parent) = destination.parent() {
            self.ensure_parents(parent)?;
        }
        let mut active = BTreeSet::new();
        self.copy_entry(source, destination, allowed, &mut active)
    }

    fn copy_entry(
        &mut self,
        source: &Path,
        destination: &Path,
        allowed: &[PathBuf],
        active: &mut BTreeSet<PathBuf>,
    ) -> Result<(), String> {
        validate_relative(destination)?;
        let metadata = fs::symlink_metadata(source)
            .map_err(|error| format!("{}: {error}", source.display()))?;
        let selected = if metadata.file_type().is_symlink() {
            let target = source
                .canonicalize()
                .map_err(|error| format!("{}: {error}", source.display()))?;
            if !allowed.iter().any(|root| target.starts_with(root)) {
                return Err(format!(
                    "source symlink escaped selected owners: {}",
                    source.display()
                ));
            }
            target
        } else {
            source.to_owned()
        };
        let metadata =
            fs::metadata(&selected).map_err(|error| format!("{}: {error}", selected.display()))?;
        if metadata.is_dir() {
            if !active.insert(selected.clone()) {
                return Err(format!("source directory cycle: {}", selected.display()));
            }
            self.create_directory(destination)?;
            let mut members = fs::read_dir(&selected)
                .map_err(|error| format!("{}: {error}", selected.display()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            members.sort_by_key(|entry| entry.file_name());
            for member in members {
                let name = member.file_name();
                if matches!(name.to_str(), Some(".git" | ".sim" | "target")) {
                    continue;
                }
                self.copy_entry(&member.path(), &destination.join(name), allowed, active)?;
            }
            active.remove(&selected);
            return Ok(());
        }
        if !metadata.is_file() {
            return Err(format!(
                "selected input is not regular: {}",
                selected.display()
            ));
        }
        let bytes = bounded_read_with_limit(&selected, self.limit.bytes)?;
        let executable = metadata.permissions().mode() & 0o111 != 0;
        self.write_file(destination, &bytes, executable)
    }

    fn create_directory(&mut self, relative: &Path) -> Result<(), String> {
        let path = self.root.join(relative);
        if path == self.root {
            return Ok(());
        }
        self.add_entry(0)?;
        fs::create_dir(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .map_err(|error| error.to_string())
    }

    fn write_file(
        &mut self,
        relative: &Path,
        bytes: &[u8],
        executable: bool,
    ) -> Result<(), String> {
        validate_relative(relative)?;
        if let Some(parent) = relative.parent() {
            self.ensure_parents(parent)?;
        }
        self.add_entry(bytes.len())?;
        let path = self.root.join(relative);
        write_new(&path, bytes, executable)?;
        self.files.push(FileRecord {
            path: slash(relative)?,
            bytes: bytes.len(),
            sha256: digest(bytes),
            executable,
        });
        Ok(())
    }

    fn ensure_parents(&mut self, relative: &Path) -> Result<(), String> {
        let mut path = PathBuf::new();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err("selected output parent is not relative".into());
            };
            path.push(name);
            let full = self.root.join(&path);
            if !full.exists() {
                self.create_directory(&path)?;
            }
        }
        Ok(())
    }

    fn add_entry(&mut self, bytes: usize) -> Result<(), String> {
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or("tree entry count overflow")?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or("tree byte count overflow")?;
        if self.entries > self.limit.entries || self.bytes > self.limit.bytes {
            return Err("materialized tree exceeds its exact finite limit".into());
        }
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<TreeReport, String> {
        self.files.sort_by(|left, right| left.path.cmp(&right.path));
        let identity = digest(&serde_json::to_vec(&self.files).map_err(|error| error.to_string())?);
        sync_tree(&self.root)?;
        Ok(TreeReport {
            entries: self.entries,
            bytes: self.bytes,
            identity,
            files: self.files,
        })
    }
}

pub(crate) fn write_new(path: &Path, bytes: &[u8], executable: bool) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if executable { 0o555 } else { 0o444 }),
    )
    .map_err(|error| error.to_string())
}

fn sync_tree(path: &Path) -> Result<(), String> {
    let mut directories = vec![path.to_owned()];
    let mut index = 0;
    while index < directories.len() {
        let directory = directories[index].clone();
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                directories.push(entry.path());
            }
        }
        index += 1;
    }
    for directory in directories.into_iter().rev() {
        sync_directory(&directory)?;
    }
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("{}: {error}", path.display()))
}

pub(crate) fn bounded_read(path: &Path) -> Result<Vec<u8>, String> {
    bounded_read_with_limit(path, MAX_INPUT_BYTES)
}

fn bounded_read_with_limit(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let length = usize::try_from(file.metadata().map_err(|error| error.to_string())?.len())
        .map_err(|_| "selected input length exceeds address space")?;
    if length == 0 || length > limit {
        return Err(format!(
            "selected input length is outside bounds: {}",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(length);
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() != length {
        return Err(format!(
            "selected input changed while read: {}",
            path.display()
        ));
    }
    Ok(bytes)
}

pub(crate) fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "selected path is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn validate_relative(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.components().count() > 32
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!(
            "selected output path is not bounded relative: {}",
            path.display()
        ));
    }
    Ok(())
}

fn slash(path: &Path) -> Result<String, String> {
    path.to_str()
        .filter(|value| value.len() <= 4096 && !value.contains('\\'))
        .map(str::to_owned)
        .ok_or_else(|| "selected output path is not bounded UTF-8".into())
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn staging_path(parent: &Path, destination: &Path) -> Result<PathBuf, String> {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("build-inputs destination has no UTF-8 name")?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    Ok(parent.join(format!(
        ".{name}.materializing-{}-{nonce}",
        std::process::id()
    )))
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} build-inputs <select --metadata <metadata.json> --unit-graph <unit-graph.json> --workspace <path> --lock <Cargo.lock> --config <config.toml> --vendor-root <path> --cargo-home <path> --cargo <path> --rustc <path> --destination <new-path> --owner-root <path>... --package <name> --bin <name> --target <triple> --profile development|release --expected-<input>-sha256 <hex> --expected-cargo-version <version> --expected-rustc-version <version> --expected-units <n> --expected-local-packages <n> --expected-registry-packages <n> --max-view-entries <n> --max-view-bytes <n>|derive --workspace <materialized-source> --vendor-root <materialized-vendor> --materialization <materialization.json> --expected-materialization-sha256 <hex> --cargo <path> --expected-cargo-sha256 <hex> --rustc <path> --expected-rustc-sha256 <hex> --package <name> --bin <name> --target <triple> --profile development|release --destination <new-path>|materialize --metadata <metadata.json> --unit-graph <unit-graph.json> --workspace <path> --vendor-root <path> --lock <Cargo.lock> --config <config.toml> --destination <new-path> --owner-root <path>... --expected-metadata-sha256 <hex> --expected-unit-graph-sha256 <hex> --expected-workspace-manifest-sha256 <hex> --expected-lock-sha256 <hex> --expected-config-sha256 <hex> --max-source-entries <n> --max-source-bytes <n> --max-vendor-entries <n> --max-vendor-bytes <n> --max-input-entries <n> --max-input-bytes <n>|finalize <materialize arguments> --assembled-materialization <record> --expected-assembled-materialization-sha256 <hex> --destination <new-record>>"
    )
}

#[cfg(test)]
mod tests;
