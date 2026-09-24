//! Shared fixtures for the exact check-pack currentness boundary.

pub(crate) fn operation_evidence(variant: &str) -> String {
    let mut facts = vec![
        "operation.local-cancellation-terminates-group=true",
        "operation.local-command-id-binds-complete-spec=true",
        "operation.local-command-is-installed-and-allowlisted=true",
        "operation.local-descendants-zero=true",
        "operation.local-docs-command-exact=true",
        "operation.local-environment-sealed=true",
        "operation.local-formatter-mutation-observed=true",
        "operation.local-independent-postcondition=true",
        "operation.local-later-owner-gates-reachable=true",
        "operation.local-manifest-script-bytes-exact=true",
        "operation.local-model-interpolation-refused=true",
        "operation.local-network-absent=true",
        "operation.local-network-grant-separate=true",
        "operation.local-port-is-portable=true",
        "operation.local-release-credentials-absent=true",
        "operation.local-scratch-zero=true",
        "operation.local-signal-escalation-bounded=true",
        "operation.local-test-failure-observed=true",
        "operation.local-test-success-observed=true",
        "operation.local-timeout-terminates-group=true",
        "operation.local-validation-command-exact=true",
        "operation.local-writable-roots-confined=true",
    ];
    let variant = format!("subject.variant={variant}");
    facts.push(&variant);
    facts.sort_unstable();
    facts.into_iter().map(|fact| format!("{fact}\n")).collect()
}
