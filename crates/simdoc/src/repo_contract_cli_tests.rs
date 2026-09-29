// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use sim_codec_index::{IndexCodec, IndexForm};

use super::*;

// conformance: the emission interface is exclusive, bounded, atomic, and side-effect free.

#[test]
fn emit_mode_is_exclusive_bounded_and_repeatable() {
    let fixture = crate::test_fixture::FixtureRepo::nested("repo-contract-options");
    let repo = fixture.path().to_string_lossy().into_owned();
    let out = temp_root("repo-contract-options");
    let args = |tail: &[&str]| {
        let mut args = vec!["simdoc".to_owned(), "repo-contract".to_owned()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        args.extend(["--repo".to_owned(), repo.clone()]);
        args
    };
    let parsed = parse_options(&args(&[
        "--emit",
        "sim-index-fragment.sx",
        "--emit",
        "sim-index-fragment.claims.sx",
        "--out-dir",
        out.to_str().unwrap(),
    ]))
    .unwrap();
    assert_eq!(parsed.emission.unwrap().names.len(), 2);
    assert!(
        parse_options(&args(&[
            "--emit",
            "unknown",
            "--out-dir",
            out.to_str().unwrap()
        ]))
        .unwrap_err()
        .contains("unknown repo-contract artifact")
    );
    assert!(
        parse_options(&args(&["--emit", "sim-index-fragment.sx"]))
            .unwrap_err()
            .contains("requires --out-dir")
    );
    assert!(
        parse_options(&args(&[
            "--check",
            "--emit",
            "sim-index-fragment.sx",
            "--out-dir",
            out.to_str().unwrap()
        ]))
        .unwrap_err()
        .contains("mutually exclusive")
    );
    assert!(
        parse_options(&args(&[
            "--emit",
            "sim-index-fragment.sx",
            "--emit",
            "sim-index-fragment.sx",
            "--out-dir",
            out.to_str().unwrap()
        ]))
        .unwrap_err()
        .contains("duplicate")
    );
}

#[test]
fn emit_uses_canonical_fragment_and_leaves_repository_untouched() {
    let fixture = crate::test_fixture::FixtureRepo::nested("repo-contract-emit");
    let repo = fixture.path().to_path_buf();
    let before = git_output(&repo, &["status", "--porcelain"]).unwrap();
    let expected = contract_artifacts(&repo).unwrap();
    let out = temp_root("repo-contract-emit");
    let names = vec![
        "sim-index-fragment.sx".to_owned(),
        "sim-index-fragment.claims.sx".to_owned(),
    ];
    emit_contract_artifacts(&repo, &names, &out).unwrap();
    assert_eq!(
        fs::read_to_string(out.join("sim-index-fragment.sx")).unwrap(),
        expected.files["sim-index-fragment.sx"]
    );
    let claims = fs::read_to_string(out.join("sim-index-fragment.claims.sx")).unwrap();
    let doc = IndexCodec
        .decode_fragment(IndexForm::Sx, &expected.files["sim-index-fragment.sx"])
        .unwrap();
    let row_count = doc.inventory().1.len();
    assert!(claims.contains(&format!("[row-count {row_count}]")));
    assert_eq!(claims.matches("(primary ").count(), row_count);
    assert_eq!(
        git_output(&repo, &["status", "--porcelain"]).unwrap(),
        before
    );
}

#[cfg(unix)]
#[test]
fn emit_rejects_output_symlink_escapes_and_interrupted_stage() {
    use std::os::unix::fs::symlink;
    let root = temp_root("repo-contract-symlink");
    let real = root.join("real");
    fs::create_dir(&real).unwrap();
    let link = root.join("link");
    symlink(&real, &link).unwrap();
    assert!(
        validate_preopened_directory(&link)
            .unwrap_err()
            .contains("preopened real directory")
    );

    let name = "sim-index-fragment.sx";
    let stage = real.join(format!(".{name}.sim-stage-{}", std::process::id()));
    fs::write(&stage, "interrupted").unwrap();
    fs::write(real.join(name), "old").unwrap();
    assert!(
        atomic_write(&real, name, b"new")
            .unwrap_err()
            .contains("atomic output stage")
    );
    assert_eq!(fs::read_to_string(real.join(name)).unwrap(), "old");
}

fn git_output(repo: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn temp_root(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("{name}-{}-{stamp}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}
