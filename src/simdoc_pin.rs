// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The committed identity of the contract engine and its toolchain, and its
//! enforcement.
//!
//! The repository commits the identity it trusts under
//! `[workspace.metadata.sim.encoder]` in its root `Cargo.toml`:
//!
//! ```toml
//! [workspace.metadata.sim.encoder]
//! source_sha256 = "..."
//! lock_sha256 = "..."
//! toolchain = "1.96.0"
//! rustc_commit = "..."
//! cargo_commit = "..."
//!
//! [workspace.metadata.sim.encoder.toolchain_sha256]
//! x86_64-unknown-linux-gnu = "..."
//! ```
//!
//! Before xtask runs the engine it verifies, independently of the engine's
//! own code, that the engine's source tree and lock hash to the committed
//! digests and that the tooling pins the committed toolchain. The toolchain
//! is then found without executing anything on `PATH`, not even `rustup`: a
//! directory (the one holding the `cargo` that started xtask, or the pinned
//! channel under `RUSTUP_HOME`/`~/.rustup`) is accepted only when its
//! `cargo`, `rustc`, `rustdoc`, and libraries hash to the digest committed for
//! its host and the `-vV` commits of `cargo` and `rustc` equal the committed
//! ones. Every child runs by absolute path with a cleared environment. After
//! building, xtask asks the executable for its embedded identity and refuses
//! unless every field matches. The committed identity changes only through
//! `xtask simdoc-pin`, an explicit command whose diff is reviewed like any
//! other.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

use crate::toolchain_identity::{host_triple, tool_identity, toolchain_digest};

const PIN_TABLE: &str = "[workspace.metadata.sim.encoder]";
const DIGEST_TABLE: &str = "[workspace.metadata.sim.encoder.toolchain_sha256]";

/// The committed engine and toolchain identity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EncoderPin {
    pub(crate) source_sha256: String,
    pub(crate) lock_sha256: String,
    pub(crate) toolchain: String,
    pub(crate) rustc_commit: String,
    pub(crate) cargo_commit: String,
    /// Content digest of the pinned toolchain, per host triple.
    pub(crate) toolchain_sha256: BTreeMap<String, String>,
}

impl EncoderPin {
    /// Reads the committed identity from a root manifest.
    pub(crate) fn from_manifest(text: &str) -> Result<Self, String> {
        let table: toml::Table = text
            .parse()
            .map_err(|err| format!("parse root Cargo.toml: {err}"))?;
        let encoder = table
            .get("workspace")
            .and_then(|value| value.get("metadata"))
            .and_then(|value| value.get("sim"))
            .and_then(|value| value.get("encoder"))
            .ok_or_else(|| {
                format!("the root Cargo.toml commits no {PIN_TABLE}; run `cargo run -p xtask -- simdoc-pin`")
            })?;
        let field = |name: &str| {
            encoder
                .get(name)
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("{PIN_TABLE} has no string `{name}`"))
        };
        let toolchain_sha256 = encoder
            .get("toolchain_sha256")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| format!("{DIGEST_TABLE} is missing"))?
            .iter()
            .map(|(host, digest)| {
                digest
                    .as_str()
                    .map(|digest| (host.clone(), digest.to_owned()))
                    .ok_or_else(|| format!("{DIGEST_TABLE} `{host}` is not a string"))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        Ok(Self {
            source_sha256: field("source_sha256")?,
            lock_sha256: field("lock_sha256")?,
            toolchain: field("toolchain")?,
            rustc_commit: field("rustc_commit")?,
            cargo_commit: field("cargo_commit")?,
            toolchain_sha256,
        })
    }

    /// Computes the engine part of the identity (source, lock, channel) of
    /// the engine at `simdoc_dir` in `tooling_root`. The toolchain fields are
    /// left empty: they come from a located toolchain.
    pub(crate) fn measure(tooling_root: &Path, simdoc_dir: &Path) -> Result<Self, String> {
        Ok(Self {
            source_sha256: source_digest(simdoc_dir)?,
            lock_sha256: file_digest(&simdoc_dir.join("Cargo.lock"))?,
            toolchain: pinned_channel(tooling_root)?,
            ..Self::default()
        })
    }

    /// Whether the engine part (source, lock, channel) equals `other`'s.
    fn same_engine(&self, other: &Self) -> bool {
        self.source_sha256 == other.source_sha256
            && self.lock_sha256 == other.lock_sha256
            && self.toolchain == other.toolchain
    }

    fn table(&self) -> String {
        let mut text = format!(
            "{PIN_TABLE}\nsource_sha256 = \"{}\"\nlock_sha256 = \"{}\"\ntoolchain = \"{}\"\n\
             rustc_commit = \"{}\"\ncargo_commit = \"{}\"\n\n{DIGEST_TABLE}\n",
            self.source_sha256,
            self.lock_sha256,
            self.toolchain,
            self.rustc_commit,
            self.cargo_commit
        );
        for (host, digest) in &self.toolchain_sha256 {
            text.push_str(&format!("{host} = \"{digest}\"\n"));
        }
        text
    }

    /// Rewrites the committed identity tables in `manifest`.
    pub(crate) fn write_into(&self, manifest: &str) -> String {
        let mut kept = Vec::new();
        let mut skipping = false;
        for line in manifest.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                skipping = trimmed == PIN_TABLE || trimmed == DIGEST_TABLE;
            }
            if !skipping {
                kept.push(line);
            }
        }
        let mut text = kept.join("\n").trim_end().to_owned();
        text.push_str("\n\n");
        text.push_str(&self.table());
        text
    }
}

