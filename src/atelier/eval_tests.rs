use serde_json::Value;

use super::eval::{scenario_json, scenarios, validate_scenarios};

#[test]
fn self_hosting_scenarios_are_offline_and_hash_checked() {
    let scenarios = scenarios();
    assert_eq!(scenarios.len(), 5);
    assert!(validate_scenarios(&scenarios).is_empty());
    assert!(scenarios.iter().all(|scenario| !scenario.live_model));
    assert!(scenarios.iter().all(|scenario| !scenario.network));
}

#[test]
fn scenario_json_contains_capsule_and_prism_evidence() {
    let report = scenario_json();
    assert_eq!(
        report["schema"],
        Value::String("sim.atelier.self-hosting-scenarios.v1".to_owned())
    );
    let scenarios = report["scenarios"].as_array().unwrap();
    assert!(
        scenarios
            .iter()
            .any(|scenario| scenario["id"] == "atelier-change-capsule")
    );
    assert!(scenarios.iter().any(|scenario| {
        scenario["evidence"]
            .as_array()
            .unwrap()
            .contains(&Value::String("codec-prism".to_owned()))
    }));
}
