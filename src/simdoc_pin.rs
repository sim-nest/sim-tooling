// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The committed identity of the contract engine, and its enforcement.
//!
//! The repository commits the identity it trusts under
//! `[workspace.metadata.sim.encoder]` in its root `Cargo.toml`:
//!
//! ```toml
//! [workspace.metadata.sim.encoder]
//! source_sha256 = "..."
//! lock_sha256 = "..."
//! toolchain = "1.96.0"
//! ```
//!
//! Before xtask runs the engine it verifies, independently of the engine's
//! own code, that the engine's source tree and lock hash to the committed
//! digests, that the tooling pins the committed toolchain, and that `cargo`
//! and `rustc` are that toolchain's binaries as resolved by `rustup` (never
//! whatever comes first on `PATH`). After building, it asks the executable
//! for its embedded identity and refuses unless every field matches. The
//! committed identity changes only through `xtask simdoc-pin`, an explicit
//! command whose diff is reviewed like any other.

use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

const PIN_TABLE: &str = "[workspace.metadata.sim.encoder]";

/// The committed engine identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EncoderPin {
    pub(crate) source_sha256: String,
    pub(crate) lock_sha256: String,
    pub(crate) toolchain: String,
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
        Ok(Self {
            source_sha256: field("source_sha256")?,
            lock_sha256: field("lock_sha256")?,
            toolchain: field("toolchain")?,
        })
    }

    /// Computes the identity of the engine at `simdoc_dir` in `tooling_root`.
    pub(crate) fn measure(tooling_root: &Path, simdoc_dir: &Path) -> Result<Self, String> {
        Ok(Self {
            source_sha256: source_digest(simdoc_dir)?,
            lock_sha256: file_digest(&simdoc_dir.join("Cargo.lock"))?,
            toolchain: pinned_channel(tooling_root)?,
        })
    }

    fn table(&self) -> String {
        format!(
            "{PIN_TABLE}\nsource_sha256 = \"{}\"\nlock_sha256 = \"{}\"\ntoolchain = \"{}\"\n",
            self.source_sha256, self.lock_sha256, self.toolchain
        )
    }

    /// Rewrites the committed identity table in `manifest`.
    pub(crate) fn write_into(&self, manifest: &str) -> String {
        let mut kept = Vec::new();
        let mut skipping = false;
        for line in manifest.lines() {
            let header = line.trim_start().starts_with('[');
            if line.trim() == PIN_TABLE {
                skipping = true;
                continue;
            }
            if skipping && header {
                skipping = false;
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
/// path, a NUL, its length (u64 little-endian), and its bytes, in path order.
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

/// The pinned toolchain's binaries, resolved through `rustup`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Toolchain {
    pub(crate) cargo: PathBuf,
    pub(crate) rustc: PathBuf,
    pub(crate) channel: String,
}

/// Resolves `cargo` and `rustc` of `channel` with `rustup which` and
/// requires both to report exactly that release.
pub(crate) fn resolve_toolchain(rustup: &OsStr, channel: &str) -> Result<Toolchain, String> {
    let which = |tool: &str| -> Result<PathBuf, String> {
        let output = Command::new(rustup)
            .args(["which", "--toolchain", channel, tool])
            .output()
            .map_err(|err| format!("run rustup which {tool}: {err}"))?;
        if !output.status.success() {
            return Err(format!(
                "rustup cannot resolve {tool} for toolchain {channel}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        if !path.is_absolute() {
            return Err(format!("rustup resolved {tool} to a relative path"));
        }
        Ok(path)
    };
    let toolchain = Toolchain {
        cargo: which("cargo")?,
        rustc: which("rustc")?,
        channel: channel.to_owned(),
    };
    for tool in [&toolchain.cargo, &toolchain.rustc] {
        let release = tool_release(tool)?;
        if release != channel {
            return Err(format!(
                "{} reports release {release}, not the pinned {channel}",
                tool.display()
            ));
        }
    }
    Ok(toolchain)
}

fn tool_release(tool: &Path) -> Result<String, String> {
    let output = Command::new(tool)
        .args(["--version", "--verbose"])
        .output()
        .map_err(|err| format!("run {} -vV: {err}", tool.display()))?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("release:"))
        .map(|release| release.trim().to_owned())
        .ok_or_else(|| format!("{} -vV reports no release", tool.display()))
}

/// Requires the executable's reported identity to be exactly the pin.
pub(crate) fn verify_identity(reported: &str, pin: &EncoderPin) -> Result<(), String> {
    let field = |name: &str| {
        reported
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .unwrap_or_default()
    };
    for (name, expected) in [
        ("source_sha256", pin.source_sha256.as_str()),
        ("lock_sha256", pin.lock_sha256.as_str()),
        ("rustc_release", pin.toolchain.as_str()),
        ("cargo_release", pin.toolchain.as_str()),
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
    if measured != *pin {
        return Err(format!(
            "the engine on disk is not the committed one (committed {pin:?}, found {measured:?}); \
             review the change and run `cargo run -p xtask -- simdoc-pin`"
        ));
    }
    Ok(())
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
    let measured = EncoderPin::measure(tooling_root, &tooling_root.join("crates/simdoc"))?;
    if check {
        let committed = EncoderPin::from_manifest(&manifest)?;
        if committed != measured {
            return Err(format!(
                "the committed engine identity is stale (committed {committed:?}, found {measured:?})"
            ));
        }
        println!("simdoc-pin: committed engine identity is current");
        return Ok(());
    }
    fs::write(&manifest_path, measured.write_into(&manifest))
        .map_err(|err| format!("write {}: {err}", manifest_path.display()))?;
    println!(
        "simdoc-pin: committed engine source {} lock {} toolchain {}",
        measured.source_sha256, measured.lock_sha256, measured.toolchain
    );
    Ok(())
}

/// The `rustup` executable launchers resolve toolchains with.
pub(crate) fn rustup() -> OsString {
    OsString::from("rustup")
}

#[cfg(test)]
#[path = "simdoc_pin_tests.rs"]
mod tests;
