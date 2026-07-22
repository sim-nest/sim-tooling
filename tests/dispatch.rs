use std::{fs, path::PathBuf};

#[test]
fn simdoc_extra_flags_reach_simdoc_parser() {
    let args = vec![
        "xtask".to_owned(),
        "simdoc".to_owned(),
        "--not-a-real-flag".to_owned(),
    ];

    let err = xtask::run(args).expect_err("unknown simdoc flag should fail");
    assert!(err.contains("unknown simdoc argument"));
}

#[test]
fn generator_commands_accept_explicit_repo_root() {
    let repo = source_checkout_root().to_string_lossy().into_owned();
    for args in [
        vec![
            "xtask".to_owned(),
            "repo-contract".to_owned(),
            "--check".to_owned(),
            "--repo".to_owned(),
            repo.clone(),
        ],
        vec![
            "xtask".to_owned(),
            "validation-matrix".to_owned(),
            "--check".to_owned(),
            "--repo".to_owned(),
            repo.clone(),
        ],
        vec![
            "xtask".to_owned(),
            "crate-catalog".to_owned(),
            "--check".to_owned(),
            "--repo".to_owned(),
            repo.clone(),
        ],
    ] {
        xtask::run(args).expect("generator command should accept explicit --repo");
    }
}

fn source_checkout_root() -> PathBuf {
    let manifest_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = manifest_root.join("src");
    if let Ok(target) = fs::read_link(&src) {
        let target = if target.is_absolute() {
            target
        } else {
            manifest_root.join(target)
        };
        if let Some(root) = target.parent() {
            return root.to_path_buf();
        }
    }
    manifest_root
}
