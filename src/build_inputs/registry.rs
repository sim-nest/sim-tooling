use super::{Package, ResolverPackage, TreeWriter, bounded_read, digest};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub(super) fn locked_resolver_packages(
    lock: &[u8],
    vendor_root: &Path,
    selected: &[&Package],
    resolver: &BTreeSet<String>,
    local_resolver: &[ResolverPackage],
) -> Result<Vec<ResolverPackage>, String> {
    let selected = selected
        .iter()
        .map(|package| (package.name.as_str(), package.version.as_str()))
        .collect::<BTreeSet<_>>();
    let root: toml::Value = toml::from_str(
        std::str::from_utf8(lock).map_err(|error| format!("Cargo.lock is not UTF-8: {error}"))?,
    )
    .map_err(|error| format!("Cargo.lock is invalid: {error}"))?;
    let rows = root
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or("Cargo.lock has no packages")?;
    let unused_patches = root
        .get("patch")
        .and_then(|patch| patch.get("unused"))
        .and_then(toml::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut retained = resolver.clone();
    let mut pending = Vec::new();
    for package in local_resolver {
        let version = package
            .id
            .rsplit_once('#')
            .map(|(_, version)| version)
            .ok_or_else(|| format!("resolver package has no version: {}", package.id))?;
        let matches = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                row.get("source").is_none()
                    && row.get("name").and_then(toml::Value::as_str) == Some(package.name.as_str())
                    && row.get("version").and_then(toml::Value::as_str) == Some(version)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [index] => pending.push(*index),
            [] if uniquely_unused_patch(unused_patches, &package.name, version) => {
                // Cargo records a selected-view patch here when it was
                // available to the resolver but not reached by the exact
                // production graph. Preserve its source for deterministic
                // resolver replay, but it has no locked dependency closure.
            }
            _ => {
                return Err(format!(
                    "Cargo.lock does not identify local resolver package {} exactly once",
                    package.id
                ));
            }
        }
    }
    let mut visited = BTreeSet::new();
    while let Some(index) = pending.pop() {
        if !visited.insert(index) {
            continue;
        }
        for dependency in rows[index]
            .get("dependencies")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            let dependency = dependency
                .as_str()
                .ok_or("Cargo.lock dependency is not a string")?;
            let matches = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| lock_dependency_matches(dependency, row))
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let [dependency_index] = matches.as_slice() else {
                return Err(format!(
                    "Cargo.lock dependency `{dependency}` does not select exactly one package"
                ));
            };
            pending.push(*dependency_index);
            let row = &rows[*dependency_index];
            if let Some(source) = row.get("source").and_then(toml::Value::as_str)
                && source.starts_with("registry+")
            {
                retained.insert(format!(
                    "{source}#{}@{}",
                    lock_string(row, "name")?,
                    lock_string(row, "version")?
                ));
            }
        }
    }

    let mut packages = Vec::new();
    for row in rows {
        let source = row
            .get("source")
            .and_then(toml::Value::as_str)
            .unwrap_or_default();
        if !source.starts_with("registry+") {
            continue;
        }
        let name = lock_string(row, "name")?;
        let version = lock_string(row, "version")?;
        lock_string(row, "checksum")?;
        if selected.contains(&(name, version)) {
            continue;
        }
        let id = format!("{source}#{name}@{version}");
        if !retained.contains(&id) {
            continue;
        }
        let package_root = vendor_root.join(format!("{name}-{version}"));
        let manifest = package_root.join("Cargo.toml");
        packages.push(ResolverPackage {
            id,
            name: name.to_owned(),
            target_entries: manifest_target_entries(&manifest, &package_root)?,
            manifest,
        });
    }
    packages.sort_by(|left, right| left.id.cmp(&right.id));
    if packages.windows(2).any(|pair| pair[0].id == pair[1].id) {
        return Err("Cargo.lock repeats a registry package identity".into());
    }
    Ok(packages)
}

fn uniquely_unused_patch(rows: &[toml::Value], name: &str, version: &str) -> bool {
    rows.iter()
        .filter(|row| {
            row.get("name").and_then(toml::Value::as_str) == Some(name)
                && row.get("version").and_then(toml::Value::as_str) == Some(version)
        })
        .count()
        == 1
}