/// SHA-256 over the engine's `Cargo.toml`, `Cargo.lock`, `build.rs`,
/// `build_identity.rs`, and `src` tree: each file's slash-separated relative
/// path, a NUL, its length (u64 little-endian), and its bytes, in `Path` order (component-wise, so `src/a/b.rs` sorts before
/// `src/a.rs`; not the byte order of the joined strings).
/// Any symlink refuses the digest. This is computed here, not by the engine,
/// so a modified engine cannot vouch for itself.
pub(crate) fn source_digest(simdoc_dir: &Path) -> Result<String, String> {
    let mut files = vec![
        PathBuf::from("Cargo.toml"),
        PathBuf::from("Cargo.lock"),
        PathBuf::from("build.rs"),
        PathBuf::from("build_identity.rs"),
    ];
    collect(simdoc_dir, Path::new("src"), &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for relative in files {
        let bytes = ordinary_bytes(&simdoc_dir.join(&relative))?;
        let name = relative
            .to_str()
            .ok_or("engine source path is not UTF-8")?
            .replace(std::path::MAIN_SEPARATOR, "/");
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hex(&hasher.finalize()))
}

fn collect(root: &Path, relative: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let dir = root.join(relative);
    let metadata = fs::symlink_metadata(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("engine source {} is a symlink", dir.display()));
    }
    for entry in fs::read_dir(&dir).map_err(|err| format!("{}: {err}", dir.display()))? {
        let entry = entry.map_err(|err| format!("{}: {err}", dir.display()))?;
        let path = relative.join(entry.file_name());
        let kind = entry
            .file_type()
            .map_err(|err| format!("{}: {err}", path.display()))?;
        if kind.is_dir() {
            collect(root, &path, files)?;
        } else if kind.is_file() {
            files.push(path);
        } else {
            return Err(format!(
                "engine source {} is not an ordinary file",
                path.display()
            ));
        }
    }
    Ok(())
}

fn ordinary_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|err| format!("{}: {err}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "engine source {} is not an ordinary file",
            path.display()
        ));
    }
    fs::read(path).map_err(|err| format!("{}: {err}", path.display()))
}

fn file_digest(path: &Path) -> Result<String, String> {
    Ok(hex(&Sha256::digest(ordinary_bytes(path)?)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The channel pinned by the tooling's `rust-toolchain.toml`.
pub(crate) fn pinned_channel(tooling_root: &Path) -> Result<String, String> {
    let text = fs::read_to_string(tooling_root.join("rust-toolchain.toml"))
        .map_err(|err| format!("read rust-toolchain.toml: {err}"))?;
    text.lines()
        .find_map(|line| {
            let value = line
                .trim()
                .strip_prefix("channel")?
                .trim()
                .strip_prefix('=')?;
            Some(value.trim().trim_matches('"').to_owned())
        })
        .ok_or_else(|| "rust-toolchain.toml pins no channel".to_owned())
}

/// A toolchain accepted against the committed identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Toolchain {
    pub(crate) cargo: PathBuf,
    pub(crate) rustc: PathBuf,
    pub(crate) rustdoc: PathBuf,
    pub(crate) channel: String,
    pub(crate) host: String,
}

/// Toolchain directories worth examining, in order: the one holding the
/// `cargo` that started this process, then the pinned channel under the
/// rustup home for each host the pin knows. Nothing here is trusted: every
/// candidate is accepted only by content.
pub(crate) fn candidate_roots(pin: &EncoderPin) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(cargo) = std::env::var_os("CARGO").map(PathBuf::from)
        && cargo.is_absolute()
        && let Some(root) = cargo.parent().and_then(Path::parent)
    {
        roots.push(root.to_path_buf());
    }
    let home = std::env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".rustup")));
    if let Some(home) = home {
        for host in pin.toolchain_sha256.keys() {
            roots.push(
                home.join("toolchains")
                    .join(format!("{}-{host}", pin.toolchain)),
            );
        }
    }
    roots
}

