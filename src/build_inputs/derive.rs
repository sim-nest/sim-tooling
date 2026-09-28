//! Derives Cargo metadata and a unit graph from one exact materialized input.

use super::{bounded_read, canonical_directory, digest, staging_path, sync_directory, write_new};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const MAXIMUM_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_DIAGNOSTIC_BYTES: usize = 2 * 1024 * 1024;

struct Options {
    workspace: PathBuf,
    vendor: PathBuf,
    materialization: PathBuf,
    cargo: PathBuf,
    rustc: PathBuf,
    destination: PathBuf,
    package: String,
    binary: String,
    target: String,
    profile: Profile,
    expected_materialization: String,
    expected_cargo: String,
    expected_rustc: String,
}

#[derive(Clone, Copy)]
enum Profile {
    Development,
    Release,
}

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let destination = options.destination.clone();
    derive(options)?;
    println!(
        "build-inputs: exact Cargo metadata and unit graph derived at {}",
        destination.display()
    );
    Ok(())
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut values = BTreeMap::new();
    let mut index = 3;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} requires a value"))?;
        if values.insert(flag, value.clone()).is_some() {
            return Err(format!("build-inputs derive repeats {flag}"));
        }
        index += 2;
    }
    let required = |name: &'static str| {
        values
            .get(name)
            .cloned()
            .ok_or_else(|| format!("build-inputs derive requires {name}"))
    };
    let package = required("--package")?;
    let binary = required("--bin")?;
    for (role, value) in [("package", &package), ("binary", &binary)] {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(format!("build-inputs derive {role} is invalid"));
        }
    }
    let target = required("--target")?;
    if target.is_empty()
        || target.len() > 128
        || !target
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("build-inputs derive target is invalid".into());
    }
    let profile = match required("--profile")?.as_str() {
        "development" => Profile::Development,
        "release" => Profile::Release,
        _ => return Err("build-inputs derive profile is invalid".into()),
    };
    let expected_materialization =
        expected_digest(&required("--expected-materialization-sha256")?)?;
    let expected_cargo = expected_digest(&required("--expected-cargo-sha256")?)?;
    let expected_rustc = expected_digest(&required("--expected-rustc-sha256")?)?;
    Ok(Options {
        workspace: PathBuf::from(required("--workspace")?),
        vendor: PathBuf::from(required("--vendor-root")?),
        materialization: PathBuf::from(required("--materialization")?),
        cargo: PathBuf::from(required("--cargo")?),
        rustc: PathBuf::from(required("--rustc")?),
        destination: PathBuf::from(required("--destination")?),
        package,
        binary,
        target,
        profile,
        expected_materialization,
        expected_cargo,
        expected_rustc,
    })
}

