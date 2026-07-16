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
    for args in [
        vec!["xtask", "repo-contract", "--check", "--repo", "."],
        vec!["xtask", "validation-matrix", "--check", "--repo", "."],
        vec!["xtask", "crate-catalog", "--check", "--repo", "."],
    ] {
        let args = args.into_iter().map(str::to_owned).collect();
        xtask::run(args).expect("generator command should accept --repo .");
    }
}