fn lock_dependency_matches(dependency: &str, row: &toml::Value) -> bool {
    let Some(name) = row.get("name").and_then(toml::Value::as_str) else {
        return false;
    };
    let Some(version) = row.get("version").and_then(toml::Value::as_str) else {
        return false;
    };
    dependency == name
        || dependency == format!("{name} {version}")
        || row
            .get("source")
            .and_then(toml::Value::as_str)
            .is_some_and(|source| dependency == format!("{name} {version} ({source})"))
}

pub(super) fn manifest_target_entries(
    manifest: &Path,
    package_root: &Path,
) -> Result<Vec<PathBuf>, String> {
    let bytes = bounded_read(manifest)?;
    let root: toml::Value = toml::from_str(
        std::str::from_utf8(&bytes)
            .map_err(|error| format!("{} is not UTF-8: {error}", manifest.display()))?,
    )
    .map_err(|error| format!("{} is invalid: {error}", manifest.display()))?;
    let package = root
        .get("package")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| format!("{} has no package table", manifest.display()))?;
    let mut entries = Vec::new();
    match package.get("build") {
        Some(toml::Value::String(path)) => entries.push(package_root.join(path)),
        Some(toml::Value::Boolean(true)) => entries.push(package_root.join("build.rs")),
        Some(toml::Value::Boolean(false)) => {}
        Some(_) => {
            return Err(format!(
                "{} has an invalid build target",
                manifest.display()
            ));
        }
        None if package_root.join("build.rs").is_file() => {
            entries.push(package_root.join("build.rs"));
        }
        None => {}
    }
    if let Some(library) = root.get("lib").and_then(toml::Value::as_table) {
        let path = library
            .get("path")
            .and_then(toml::Value::as_str)
            .unwrap_or("src/lib.rs");
        entries.push(package_root.join(path));
    } else if package.get("autolib").and_then(toml::Value::as_bool) != Some(false)
        && package_root.join("src/lib.rs").is_file()
    {
        entries.push(package_root.join("src/lib.rs"));
    }
    for binary in root
        .get("bin")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        let path = binary
            .get("path")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("{} has a binary without a path", manifest.display()))?;
        entries.push(package_root.join(path));
    }
    if package.get("autobins").and_then(toml::Value::as_bool) != Some(false)
        && package_root.join("src/main.rs").is_file()
    {
        entries.push(package_root.join("src/main.rs"));
    }
    entries.sort();
    entries.dedup();
    if entries.iter().any(|path| !path.is_file()) {
        return Err(format!(
            "{} names a missing production target",
            manifest.display()
        ));
    }
    Ok(entries)
}

fn lock_string<'a>(row: &'a toml::Value, key: &str) -> Result<&'a str, String> {
    row.get(key)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 4096)
        .ok_or_else(|| format!("Cargo.lock registry package has no valid {key}"))
}

pub(super) fn copy_resolver(
    vendor: &mut TreeWriter,
    package: &ResolverPackage,
    vendor_root: &Path,
    lock: &[u8],
) -> Result<(), String> {
    let package_root = package_root(&package.manifest, vendor_root, &package.id)?;
    let destination = PathBuf::from(format!(
        "{}-{}",
        package.name,
        package_version(&package.id)?
    ));
    copy_manifest(vendor, &package.manifest, &destination, vendor_root)?;
    for target in &package.target_entries {
        let target = target
            .canonicalize()
            .map_err(|error| format!("{}: {error}", target.display()))?;
        let relative = target
            .strip_prefix(&package_root)
            .map_err(|_| format!("registry resolver target escaped package: {}", package.id))?;
        vendor.copy_tree(
            &target,
            &destination.join(relative),
            &[vendor_root.to_owned()],
        )?;
    }
    write_checksum(
        vendor,
        &destination,
        &package.name,
        package_version(&package.id)?,
        lock,
    )
}

