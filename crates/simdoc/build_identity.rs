// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Pure identity rules for the simdoc build script, compiled into `build.rs`
//! and into simdoc's own test build so each rule has a falsifying test.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

/// The `channel` pinned by a `rust-toolchain.toml` text.
pub(crate) fn pinned_channel(toolchain_toml: &str) -> Option<String> {
    toolchain_toml.lines().find_map(|line| {
        let value = line
            .trim()
            .strip_prefix("channel")?
            .trim()
            .strip_prefix('=')?;
        Some(value.trim().trim_matches('"').to_owned())
    })
}

/// `(release, commit-hash)` from `<tool> --version --verbose` output.
pub(crate) fn tool_identity(verbose_version: &str) -> Option<(String, String)> {
    let field = |name: &str| {
        verbose_version
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .map(|value| value.trim().to_owned())
    };
    Some((field("release:")?, field("commit-hash:")?))
}

/// Refuses a tool whose release is not the pinned channel.
pub(crate) fn require_pinned(tool: &str, release: &str, pinned: &str) -> Result<(), String> {
    if release == pinned {
        Ok(())
    } else {
        Err(format!(
            "simdoc must be built with the pinned toolchain {pinned}; {tool} is {release}"
        ))
    }
}

/// SHA-256 over the encoder's manifest, lock, build inputs, and `src` tree:
/// every file's repository-relative path, length, and bytes, in path order.
pub(crate) fn source_digest(root: &Path) -> io::Result<String> {
    let mut files = vec![
        PathBuf::from("Cargo.toml"),
        PathBuf::from("Cargo.lock"),
        PathBuf::from("build.rs"),
        PathBuf::from("build_identity.rs"),
    ];
    collect(root, Path::new("src"), &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for relative in files {
        let bytes = fs::read(root.join(&relative))?;
        let name = relative
            .to_str()
            .ok_or_else(|| io::Error::other("simdoc source path is not UTF-8"))?
            .replace(std::path::MAIN_SEPARATOR, "/");
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn collect(root: &Path, relative: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(root.join(relative))? {
        let entry = entry?;
        let path = relative.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(root, &path, files)?;
        } else if kind.is_file() {
            files.push(path);
        } else {
            return Err(io::Error::other(format!(
                "simdoc source {} is not an ordinary file",
                path.display()
            )));
        }
    }
    Ok(())
}

/// The `host:` triple from `<tool> --version --verbose` output.
pub(crate) fn host_triple(verbose_version: &str) -> Option<String> {
    verbose_version
        .lines()
        .find_map(|line| line.strip_prefix("host:"))
        .map(|value| value.trim().to_owned())
}

/// The ordinary files that define a toolchain's identity: the `cargo`,
/// `rustc`, and `rustdoc` executables and every file beneath `lib`,
/// recursively: `librustc_driver`, LLVM, and the whole `rustlib` sysroot
/// (standard-library rlibs, target tools, manifests), everything rustc and
/// rustdoc read to compile. A symlink or any other kind of entry refuses the
/// identity.
pub(crate) fn toolchain_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = ["cargo", "rustc", "rustdoc"]
        .iter()
        .map(|name| Path::new("bin").join(name))
        .collect::<Vec<_>>();
    let mut pending = vec![PathBuf::from("lib")];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(root.join(&dir))? {
            let entry = entry?;
            let relative = dir.join(entry.file_name());
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(relative);
            } else {
                files.push(relative);
            }
        }
    }
    files.sort();
    for relative in &files {
        let metadata = fs::symlink_metadata(root.join(relative))?;
        if !metadata.is_file() {
            return Err(io::Error::other(format!(
                "toolchain file {} is not an ordinary file",
                relative.display()
            )));
        }
    }
    Ok(files)
}