fn derive(options: Options) -> Result<(), String> {
    let workspace = canonical_directory(&options.workspace)?;
    let vendor = canonical_directory(&options.vendor)?;
    // `workspace` carries its own content-bound `.cargo/config.toml`,
    // written and verified by materialization (re-checked again below by
    // `require_tree_matches_record`); Cargo also merges any config from
    // every ancestor ABOVE it, which nothing checks. `invoke` already gives
    // every cargo run a private, empty Cargo home, so this closes the
    // remaining ambient-config surface.
    if let Some(above) = workspace.parent() {
        crate::toolchain_identity::require_no_cargo_config(&[above])?;
    }
    let materialization = options
        .materialization
        .canonicalize()
        .map_err(|error| format!("{}: {error}", options.materialization.display()))?;
    let owner = workspace
        .parent()
        .filter(|owner| vendor.parent() == Some(*owner))
        .ok_or("build-input source and vendor do not share one materialization owner")?;
    if materialization != owner.join("materialization.json") {
        return Err("build-input derivation record is not owned beside the selected trees".into());
    }
    let materialization_bytes = bounded_read(&materialization)?;
    if digest(&materialization_bytes) != options.expected_materialization {
        return Err("build-input derivation materialization digest differs".into());
    }
    let materialization_value: Json = serde_json::from_slice(&materialization_bytes)
        .map_err(|error| format!("materialization record is invalid: {error}"))?;
    if materialization_value.get("schema").and_then(Json::as_str)
        != Some("sim.build-input-materialization/v1")
    {
        return Err("build-input derivation materialization schema differs".into());
    }
    // The record's own digest, just checked, proves only that this exact
    // JSON was produced by a real materialization at some point; it says
    // nothing about whether `workspace`/`vendor` still hold what that
    // record describes. Nothing else here re-reads them before running
    // cargo, so re-verify both trees against the record's own per-file
    // identities now, while there is still a trusted record to check them
    // against.
    require_tree_matches_record(&workspace, &materialization_value["source"], "source")?;
    require_tree_matches_record(&vendor, &materialization_value["vendor"], "vendor")?;
    let cargo = exact_file(&options.cargo, &options.expected_cargo, "Cargo")?;
    let rustc = exact_file(&options.rustc, &options.expected_rustc, "rustc")?;
    if options.destination.exists() {
        return Err("build-input derivation destination already exists".into());
    }
    let parent = options
        .destination
        .parent()
        .ok_or("build-input derivation destination has no parent")?;
    let parent = canonical_directory(parent)?;
    let staging = staging_path(&parent, &options.destination)?;
    fs::create_dir(&staging).map_err(|error| format!("{}: {error}", staging.display()))?;
    let work = staging.join("work");
    for path in [
        work.clone(),
        work.join("cargo-home"),
        work.join("target"),
        work.join("tmp"),
    ] {
        fs::create_dir(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    let result = derive_into(DeriveInputs {
        staging: &staging,
        work: &work,
        workspace: &workspace,
        vendor: &vendor,
        materialization_sha256: &options.expected_materialization,
        cargo: &cargo,
        cargo_sha256: &options.expected_cargo,
        rustc: &rustc,
        rustc_sha256: &options.expected_rustc,
        package: &options.package,
        binary: &options.binary,
        target: &options.target,
        profile: options.profile,
    });
    result.map_err(|error| {
        format!(
            "{error}; incomplete derivation retained at {}",
            staging.display()
        )
    })?;
    fs::remove_dir_all(&work).map_err(|error| format!("{}: {error}", work.display()))?;
    sync_directory(&staging)?;
    fs::rename(&staging, &options.destination).map_err(|error| {
        format!(
            "publish {} as {}: {error}",
            staging.display(),
            options.destination.display()
        )
    })?;
    sync_directory(&parent)
}

/// Every file `report` (a materialization record's `TreeReport`, as JSON)
/// names under `root` must currently have exactly the recorded bytes and
/// executable bit, and `root` must hold no file the record does not name:
/// a materialized tree sits on disk as ordinary files between the
/// `materialize` step that produces it and a later `derive` step that
/// trusts it, and nothing else re-reads it in between to notice a change.
fn require_tree_matches_record(root: &Path, report: &Json, label: &str) -> Result<(), String> {
    let recorded = report["files"]
        .as_array()
        .ok_or_else(|| format!("materialization record has no {label} files"))?;
    let mut expected = BTreeMap::new();
    for file in recorded {
        let path = file["path"]
            .as_str()
            .ok_or_else(|| format!("materialization record {label} file has no path"))?;
        let sha256 = file["sha256"]
            .as_str()
            .ok_or_else(|| format!("materialization record {label} file has no sha256"))?;
        let executable = file["executable"].as_bool().unwrap_or(false);
        if expected
            .insert(path.to_owned(), (sha256.to_owned(), executable))
            .is_some()
        {
            return Err(format!(
                "materialization record repeats {label} file {path}"
            ));
        }
    }
    let mut seen = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).map_err(|error| format!("{}: {error}", dir.display()))? {
            let entry = entry.map_err(|error| format!("{}: {error}", dir.display()))?;
            let kind = entry
                .file_type()
                .map_err(|error| format!("{}: {error}", entry.path().display()))?;
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !kind.is_file() {
                return Err(format!(
                    "materialized {label} tree has a non-regular entry: {}",
                    entry.path().display()
                ));
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| format!("materialized {label} file escaped its root"))?
                .to_str()
                .ok_or_else(|| format!("materialized {label} file path is not UTF-8"))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            let Some((expected_sha256, expected_executable)) = expected.get(&relative) else {
                return Err(format!(
                    "materialized {label} tree has an unrecorded file: {relative}"
                ));
            };
            let bytes = bounded_read(&entry.path())?;
            if digest(&bytes) != *expected_sha256 {
                return Err(format!(
                    "materialized {label} file changed since materialization: {relative}"
                ));
            }
            let executable = entry
                .metadata()
                .map_err(|error| format!("{}: {error}", entry.path().display()))?
                .permissions()
                .mode()
                & 0o111
                != 0;
            if executable != *expected_executable {
                return Err(format!(
                    "materialized {label} file permissions changed since materialization: {relative}"
                ));
            }
            seen.insert(relative);
        }
    }
    if seen.len() != expected.len() {
        let missing = expected
            .keys()
            .filter(|path| !seen.contains(path.as_str()))
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "materialized {label} tree is missing recorded files: {missing}"
        ));
    }
    Ok(())
}

