//! Closed fact inventory for the exact C-OP contract.

pub(crate) const FACTS: [&str; 22] = [
    "operation.local-port-is-portable",
    "operation.local-command-is-installed-and-allowlisted",
    "operation.local-command-id-binds-complete-spec",
    "operation.local-manifest-script-bytes-exact",
    "operation.local-model-interpolation-refused",
    "operation.local-environment-sealed",
    "operation.local-writable-roots-confined",
    "operation.local-network-absent",
    "operation.local-network-grant-separate",
    "operation.local-release-credentials-absent",
    "operation.local-formatter-mutation-observed",
    "operation.local-test-success-observed",
    "operation.local-test-failure-observed",
    "operation.local-validation-command-exact",
    "operation.local-docs-command-exact",
    "operation.local-timeout-terminates-group",
    "operation.local-cancellation-terminates-group",
    "operation.local-signal-escalation-bounded",
    "operation.local-descendants-zero",
    "operation.local-scratch-zero",
    "operation.local-independent-postcondition",
    "operation.local-later-owner-gates-reachable",
];

/// Exact native witness roles still absent after the sound j/k baseline.
///
/// Names intentionally match the semantic obligation rather than a caller
/// selected specimen label. A later platform API must provide a typed opaque
/// role for each member before this producer can classify it as present.
pub const MISSING_PLATFORM_WITNESS_ROLES: [&str; 9] = [
    "manifest-script-bytes-exact",
    "model-interpolation-refused",
    "formatter-mutation-observed",
    "validation-command-exact",
    "docs-command-exact",
    "timeout-terminates-group",
    "cancellation-terminates-group",
    "signal-escalation-bounded",
    "later-owner-gates-reachable",
];