/// SHA-256 over [`toolchain_files`] of the toolchain rooted at `root`: each
/// file's slash-separated path relative to the root, a NUL, its length
/// (u64 little-endian), and its bytes, streamed in path order.
pub(crate) fn toolchain_digest(root: &Path) -> io::Result<String> {
    use std::io::Read;

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    for relative in toolchain_files(root)? {
        let mut file = fs::File::open(root.join(&relative))?;
        let length = file.metadata()?.len();
        hasher.update(
            relative
                .to_str()
                .ok_or_else(|| io::Error::other("toolchain path is not UTF-8"))?
                .replace(std::path::MAIN_SEPARATOR, "/")
                .as_bytes(),
        );
        hasher.update([0]);
        hasher.update(length.to_le_bytes());
        let mut remaining = length;
        while remaining > 0 {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                return Err(io::Error::other("toolchain file shrank while hashed"));
            }
            hasher.update(&buffer[..read]);
            remaining = remaining.saturating_sub(read as u64);
        }
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// The toolchain digest the committed root manifest pins for `host`, from its
/// `[workspace.metadata.sim.encoder.toolchain_sha256]` table.
pub(crate) fn pinned_toolchain_digest(root_manifest: &str, host: &str) -> Option<String> {
    let mut in_table = false;
    for line in root_manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_table = line == "[workspace.metadata.sim.encoder.toolchain_sha256]";
            continue;
        }
        if in_table
            && let Some((key, value)) = line.split_once('=')
            && key.trim().trim_matches('"') == host
        {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn the_pin_and_tool_identities_are_parsed_exactly() {
        assert_eq!(
            pinned_channel("[toolchain]\nchannel = \"1.96.0\"\ncomponents = []\n").as_deref(),
            Some("1.96.0")
        );
        assert_eq!(pinned_channel("[toolchain]\n"), None);
        assert_eq!(
            tool_identity("rustc 1.96.0\nbinary: rustc\ncommit-hash: abc\nrelease: 1.96.0\n"),
            Some(("1.96.0".to_owned(), "abc".to_owned()))
        );
        assert_eq!(tool_identity("rustc 1.96.0\n"), None);
    }

    #[test]
    fn a_toolchain_other_than_the_pin_is_refused() {
        assert!(require_pinned("rustc", "1.96.0", "1.96.0").is_ok());
        let err = require_pinned("rustc", "1.97.1", "1.96.0").unwrap_err();
        assert!(
            err.contains("pinned toolchain 1.96.0; rustc is 1.97.1"),
            "{err}"
        );
    }

    #[test]
    fn the_source_digest_binds_every_source_and_the_lock() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("simdoc-digest-{}-{stamp}", std::process::id()));
        fs::create_dir_all(root.join("src/nested")).unwrap();
        for file in ["Cargo.toml", "Cargo.lock", "build.rs", "build_identity.rs"] {
            fs::write(root.join(file), file).unwrap();
        }
        fs::write(root.join("src/lib.rs"), "lib").unwrap();
        fs::write(root.join("src/nested/mod.rs"), "nested").unwrap();

        let first = source_digest(&root).unwrap();
        assert_eq!(first, source_digest(&root).unwrap());
        fs::write(root.join("src/nested/mod.rs"), "changed").unwrap();
        let second = source_digest(&root).unwrap();
        assert_ne!(first, second);
        fs::write(root.join("Cargo.lock"), "relocked").unwrap();
        assert_ne!(second, source_digest(&root).unwrap());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_toolchain_digest_binds_binaries_and_libraries_and_refuses_symlinks() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("simdoc-toolchain-{}-{stamp}", std::process::id()));
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("lib/rustlib")).unwrap();
        for tool in ["cargo", "rustc", "rustdoc"] {
            fs::write(root.join("bin").join(tool), tool).unwrap();
        }
        fs::write(root.join("bin/cargo-fmt"), "not part of the identity").unwrap();
        fs::write(root.join("lib/librustc_driver-1.so"), "driver").unwrap();
        fs::create_dir_all(root.join("lib/rustlib/x86_64-unknown-linux-gnu/lib")).unwrap();
        fs::write(
            root.join("lib/rustlib/x86_64-unknown-linux-gnu/lib/libstd-1.rlib"),
            "std",
        )
        .unwrap();

        let first = toolchain_digest(&root).unwrap();
        assert_eq!(first, toolchain_digest(&root).unwrap());
        fs::write(root.join("bin/cargo-fmt"), "changed").unwrap();
        assert_eq!(first, toolchain_digest(&root).unwrap());
        for changed in [
            "bin/rustc",
            "lib/librustc_driver-1.so",
            "lib/rustlib/x86_64-unknown-linux-gnu/lib/libstd-1.rlib",
        ] {
            let original = fs::read(root.join(changed)).unwrap();
            fs::write(root.join(changed), "counterfeit").unwrap();
            assert_ne!(first, toolchain_digest(&root).unwrap(), "{changed}");
            fs::write(root.join(changed), original).unwrap();
        }
        fs::write(root.join("lib/libextra.so"), "extra").unwrap();
        assert_ne!(first, toolchain_digest(&root).unwrap());
        fs::remove_file(root.join("lib/libextra.so")).unwrap();
        // A link or special file anywhere in the closure, however deep, refuses.
        std::os::unix::fs::symlink(
            root.join("bin/rustc"),
            root.join("lib/rustlib/x86_64-unknown-linux-gnu/lib/libdeep.rlib"),
        )
        .unwrap();
        assert!(toolchain_digest(&root).is_err());
        fs::remove_file(root.join("lib/rustlib/x86_64-unknown-linux-gnu/lib/libdeep.rlib"))
            .unwrap();
        std::os::unix::fs::symlink(root.join("bin/rustc"), root.join("lib/liblink.so")).unwrap();
        assert!(
            toolchain_digest(&root)
                .unwrap_err()
                .to_string()
                .contains("not an ordinary file")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_pinned_toolchain_digest_is_read_per_host() {
        let manifest = "[workspace.metadata.sim.encoder]\ntoolchain = \"1.96.0\"\n\n\
                        [workspace.metadata.sim.encoder.toolchain_sha256]\n\
                        x86_64-unknown-linux-gnu = \"aa\"\n\"aarch64-apple-darwin\" = \"bb\"\n\n\
                        [dependencies]\nx86_64-unknown-linux-gnu = \"cc\"\n";
        assert_eq!(
            pinned_toolchain_digest(manifest, "x86_64-unknown-linux-gnu").as_deref(),
            Some("aa")
        );
        assert_eq!(
            pinned_toolchain_digest(manifest, "aarch64-apple-darwin").as_deref(),
            Some("bb")
        );
        assert_eq!(pinned_toolchain_digest(manifest, "riscv64"), None);
        assert_eq!(
            host_triple("rustc 1.96.0\nhost: x86_64-unknown-linux-gnu\nrelease: 1.96.0\n")
                .as_deref(),
            Some("x86_64-unknown-linux-gnu")
        );
    }
}
