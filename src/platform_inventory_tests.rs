use super::*;

#[test]
fn adversarial_roles_and_syntax_are_semantic() {
    let mut facts = Vec::new();
    scan_file(
        "sim-x",
        Path::new("."),
        Path::new("crates/a/src/lib.rs"),
        "rust",
        "// std::fs::read is prose\nuse std::net as wire;\n#[cfg(any(test, feature = \"x\"))]\nmod nested { fn x(){ std::fs::read(\"a\"); } }\nextern \"C\" { fn read(); }\nfn call(){ std::process::Command::new(\"x\"); wire::TcpStream::connect(\"x\"); }",
        &mut facts,
    );
    assert_eq!(facts.len(), 4);
    assert!(
        facts
            .iter()
            .any(|f| f.test_member && f.binding_kind == "call")
    );
    assert!(facts.iter().any(|f| f.binding_kind == "abi-declaration"));
    assert!(facts.iter().any(|f| f.binding_kind == "subprocess"));
    assert!(facts.iter().any(|f| f.evidence.contains("aliased")));
}

#[test]
fn foreign_alias_manifest_and_reexport_patterns_are_not_prose_hits() {
    assert!(patterns("kotlin").iter().any(|row| row.0 == "java.io."));
    assert!(
        patterns("manifest")
            .iter()
            .any(|row| row.0 == "target.'cfg(")
    );
    assert!(
        patterns("javascript")
            .iter()
            .any(|row| row.0.contains("require"))
    );
}

#[test]
fn tooling_facts_are_distinct_and_never_product_reachable() {
    let mut facts = Vec::new();
    scan_file(
        "sim-tooling",
        Path::new("."),
        Path::new("src/check.rs"),
        "rust",
        "fn check() { std::fs::read(\"Cargo.toml\"); }",
        &mut facts,
    );
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].role, "tool");
    assert_eq!(facts[0].owner_phase, "resolved");
    assert_eq!(fact_class(facts[0].role), "host-tool");
}
