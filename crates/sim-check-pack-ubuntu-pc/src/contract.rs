// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

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

/// Exact native witness roles the platform owner cannot yet derive.
///
/// Names intentionally match the semantic obligation rather than a caller
/// selected specimen label. A later platform API must provide a typed opaque
/// role for each member before this producer can classify it as present.
/// Empty: the platform derives every C-OP fact from its corpus.
pub const MISSING_PLATFORM_WITNESS_ROLES: [&str; 0] = [];