pub(super) fn copy_selected(
    vendor: &mut TreeWriter,
    package: &Package,
    vendor_root: &Path,
    lock: &[u8],
) -> Result<(), String> {
    let package_root = package_root(&package.manifest, vendor_root, &package.id)?;
    let destination = PathBuf::from(format!("{}-{}", package.name, package.version));
    copy_manifest(vendor, &package.manifest, &destination, vendor_root)?;
    let mut copied_directories = BTreeSet::new();
    for target in &package.targets {
        let source = target
            .source
            .canonicalize()
            .map_err(|error| format!("{}: {error}", target.source.display()))?;
        let relative = source
            .strip_prefix(&package_root)
            .map_err(|_| format!("registry target escaped selected package: {}", package.id))?;
        if target.custom_build {
            vendor.copy_tree(
                &source,
                &destination.join(relative),
                &[vendor_root.to_owned()],
            )?;
            continue;
        }
        let directory = relative
            .parent()
            .ok_or_else(|| format!("registry target has no source directory: {}", package.id))?;
        if copied_directories.insert(directory.to_owned()) {
            vendor.copy_tree(
                &package_root.join(directory),
                &destination.join(directory),
                &[vendor_root.to_owned()],
            )?;
        }
    }
    write_checksum(vendor, &destination, &package.name, &package.version, lock)
}

fn copy_manifest(
    vendor: &mut TreeWriter,
    manifest: &Path,
    destination: &Path,
    vendor_root: &Path,
) -> Result<(), String> {
    vendor.copy_tree(
        manifest,
        &destination.join("Cargo.toml"),
        &[vendor_root.to_owned()],
    )
}

fn write_checksum(
    vendor: &mut TreeWriter,
    destination: &Path,
    name: &str,
    version: &str,
    lock: &[u8],
) -> Result<(), String> {
    let package_root = vendor.root.join(destination);
    let mut pending = vec![package_root.clone()];
    let mut files = BTreeMap::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                pending.push(entry.path());
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&package_root)
                .map_err(|_| "registry checksum path escaped package")?
                .to_str()
                .ok_or("registry checksum path is not UTF-8")?
                .replace('\\', "/");
            files.insert(relative, digest(&bounded_read(&entry.path())?));
        }
    }
    let package = lock_checksum(lock, name, version)?;
    let bytes = serde_json::to_vec(&serde_json::json!({"files": files, "package": package}))
        .map_err(|error| error.to_string())?;
    vendor.write_file(&destination.join(".cargo-checksum.json"), &bytes, false)
}

fn lock_checksum(lock: &[u8], name: &str, version: &str) -> Result<String, String> {
    let text = std::str::from_utf8(lock).map_err(|error| error.to_string())?;
    let root: toml::Value = toml::from_str(text).map_err(|error| error.to_string())?;
    root.get("package")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .find(|package| {
            package.get("name").and_then(toml::Value::as_str) == Some(name)
                && package.get("version").and_then(toml::Value::as_str) == Some(version)
                && package.get("source").is_some()
        })
        .and_then(|package| package.get("checksum"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("resolver lock has no registry checksum for {name} {version}"))
}

fn package_root(manifest: &Path, vendor_root: &Path, id: &str) -> Result<PathBuf, String> {
    let root = manifest
        .parent()
        .ok_or("registry package manifest has no parent")?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !root.starts_with(vendor_root) {
        return Err(format!("registry package escaped vendor root: {id}"));
    }
    Ok(root)
}

fn package_version(id: &str) -> Result<&str, String> {
    id.rsplit_once('@')
        .map(|(_, version)| version)
        .filter(|version| !version.is_empty() && version.len() <= 128)
        .ok_or_else(|| format!("registry package id has no version: {id}"))
}

#[cfg(test)]
mod tests {
    use super::uniquely_unused_patch;

    #[test]
    fn unused_patch_identity_must_be_exact_and_unique() {
        let lock: toml::Value = toml::from_str(
            r#"
                [[patch.unused]]
                name = "support"
                version = "1.2.3"
            "#,
        )
        .unwrap();
        let rows = lock["patch"]["unused"].as_array().unwrap();
        assert!(uniquely_unused_patch(rows, "support", "1.2.3"));
        assert!(!uniquely_unused_patch(rows, "substituted", "1.2.3"));
        assert!(!uniquely_unused_patch(rows, "support", "9.9.9"));

        let duplicate = vec![rows[0].clone(), rows[0].clone()];
        assert!(!uniquely_unused_patch(&duplicate, "support", "1.2.3"));
    }
}
