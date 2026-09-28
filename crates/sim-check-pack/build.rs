use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

fn main() {
    for variable in [
        "SIM_CONFORMANCE_PACKS_LOCK_FILE",
        "SIM_CONFORMANCE_PACKS_LOCK_SHA256",
        "RUSTC",
        "TARGET",
        "PROFILE",
        "OPT_LEVEL",
        "DEBUG",
        "CARGO_CFG_PANIC",
        "CARGO_CFG_TARGET_FEATURE",
        "CARGO_ENCODED_RUSTFLAGS",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let source = hash_source_inputs(&manifest_dir);
    let (lock_path, lock_bytes) = selected_lock(&manifest_dir);
    println!("cargo:rerun-if-changed={}", lock_path.display());
    let lock = hash_bytes(&lock_bytes);
    let rustc = env::var_os("RUSTC").expect("rustc executable");
    let toolchain = Command::new(rustc)
        .arg("-vV")
        .output()
        .expect("query rustc identity");
    assert!(toolchain.status.success(), "rustc -vV failed");
    let toolchain = hash_bytes(&toolchain.stdout);
    let mut features = env::vars()
        .filter_map(|(key, value)| key.starts_with("CARGO_FEATURE_").then_some((key, value)))
        .collect::<Vec<_>>();
    features.sort();
    let features = hash_lines(features.iter().map(|(key, value)| format!("{key}={value}")));
    let configuration = hash_lines(
        [
            "TARGET",
            "PROFILE",
            "OPT_LEVEL",
            "DEBUG",
            "CARGO_CFG_PANIC",
            "CARGO_CFG_TARGET_FEATURE",
            "CARGO_ENCODED_RUSTFLAGS",
        ]
        .into_iter()
        .map(|key| format!("{key}={}", env::var(key).unwrap_or_default())),
    );
    let exact = hash_lines([
        format!("source={source}"),
        format!("dependency-lock={lock}"),
        format!("toolchain={toolchain}"),
        format!("features={features}"),
        format!("build-configuration={configuration}"),
    ]);
    println!("cargo:rustc-env=SIM_CHECK_PACK_BUILD_INPUTS_SHA256={exact}");
    println!("cargo:rustc-env=SIM_CHECK_PACK_DEPENDENCY_LOCK_SHA256={lock}");
}

fn hash_source_inputs(manifest_dir: &Path) -> String {
    let mut files = vec![
        manifest_dir.join("Cargo.toml"),
        manifest_dir.join("build.rs"),
    ];
    // Fields such as `edition` are `.workspace = true` and actually come
    // from here; a change to it can change compilation without changing
    // this crate's own manifest.
    if let Some(workspace) = manifest_dir.parent() {
        files.push(workspace.join("Cargo.toml"));
    }
    collect_files(&manifest_dir.join("src"), &mut files);
    files.sort();
    let mut hasher = Sha256::new();
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let name = match path.strip_prefix(manifest_dir) {
            Ok(relative) => relative.to_string_lossy().into_owned(),
            Err(_) => format!("../{}", path.file_name().expect("named path").to_string_lossy()),
        };
        let bytes = fs::read(&path).expect("read adapter source input");
        hasher.update((name.len() as u64).to_be_bytes());
        hasher.update(name.as_bytes());
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    hex(hasher.finalize())
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(directory)
        .expect("read source directory")
        .map(|entry| entry.expect("read source entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files(&path, files);
        } else if path.is_file() {
            files.push(path);
        }
    }
}

fn find_lock(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|directory| directory.join("Cargo.lock"))
        .find(|path| path.is_file())
}

fn selected_lock(manifest_dir: &Path) -> (PathBuf, Vec<u8>) {
    let selected = env::var_os("SIM_CONFORMANCE_PACKS_LOCK_FILE");
    let expected = env::var("SIM_CONFORMANCE_PACKS_LOCK_SHA256").ok();
    match (selected, expected) {
        (Some(path), Some(expected)) => {
            assert!(
                expected.len() == 64
                    && expected
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
                "SIM_CONFORMANCE_PACKS_LOCK_SHA256 must be lowercase SHA-256"
            );
            let path = PathBuf::from(path);
            assert!(
                path.is_absolute(),
                "SIM_CONFORMANCE_PACKS_LOCK_FILE must be absolute"
            );
            let metadata = fs::symlink_metadata(&path).expect("inspect selected dependency lock");
            assert!(
                metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
                "selected dependency lock must be a regular non-symlink file"
            );
            let bytes = fs::read(&path).expect("read selected dependency lock");
            let actual = hash_bytes(&bytes);
            assert_eq!(actual, expected, "selected dependency lock digest differs");
            (path, bytes)
        }
        (None, None) => {
            let packaged = manifest_dir.join("Cargo.lock");
            if packaged.is_file() {
                let bytes = fs::read(&packaged).expect("read packaged dependency lock");
                return (packaged, bytes);
            }
            let path = find_lock(manifest_dir)
                .expect("sim-check-pack requires an exact Cargo.lock build input");
            let bytes = fs::read(&path).expect("read primary dependency lock");
            (path, bytes)
        }
        _ => panic!(
            "SIM_CONFORMANCE_PACKS_LOCK_FILE and SIM_CONFORMANCE_PACKS_LOCK_SHA256 must be supplied together"
        ),
    }
}

fn hash_lines(lines: impl IntoIterator<Item = String>) -> String {
    let mut hasher = Sha256::new();
    for line in lines {
        hasher.update((line.len() as u64).to_be_bytes());
        hasher.update(line.as_bytes());
    }
    hex(hasher.finalize())
}

fn hash_bytes(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes))
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
