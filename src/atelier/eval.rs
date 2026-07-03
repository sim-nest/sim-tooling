//! Deterministic Atelier self-hosting scenario fixtures.

use serde_json::{Value, json};

const SCHEMA: &str = "sim.atelier.self-hosting-scenarios.v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AtelierEvalScenario {
    /// Stable scenario id.
    pub(super) id: &'static str,
    /// Runner mode, either fake or cassette.
    pub(super) runner_mode: &'static str,
    /// Whether a live model participates.
    pub(super) live_model: bool,
    /// Whether network access participates.
    pub(super) network: bool,
    /// Evidence families covered by the scenario.
    pub(super) evidence: &'static [&'static str],
    /// Agent roles exercised by the scenario.
    pub(super) roles: &'static [&'static str],
    /// Deterministic cassette events.
    pub(super) cassette_events: &'static [&'static str],
    /// Expected stable hash of `cassette_events`.
    pub(super) cassette_hash: &'static str,
}

/// Returns the scenario fixture as shell-cache JSON.
pub(super) fn scenario_json() -> Value {
    let scenarios = scenarios();
    let diagnostics = validate_scenarios(&scenarios);
    json!({
        "schema": SCHEMA,
        "diagnostics": diagnostics,
        "scenarios": scenarios.iter().map(scenario_row_json).collect::<Vec<_>>(),
    })
}

/// Returns all deterministic Atelier self-hosting scenarios.
pub(super) fn scenarios() -> Vec<AtelierEvalScenario> {
    vec![
        AtelierEvalScenario {
            id: "atelier-radar-standard-crate",
            runner_mode: "fake",
            live_model: false,
            network: false,
            evidence: &["radar", "ranked-hints", "confidence"],
            roles: &["cartographer"],
            cassette_events: &[
                "fake-runner:load-index:sim-kernel",
                "radar-query:standard-crate-operations",
                "rank:hints:3",
                "explain:confidence:0.91",
            ],
            cassette_hash: "fnv1a64:7f9f5b6c744aa528",
        },
        AtelierEvalScenario {
            id: "atelier-runtime-operation",
            runner_mode: "fake",
            live_model: false,
            network: false,
            evidence: &[
                "runtime-operation",
                "rustdoc",
                "codec-prism",
                "generated-docs",
                "validation",
                "pin-plan",
            ],
            roles: &["editor", "validator", "docs-agent", "pin-agent"],
            cassette_events: &[
                "fake-runner:open-runtime-op-descriptor",
                "editor:add-operation:sample-op",
                "codec-prism:lisp-json-roundtrip",
                "validate:meta-check",
                "pin-plan:sim-runtime",
            ],
            cassette_hash: "fnv1a64:8a3282f3c7ef34b2",
        },
        AtelierEvalScenario {
            id: "atelier-codec-roundtrip",
            runner_mode: "fake",
            live_model: false,
            network: false,
            evidence: &["codec-prism", "roundtrip", "semantic-id", "simdoc"],
            roles: &["editor", "validator"],
            cassette_events: &[
                "fake-runner:load-codec-fixture",
                "codec-prism:lisp-json-algol-roundtrip",
                "assert:semantic-id-stable",
                "docs:simdoc-check",
            ],
            cassette_hash: "fnv1a64:32be093d6f32315a",
        },
        AtelierEvalScenario {
            id: "atelier-guideline-firewall",
            runner_mode: "fake",
            live_model: false,
            network: false,
            evidence: &["guideline-firewall", "rule-evidence", "refused-capability"],
            roles: &["guard", "reviewer"],
            cassette_events: &[
                "fake-runner:load-bad-fixture",
                "guard:present-tense-public-docs",
                "refuse:EditRepo(sim-web)",
                "evidence:past-tense-wording",
            ],
            cassette_hash: "fnv1a64:768ad342812bdd9a",
        },
        AtelierEvalScenario {
            id: "atelier-change-capsule",
            runner_mode: "cassette",
            live_model: false,
            network: false,
            evidence: &[
                "change-capsule",
                "validation",
                "docs",
                "pin-plan",
                "human-gate",
                "replay-hash",
            ],
            roles: &[
                "cartographer",
                "editor",
                "guard",
                "validator",
                "docs-agent",
                "pin-agent",
                "reviewer",
                "human-gate",
            ],
            cassette_events: &[
                "cassette-runner:cartographer-scope",
                "fake-runner:editor-patch",
                "fake-runner:guard-approve",
                "process:validator-pass",
                "process:docs-agent-pass",
                "fake-runner:pin-agent-plan",
                "fake-runner:reviewer-summary",
                "human-gate:approved",
                "replay:hash-match",
            ],
            cassette_hash: "fnv1a64:5ec7c4222478f8f1",
        },
    ]
}

/// Validate runner, evidence, and cassette-hash invariants.
pub(super) fn validate_scenarios(scenarios: &[AtelierEvalScenario]) -> Vec<String> {
    let mut failures = Vec::new();
    for scenario in scenarios {
        if scenario.live_model {
            failures.push(format!("{} uses a live model", scenario.id));
        }
        if scenario.network {
            failures.push(format!("{} uses network access", scenario.id));
        }
        if scenario.runner_mode != "fake" && scenario.runner_mode != "cassette" {
            failures.push(format!("{} uses unsupported runner", scenario.id));
        }
        if scenario.evidence.is_empty() {
            failures.push(format!("{} has no evidence", scenario.id));
        }
        let actual_hash = cassette_content_hash(scenario.cassette_events);
        if actual_hash != scenario.cassette_hash {
            failures.push(format!(
                "{} cassette hash mismatch: expected {}, got {}",
                scenario.id, scenario.cassette_hash, actual_hash
            ));
        }
    }
    failures
}

fn scenario_row_json(scenario: &AtelierEvalScenario) -> Value {
    json!({
        "id": scenario.id,
        "runner_mode": scenario.runner_mode,
        "live_model": scenario.live_model,
        "network": scenario.network,
        "evidence": scenario.evidence,
        "roles": scenario.roles,
        "cassette_events": scenario.cassette_events,
        "cassette_hash": scenario.cassette_hash,
    })
}

fn cassette_content_hash(events: &[&str]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for event in events {
        for byte in event.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("fnv1a64:{hash:016x}")
}
