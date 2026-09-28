// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;

// conformance: a cargo run sees only checksum-verified registry crates in a
// private home; the caller's home never reaches it.

fn scratch(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("cargo-home-{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut out, byte| {
            out.push_str(&format!("{byte:02x}"));
            out
        })
}

/// A real home with one cached crate, its index cache, hostile configuration,
/// and a tampered unpacked source; and the lock naming the crate.
fn real_home(bytes: &[u8]) -> (PathBuf, String) {
    let real = scratch("real");
    let cache = real.join("registry/cache/index.crates.io-abc");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("dep-1.0.0.crate"), bytes).unwrap();
    fs::create_dir_all(real.join("registry/index/index.crates.io-abc/.cache")).unwrap();
    fs::write(
        real.join("registry/index/index.crates.io-abc/.cache/de"),
        "idx",
    )
    .unwrap();
    fs::create_dir_all(real.join("registry/src/index.crates.io-abc/dep-1.0.0")).unwrap();
    fs::write(
        real.join("registry/src/index.crates.io-abc/dep-1.0.0/lib.rs"),
        "evil",
    )
    .unwrap();
    fs::write(real.join("config.toml"), "paths = [\"/evil\"]\n").unwrap();
    fs::write(real.join("credentials.toml"), "token").unwrap();
    let lock = format!(
        "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n\n[[package]]\n\
         name = \"dep\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\
         checksum = \"{}\"\n",
        sha(bytes)
    );
    (real, lock)
}

#[test]
fn only_verified_registry_crates_and_their_index_enter_the_private_home() {
    let (real, lock) = real_home(b"crate bytes");
    let home = hermetic_home(&real, Some(&lock), "test").unwrap();
    assert_eq!(
        fs::read(home.join("registry/cache/index.crates.io-abc/dep-1.0.0.crate")).unwrap(),
        b"crate bytes"
    );
    assert!(
        home.join("registry/index/index.crates.io-abc/.cache/de")
            .is_file()
    );
    // Nothing else of the real home is visible.
    for hidden in ["config.toml", "credentials.toml", "registry/src"] {
        assert!(!home.join(hidden).exists(), "{hidden}");
    }
    let empty = hermetic_home(&real, None, "empty").unwrap();
    assert_eq!(fs::read_dir(&empty).unwrap().count(), 0);
}

#[test]
fn a_cached_crate_that_is_not_the_locked_checksum_refuses() {
    let (real, lock) = real_home(b"crate bytes");
    fs::write(
        real.join("registry/cache/index.crates.io-abc/dep-1.0.0.crate"),
        "tampered",
    )
    .unwrap();
    let err = hermetic_home(&real, Some(&lock), "test").unwrap_err();
    assert!(err.contains("does not match the checksum"), "{err}");
}

#[test]
fn a_locked_registry_package_absent_from_the_cache_refuses() {
    // Not silently producing a private home that is simply missing the
    // package: an absent archive must fail closed here, the same as a
    // tampered one, rather than let a caller who forgot --offline reach the
    // network for it instead.
    let (real, lock) = real_home(b"crate bytes");
    fs::remove_file(real.join("registry/cache/index.crates.io-abc/dep-1.0.0.crate")).unwrap();
    let err = hermetic_home(&real, Some(&lock), "test").unwrap_err();
    assert!(
        err.contains("dep-1.0.0.crate") && err.contains("not in the Cargo cache"),
        "{err}"
    );
}

#[test]
fn a_git_dependency_or_a_missing_checksum_refuses() {
    let real = scratch("git");
    for lock in [
        "[[package]]\nname = \"g\"\nversion = \"1\"\nsource = \"git+https://example.invalid/g\"\n",
        "[[package]]\nname = \"r\"\nversion = \"1\"\nsource = \"registry+https://x\"\n",
    ] {
        assert!(hermetic_home(&real, Some(lock), "t").is_err(), "{lock}");
    }
}

#[test]
fn the_real_home_must_be_an_absolute_path() {
    let abs = |value: &str| Some(OsString::from(value));
    assert_eq!(
        real_cargo_home(abs("/x/.cargo"), abs("/y"), abs("/z")).unwrap(),
        PathBuf::from("/x/.cargo")
    );
    assert_eq!(
        real_cargo_home(None, None, abs("/z")).unwrap(),
        PathBuf::from("/z/.cargo")
    );
    for (explicit, cargo_home, home) in [
        (abs(""), None, None),
        (None, abs(".x"), abs("/z")),
        (abs("relative/.cargo"), None, None),
        (None, None, None),
    ] {
        assert!(real_cargo_home(explicit, cargo_home, home).is_err());
    }
}

