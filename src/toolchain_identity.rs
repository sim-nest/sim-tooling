// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a toolchain is, by content.
//!
//! The same rules, byte for byte, as the encoder's build script
//! (`crates/simdoc/build_identity.rs`): the launcher computes a toolchain's
//! digest here, the encoder's build computes it there, and the encoder
//! refuses to build unless its own result equals the digest this repository
//! commits, so the two implementations cannot silently drift apart.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

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

/// Refuses when a Cargo configuration file exists where Cargo would read it
/// for a run in any of `places`: `config` or `config.toml` in `.cargo` of the
/// place or any ancestor. (Cargo's own home is private and holds no
/// configuration: see `crate::cargo_home`.) Same rule as the engine's own
/// (`crates/simdoc/src/tools.rs`).
pub(crate) fn require_no_cargo_config(places: &[&Path]) -> Result<(), String> {
    for place in places {
        let place = place.canonicalize().unwrap_or_else(|_| place.to_path_buf());
        for dir in place.ancestors() {
            for name in ["config", "config.toml"] {
                let file = dir.join(".cargo").join(name);
                if fs::symlink_metadata(&file).is_ok() {
                    return Err(format!(
                        "refused: Cargo configuration {} would apply; it can replace sources or \
                         dependencies and inject flags, linkers, or environment, and no identity \
                         binds it. Remove it or run from a tree it does not cover",
                        file.display()
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn identities_are_parsed_exactly() {
        assert_eq!(
            tool_identity("rustc 1.96.0\nbinary: rustc\ncommit-hash: abc\nrelease: 1.96.0\n"),
            Some(("1.96.0".to_owned(), "abc".to_owned()))
        );
        assert_eq!(tool_identity("rustc 1.96.0\n"), None);
        assert_eq!(
            host_triple("rustc 1.96.0\nhost: x86_64-unknown-linux-gnu\n").as_deref(),
            Some("x86_64-unknown-linux-gnu")
        );
    }

    #[test]
    fn the_toolchain_digest_binds_binaries_and_libraries_and_refuses_symlinks() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("xtask-toolchain-{}-{stamp}", std::process::id()));
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
}
