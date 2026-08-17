use std::collections::BTreeSet;

use serde_json::Value;

const FIXTURE: &str = include_str!("../tests/fixtures/index9_landed_contract.json");

#[test]
fn index9_fixture_freezes_the_single_landed_contract_path() {
    let fixture: Value = serde_json::from_str(FIXTURE).expect("valid landed-contract fixture");
    assert_eq!(fixture["schema"], "sim.index-landed-contract-fixture/v1");

    let owners = fixture["owners"].as_object().expect("owner map");
    assert_eq!(owners.len(), 8, "every reused owner is explicit");
    let selected = owners
        .values()
        .map(|owner| owner.as_str().expect("owner path"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        selected.len(),
        owners.len(),
        "a second parser, report, or classifier must not alias an existing owner"
    );
    assert!(
        owners["anchor_parser"]
            .as_str()
            .unwrap()
            .contains("syn::parse_file")
    );
    assert!(
        owners["report_parser"]
            .as_str()
            .unwrap()
            .contains("index_overlap_report")
    );
    assert!(
        owners["classifier"]
            .as_str()
            .unwrap()
            .contains("index_overlap.rs::overlap_findings")
    );

    assert_eq!(fixture["report_schema"]["id"], "sim.overlap-report/v1");
    assert_eq!(
        fixture["classification_vocabulary"]["authored_exceptions"],
        serde_json::json!(["keep", "delegated"])
    );
    assert_eq!(
        fixture["scan_bounds"]["anchor_visibility"],
        "public items only; test source excluded"
    );

    let families = fixture["source_fixtures"]
        .as_array()
        .expect("source fixtures");
    let actual = families
        .iter()
        .map(|family| family["family"].as_str().expect("family"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual,
        BTreeSet::from([
            "admission-envelope",
            "guest-class",
            "guest-function",
            "managed-payload"
        ])
    );
    for family in families {
        assert!(
            family["declarations"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        );
    }
    assert_eq!(
        families
            .iter()
            .find(|family| family["family"] == "guest-class")
            .unwrap()["historical_expectation"],
        "the landed public survivor plus eliminated private guest class records remain a protocol-role family; do not invent replacement declarations"
    );
}