/// Accepts the first of `candidates` whose content is exactly the toolchain
/// the pin commits: its files hash to the digest committed for a host, its
/// `-vV` reports the pinned release, that host, and the committed commits.
/// `rustup` is never run.
pub(crate) fn locate_toolchain(
    pin: &EncoderPin,
    candidates: &[PathBuf],
) -> Result<Toolchain, String> {
    let mut refusals = Vec::new();
    for root in candidates {
        match accept(pin, root) {
            Ok(toolchain) => return Ok(toolchain),
            Err(why) => refusals.push(format!("{}: {why}", root.display())),
        }
    }
    Err(format!(
        "no toolchain matches the committed identity (release {}, commits {} / {}); \
         candidates: {}; install the pinned toolchain or review and run `cargo run -p xtask -- simdoc-pin`",
        pin.toolchain,
        pin.rustc_commit,
        pin.cargo_commit,
        if refusals.is_empty() {
            "none found".to_owned()
        } else {
            refusals.join("; ")
        }
    ))
}

fn accept(pin: &EncoderPin, root: &Path) -> Result<Toolchain, String> {
    let digest = toolchain_digest(root).map_err(|err| err.to_string())?;
    let Some((host, _)) = pin
        .toolchain_sha256
        .iter()
        .find(|(_, committed)| **committed == digest)
    else {
        return Err(format!(
            "content digest {digest} is not committed for any host"
        ));
    };
    let toolchain = Toolchain {
        cargo: root.join("bin/cargo"),
        rustc: root.join("bin/rustc"),
        rustdoc: root.join("bin/rustdoc"),
        channel: pin.toolchain.clone(),
        host: host.clone(),
    };
    for (path, commit) in [
        (&toolchain.cargo, &pin.cargo_commit),
        (&toolchain.rustc, &pin.rustc_commit),
    ] {
        let (release, found, reported_host) = reported(path)?;
        if release != pin.toolchain || found != *commit || reported_host != *host {
            return Err(format!(
                "{} reports {release} ({found}) for {reported_host}, not the committed {} \
                 ({commit}) for {host}",
                path.display(),
                pin.toolchain
            ));
        }
    }
    Ok(toolchain)
}

