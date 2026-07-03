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