struct DeriveInputs<'a> {
    staging: &'a Path,
    work: &'a Path,
    workspace: &'a Path,
    vendor: &'a Path,
    materialization_sha256: &'a str,
    cargo: &'a Path,
    cargo_sha256: &'a str,
    rustc: &'a Path,
    rustc_sha256: &'a str,
    package: &'a str,
    binary: &'a str,
    target: &'a str,
    profile: Profile,
}

fn derive_into(input: DeriveInputs<'_>) -> Result<(), String> {
    let config = format!(
        "source.nv12-selected-vendor.directory=\"{}\"",
        input.vendor.display()
    );
    let metadata = invoke(
        &input,
        "metadata",
        &[
            "--config",
            &config,
            "metadata",
            "--manifest-path",
            "Cargo.toml",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--quiet",
        ],
    )?;
    validate_metadata(&metadata.stdout)?;
    let mut graph_args = vec![
        "--config",
        &config,
        "build",
        "--manifest-path",
        "Cargo.toml",
        "--package",
        input.package,
        "--bin",
        input.binary,
        "--target",
        input.target,
        "--locked",
        "--offline",
        "-Z",
        "unstable-options",
        "--unit-graph",
    ];
    if matches!(input.profile, Profile::Release) {
        graph_args.push("--release");
    }
    let graph = invoke(&input, "unit-graph", &graph_args)?;
    validate_graph(&graph.stdout)?;
    let profile = match input.profile {
        Profile::Development => "development",
        Profile::Release => "release",
    };
    let report = serde_json::to_vec_pretty(&json!({
        "schema": "sim.build-input-derivation/v1",
        "materialization_sha256": input.materialization_sha256,
        "cargo": {"path": input.cargo, "sha256": input.cargo_sha256},
        "rustc": {"path": input.rustc, "sha256": input.rustc_sha256},
        "selection": {
            "package": input.package,
            "binary": input.binary,
            "target": input.target,
            "profile": profile
        },
        "outputs": {
            "metadata": {"sha256": digest(&metadata.stdout), "bytes": metadata.stdout.len()},
            "unit_graph": {"sha256": digest(&graph.stdout), "bytes": graph.stdout.len()}
        }
    }))
    .map_err(|error| error.to_string())?;
    write_new(&input.staging.join("derivation.json"), &report, false)
}

fn invoke(input: &DeriveInputs<'_>, role: &str, args: &[&str]) -> Result<Output, String> {
    let output = Command::new(input.cargo)
        .args(args)
        .current_dir(input.workspace)
        .env_clear()
        .env("CARGO_HOME", input.work.join("cargo-home"))
        .env("CARGO_TARGET_DIR", input.work.join("target"))
        .env("CARGO_INCREMENTAL", "0")
        .env("RUSTC", input.rustc)
        .env("RUSTC_BOOTSTRAP", "1")
        .env("TMPDIR", input.work.join("tmp"))
        .env("PATH", "/usr/bin:/bin")
        .output()
        .map_err(|error| format!("start Cargo {role}: {error}"))?;
    if output.stdout.len() > MAXIMUM_OUTPUT_BYTES || output.stderr.len() > MAXIMUM_DIAGNOSTIC_BYTES
    {
        return Err(format!("Cargo {role} output exceeds its finite bound"));
    }
    write_new(
        &input.staging.join(format!("{role}.stdout")),
        &output.stdout,
        false,
    )?;
    write_new(
        &input.staging.join(format!("{role}.stderr")),
        &output.stderr,
        false,
    )?;
    if !output.status.success() {
        return Err(format!("Cargo {role} failed with {}", output.status));
    }
    Ok(output)
}

