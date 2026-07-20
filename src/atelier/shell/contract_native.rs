use serde_json::{Value, json};
use sim_cookbook::fnv1a64_hex;

use crate::atelier::guard::AtelierGuardReport;

const SCHEMA: &str = "sim.atelier.contract-native.v1";
const EVIDENCE_EVENTS: &[&str] = &[
    "contract-native:deck:task-scoped",
    "contract-native:projection:tokens=114",
    "contract-native:grammar:shapegrammar",
    "contract-native:route:cheap-downshift:failed",
    "contract-native:route:escalation-downshift:accepted",
    "contract-native:guard:denials-retained",
];

pub(super) fn report_json(guard: &AtelierGuardReport) -> Value {
    json!({
        "schema": SCHEMA,
        "backend": "contract-native",
        "contract_deck": {
            "cards": 4,
            "complete_cards": 3,
            "partial_cards": 1,
            "diagnostics": [
                "missing authored example preserved with Shape-synthesized fallback"
            ],
        },
        "projection": {
            "token_budget": 160,
            "tokens": 114,
            "included": 3,
            "summary_only": 1,
            "dropped": 1,
            "diagnostics": [
                "contract projection reduced table/entries to summary only under token budget",
                "contract projection dropped unrelated export under token budget"
            ],
        },
        "grammar": {
            "dialect": "shapegrammar",
            "target_codec": "codec:lisp",
            "return_shape": "(list-rest () Any)",
            "strict": true,
        },
        "route_attempts": [
            {
                "target": "cheap-downshift",
                "status": "failed",
                "reason": "terminal output failed codec or Shape check",
            },
            {
                "target": "escalation-downshift",
                "status": "accepted",
                "reason": null,
            },
        ],
        "diagnostics": diagnostics(guard),
        "cassette_hash": cassette_hash(EVIDENCE_EVENTS),
        "guard_denials": guard_denials(),
    })
}

fn diagnostics(guard: &AtelierGuardReport) -> Vec<String> {
    let mut diagnostics = vec![
        "contract-native cache evidence is deterministic and source-free".to_owned(),
        "sim-web-shell serves this cache without issuing model requests".to_owned(),
    ];
    for rule_id in ["meta-workspace-not-source", "remote-policy"] {
        if !guard.rules.iter().any(|rule| rule.id == rule_id) {
            diagnostics.push(format!("guideline firewall rule {rule_id} is unavailable"));
        }
    }
    diagnostics
}

fn guard_denials() -> Value {
    json!([
        {
            "id": "meta-workspace-edit",
            "action": "edit sim-agent-net:.meta-workspace/packages/sim-lib-agent/src/atelier.rs",
            "reason": "edits under .meta-workspace are denied",
            "required_capability": "EditRepo(sim-agent-net)",
        },
        {
            "id": "cross-repo-write",
            "action": "edit sim-web:crates/sim-web-shell/src/atelier.rs",
            "reason": "mission lease is sim-agent-net, not sim-web",
            "required_capability": "EditRepo(sim-web)",
        },
        {
            "id": "github-outward-action",
            "action": "add-github-remote https://github.com/sim-nest/sim-private.git",
            "reason": "adding a GitHub remote is denied",
            "required_capability": "PlanPin",
        }
    ])
}

fn cassette_hash(events: &[&str]) -> String {
    let mut bytes = Vec::new();
    for event in events {
        bytes.extend_from_slice(event.as_bytes());
    }
    format!("fnv1a64:{}", fnv1a64_hex(&bytes))
}