#[test]
fn private_directories_are_owner_only_and_dead_ones_are_swept() {
    let dir = private_dir("live").unwrap();
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode_bits() & 0o077,
        0
    );
    let dead = std::env::temp_dir().join(format!("{PREFIX}4194300-1-dead"));
    fs::create_dir_all(dead.join("x")).unwrap();
    let _ = private_dir("sweeper").unwrap();
    assert!(
        !dead.exists(),
        "the private directory of a dead process is removed"
    );
    assert!(dir.exists());
}

trait ModeBits {
    fn mode_bits(&self) -> u32;
}

impl ModeBits for fs::Permissions {
    fn mode_bits(&self) -> u32 {
        std::os::unix::fs::PermissionsExt::mode(self)
    }
}

#[test]
fn lock_values_that_are_paths_or_malformed_refuse_and_every_source_kind_is_read() {
    let real = scratch("names");
    for (name, version) in [
        ("../../../../tmp/a", "1.0.0"),
        ("a/b", "1.0.0"),
        (".hidden", "1.0.0"),
        ("dep", "../1"),
        ("dep", "1/2"),
        ("", "1.0.0"),
    ] {
        let lock = format!(
            "[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n\
             source = \"registry+https://x\"\nchecksum = \"{}\"\n",
            "a".repeat(64)
        );
        assert!(
            hermetic_home(&real, Some(&lock), "n").is_err(),
            "{name} {version}"
        );
    }
    let short = "[[package]]\nname = \"dep\"\nversion = \"1.0.0\"\nsource = \"registry+https://x\"\nchecksum = \"abc\"\n";
    assert!(hermetic_home(&real, Some(short), "n").is_err());
    // A git source written without spaces, or after a patch table header,
    // is still read as what it is.
    for lock in [
        "[[package]]\nname=\"g\"\nversion=\"1\"\nsource=\"git+https://example.invalid/g\"\n",
        "[[package]]\nname = \"a\"\nversion = \"1\"\n\n[[patch.unused]]\nname = \"g\"\nversion = \"1\"\nsource = \"git+https://x\"\n",
    ] {
        let parsed = registry_packages(lock);
        assert!(parsed.is_err() || parsed.unwrap().is_empty(), "{lock}");
    }
    let err = registry_packages(
        "[[package]]\nname=\"g\"\nversion=\"1\"\nsource=\"git+https://example.invalid/g\"\n",
    )
    .err()
    .unwrap();
    assert!(err.contains("git dependency"));
}

#[test]
fn a_git_source_written_with_an_alternate_valid_toml_string_form_still_refuses() {
    // A hand-rolled line reader that only trims double quotes would leave a
    // single-quoted literal string's own quote characters attached, so
    // `source.starts_with("git+")` would see `'git+...` instead and miss it
    // entirely -- silently dropping the row rather than refusing the run.
    let lock = "[[package]]\nname = 'g'\nversion = '1'\nsource = 'git+https://example.invalid/g'\n";
    let err = registry_packages(lock)
        .err()
        .unwrap_or_else(|| panic!("a single-quoted git dependency must refuse, not vanish"));
    assert!(err.contains("git dependency"), "{err}");
}

#[test]
fn a_symlink_in_the_index_cache_refuses() {
    let (real, lock) = real_home(b"crate bytes");
    std::os::unix::fs::symlink("/etc", real.join("registry/index/index.crates.io-abc/link"))
        .unwrap();
    assert!(
        hermetic_home(&real, Some(&lock), "n")
            .unwrap_err()
            .contains("symlink")
    );
}

#[test]
fn a_scratch_parent_others_can_rename_in_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let open = scratch("open-parent");
    fs::set_permissions(&open, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(require_safe_parent(&open).is_err());
    fs::set_permissions(&open, fs::Permissions::from_mode(0o1777)).unwrap();
    require_safe_parent(&open).unwrap();
    fs::set_permissions(&open, fs::Permissions::from_mode(0o700)).unwrap();
    require_safe_parent(&open).unwrap();
}