/// `(release, commit-hash, host)` a tool reports, run by absolute path with an
/// empty environment.
fn reported(tool: &Path) -> Result<(String, String, String), String> {
    let output = Command::new(tool)
        .env_clear()
        .args(["--version", "--verbose"])
        .output()
        .map_err(|err| format!("run {} -vV: {err}", tool.display()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let (release, commit) = tool_identity(&text)
        .ok_or_else(|| format!("{} -vV reports no identity", tool.display()))?;
    let host =
        host_triple(&text).ok_or_else(|| format!("{} -vV reports no host", tool.display()))?;
    Ok((release, commit, host))
}

/// The environment variables a launched child inherits from the caller.
const PASSED_ENVIRONMENT: [&str; 6] = [
    "CARGO_NET_OFFLINE",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "SIMDOC_CARGO_MANIFEST_PATH",
    "SIMDOC_FORCE_DOCS",
];

impl Toolchain {
    /// [`Toolchain::command`] run in `cwd`, provided no Cargo configuration
    /// applies to `cwd` or to any of `manifests` (see
    /// [`crate::toolchain_identity::require_no_cargo_config`]).
    pub(crate) fn command_in(
        &self,
        program: &Path,
        cwd: &Path,
        manifests: &[&Path],
    ) -> Result<Command, String> {
        let mut places = vec![cwd];
        places.extend(manifests.iter().copied());
        crate::toolchain_identity::require_no_cargo_config(&places)?;
        let mut command = self.command(program);
        command.current_dir(cwd);
        Ok(command)
    }

    /// A command that runs `program` (an absolute path) with a cleared
    /// environment, a `PATH` of this toolchain's directory and the system
    /// binary directories, and this toolchain named for every tool.
    pub(crate) fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command.env_clear();
        for name in PASSED_ENVIRONMENT {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let bin = self
            .cargo
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mut path = OsString::from(bin);
        path.push(":/usr/bin:/bin");
        command
            .env("PATH", path)
            .env("RUSTC", &self.rustc)
            .env("RUSTDOC", &self.rustdoc)
            .env("RUSTC_WRAPPER", "")
            .env("RUSTC_WORKSPACE_WRAPPER", "");
        command
    }
}

/// Requires the executable's reported identity to be exactly the pin, for the
/// host `toolchain` was accepted for.
pub(crate) fn verify_identity(
    reported: &str,
    pin: &EncoderPin,
    toolchain: &Toolchain,
) -> Result<(), String> {
    let field = |name: &str| {
        reported
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .unwrap_or_default()
    };
    let digest = pin
        .toolchain_sha256
        .get(&toolchain.host)
        .map_or("", String::as_str);
    for (name, expected) in [
        ("source_sha256", pin.source_sha256.as_str()),
        ("lock_sha256", pin.lock_sha256.as_str()),
        ("rustc_release", pin.toolchain.as_str()),
        ("cargo_release", pin.toolchain.as_str()),
        ("rustc_commit", pin.rustc_commit.as_str()),
        ("cargo_commit", pin.cargo_commit.as_str()),
        ("toolchain_sha256", digest),
    ] {
        let actual = field(name);
        if actual != expected {
            return Err(format!(
                "the built engine reports {name}={actual:?}, not the committed {expected:?}"
            ));
        }
    }
    Ok(())
}

/// Requires the engine on disk to be the committed one.
pub(crate) fn verify_source(tooling_root: &Path, pin: &EncoderPin) -> Result<(), String> {
    let measured = EncoderPin::measure(tooling_root, &tooling_root.join("crates/simdoc"))?;
    if !measured.same_engine(pin) {
        return Err(format!(
            "the engine on disk is not the committed one (committed source {}, lock {}, \
             toolchain {}; found source {}, lock {}, toolchain {}); review the change and \
             run `cargo run -p xtask -- simdoc-pin`",
            pin.source_sha256,
            pin.lock_sha256,
            pin.toolchain,
            measured.source_sha256,
            measured.lock_sha256,
            measured.toolchain
        ));
    }
    Ok(())
}

/// The toolchain of the `cargo` that started this process: `(host, digest,
/// rustc commit, cargo commit)`, provided it is the `channel` release.
fn running_toolchain(channel: &str) -> Result<(String, String, String, String), String> {
    let cargo = std::env::var_os("CARGO")
        .map(PathBuf::from)
        .filter(|cargo| cargo.is_absolute())
        .ok_or("run simdoc-pin through `cargo run -p xtask`, which names the toolchain")?;
    let root = cargo
        .parent()
        .and_then(Path::parent)
        .ok_or("cargo does not sit in a toolchain bin directory")?;
    let digest = toolchain_digest(root).map_err(|err| err.to_string())?;
    let (cargo_release, cargo_commit, host) = reported(&root.join("bin/cargo"))?;
    let (rustc_release, rustc_commit, rustc_host) = reported(&root.join("bin/rustc"))?;
    if cargo_release != channel || rustc_release != channel || host != rustc_host {
        return Err(format!(
            "the running toolchain is cargo {cargo_release} / rustc {rustc_release}, not the \
             pinned {channel}"
        ));
    }
    Ok((host, digest, rustc_commit, cargo_commit))
}

/// Runs `simdoc-pin [--check]`: writes (or checks) the committed identity.
pub(crate) fn run(tooling_root: &Path, args: &[String]) -> Result<(), String> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => return Err("usage: xtask simdoc-pin [--check]".to_owned()),
    };
    let manifest_path = tooling_root.join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|err| format!("read {}: {err}", manifest_path.display()))?;
    let mut measured = EncoderPin::measure(tooling_root, &tooling_root.join("crates/simdoc"))?;
    let (host, digest, rustc_commit, cargo_commit) = running_toolchain(&measured.toolchain)?;
    measured.rustc_commit = rustc_commit;
    measured.cargo_commit = cargo_commit;
    let committed = EncoderPin::from_manifest(&manifest);
    if check {
        let committed = committed?;
        if !committed.same_engine(&measured)
            || committed.rustc_commit != measured.rustc_commit
            || committed.cargo_commit != measured.cargo_commit
            || committed.toolchain_sha256.get(&host) != Some(&digest)
        {
            return Err(format!(
                "the committed engine identity is stale for {host} (committed {committed:?}, \
                 found {measured:?} with toolchain digest {digest})"
            ));
        }
        println!("simdoc-pin: committed engine identity is current");
        return Ok(());
    }
    // Other hosts' digests stay only while they describe the same release.
    if let Ok(previous) = committed
        && previous.toolchain == measured.toolchain
        && previous.rustc_commit == measured.rustc_commit
        && previous.cargo_commit == measured.cargo_commit
    {
        measured.toolchain_sha256 = previous.toolchain_sha256;
    }
    measured.toolchain_sha256.insert(host.clone(), digest);
    fs::write(&manifest_path, measured.write_into(&manifest))
        .map_err(|err| format!("write {}: {err}", manifest_path.display()))?;
    println!(
        "simdoc-pin: committed engine source {} lock {} toolchain {} ({host})",
        measured.source_sha256, measured.lock_sha256, measured.toolchain
    );
    Ok(())
}

#[cfg(test)]
#[path = "simdoc_pin_tests.rs"]
mod tests;