fn validate_metadata(bytes: &[u8]) -> Result<(), String> {
    let value: Json = serde_json::from_slice(bytes)
        .map_err(|error| format!("Cargo metadata output is invalid: {error}"))?;
    let packages = value.get("packages").and_then(Json::as_array);
    let nodes = value
        .get("resolve")
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(Json::as_array);
    if packages.is_none_or(|rows| rows.is_empty() || rows.len() > 16_384)
        || nodes.is_none_or(|rows| rows.is_empty() || rows.len() > 16_384)
    {
        return Err("Cargo metadata output is empty or unbounded".into());
    }
    Ok(())
}

fn validate_graph(bytes: &[u8]) -> Result<(), String> {
    let value: Json = serde_json::from_slice(bytes)
        .map_err(|error| format!("Cargo unit graph output is invalid: {error}"))?;
    let units = value.get("units").and_then(Json::as_array);
    if value.get("version").and_then(Json::as_u64) != Some(1)
        || units.is_none_or(|rows| rows.is_empty() || rows.len() > 16_384)
    {
        return Err("Cargo unit graph output schema or unit count differs".into());
    }
    Ok(())
}

fn exact_file(path: &Path, expected: &str, role: &str) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if !canonical.is_file() || file_digest(&canonical)? != expected {
        return Err(format!("build-input derivation {role} identity differs"));
    }
    Ok(canonical)
}

fn file_digest(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn expected_digest(value: &str) -> Result<String, String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("build-input derivation expected digest is invalid".into());
    }
    Ok(value.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::{parse, validate_graph, validate_metadata};

    fn arguments() -> Vec<String> {
        [
            "xtask",
            "build-inputs",
            "derive",
            "--workspace",
            "/source",
            "--vendor-root",
            "/vendor",
            "--materialization",
            "/materialization.json",
            "--expected-materialization-sha256",
            &"a".repeat(64),
            "--cargo",
            "/toolchain/cargo",
            "--expected-cargo-sha256",
            &"b".repeat(64),
            "--rustc",
            "/toolchain/rustc",
            "--expected-rustc-sha256",
            &"c".repeat(64),
            "--package",
            "sim-run",
            "--bin",
            "sim",
            "--target",
            "x86_64-unknown-linux-gnu",
            "--profile",
            "release",
            "--destination",
            "/output",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    #[test]
    fn exact_target_is_required_and_retained() {
        let arguments = arguments();
        assert_eq!(
            parse(&arguments).expect("complete derivation").target,
            "x86_64-unknown-linux-gnu"
        );

        let mut missing = arguments;
        let target = missing
            .iter()
            .position(|argument| argument == "--target")
            .expect("target flag");
        missing.drain(target..=target + 1);
        let error = match parse(&missing) {
            Ok(_) => panic!("missing target must fail"),
            Err(error) => error,
        };
        assert_eq!(error, "build-inputs derive requires --target");
    }

    #[test]
    fn cargo_outputs_require_bounded_nonempty_owner_records() {
        validate_metadata(br#"{"packages":[{}],"resolve":{"nodes":[{}]}}"#)
            .expect("bounded metadata");
        validate_graph(br#"{"version":1,"units":[{}]}"#).expect("bounded graph");
        assert!(validate_metadata(br#"{"packages":[],"resolve":{"nodes":[]}}"#).is_err());
        assert!(validate_graph(br#"{"version":1,"units":[]}"#).is_err());
    }
}
