# Benchmark Dependency Gate

This note freezes the BENCH2.00 inventory and the placement gate for the
benchmark product. `sim-tooling` is the runner owner and
`sim-lib-numbers-stats` is the statistics owner. The direct manifest edge is
intentional; tooling must call the statistics crate rather than copy formulas.

## Reuse ledger

| Need | Existing anchor | Disposition |
|---|---|---|
| Mean and variance | `sim-numbers/crates/sim-lib-numbers-stats/src/implementation.rs` | Reuse. |
| Exact and bounded quantiles | `sim-numbers/crates/sim-lib-numbers-stats/src/quantile.rs` | Reuse and extend only in the statistics owner. |
| Runner and command conventions | `sim-tooling/src/main.rs` and `sim-tooling/src/lib.rs` | Extend the existing tooling executable. |
| Canonical artifact encoding | `sim-codec-json`, already in the tooling manifest | Reuse; no benchmark-only codec. |
| Content identity | `sim-agent-net/crates/sim-lib-stream-fabric/src/content_key.rs` | Reuse the canonical content-key semantics; do not invent an unrelated digest contract. |
| Table/Dir artifacts | `sim-foundation/crates/sim-table-core` and the Index Table/Dir route | Compose the existing storage surface. |
| Host inventory and release probes | `sim-private/test-hosts.toml` and `simctl hosts` | Reuse through the code-free control-plane adapter; no second host list. |
| Process bounds | `sim-runtime/crates/sim-lib-exec` | Compose its bounded host-exec behavior; benchmark policy remains in tooling. |
| Evidence records | existing SIM receipt/report conventions | Compose rather than introduce a second evidence vocabulary. |
| Index discovery | `simctl index find` and `simctl index route` | Reuse. No benchmark route exists yet; later implementation must author one. |

The manifest-listed constellation contains no `benches/` directory, Cargo
`[[bench]]` target, Criterion dependency or Criterion harness, and no shell
benchmark script. The one current general benchmark runner is
`sim-codecs/crates/sim-codec-compare`: `speed.rs` runs wall-clock encode/decode
samples and computes a local median, while `report.rs` computes a local mean and
the report binary formats the result. Both formulas are runner duplication to
replace with the statistics owner. Timed tensor assertions in
`sim-lib-numbers-tensor-f64` and `sim-lib-numbers-tensor-linalg`, and timed
acceptance/timeout tests elsewhere, are unrelated microtests rather than reusable
benchmark runners. Domain means and variances (signal preprocessing, audio
analysis, loudness, fitting, and physical reduction) encode domain semantics and
are not benchmark-summary duplication.

## Reconciled runner inventory

| Inventory row | Classification | Ownership disposition |
|---|---|---|
| `sim-tooling/src/bench/` and `xtask bench` | canonical runner | Sole sampling, comparison, report, and policy owner. |
| `sim-codec-compare/src/speed.rs` | legacy workload adapter | Frozen as the pre-product inventory row; it may define codec work, but no new runner or summary implementation may copy it. Migration requires the sim-codecs owner envelope. |
| `sim-codec-compare/src/report.rs` | legacy presentation adapter | Table rendering is presentation; its local summary formulas are non-canonical and must not be reused. |
| timed tensor assertions | correctness workload | Remain tests; elapsed time is an assertion input, not durable benchmark evidence. |
| acceptance and timeout tests | bounded behavior workload | Remain tests; deadlines classify behavior and do not implement benchmark sampling. |
| domain reductions in signal, audio, fitting, and physics code | domain statistic | Retained because their means and variances are product semantics, not benchmark summaries. |
| external dependencies' Criterion harnesses | upstream external harness | Excluded from SIM ownership; vendored/registry source is never a constellation implementation. |

The Index overlap board now raises public `mean`, `median`, `variance`, robust
benchmark-summary, or benchmark-runner declarations outside their canonical
subjects. A feature may expose an adapter only through an explicit checked
`reuses`, `composes`, or `delegates-to` path to the canonical owner. Workload
definitions are deliberately silent because they neither sample nor summarize.

## Dependency proof and measured cost

Cargo accepts no cyclic package graph. The standalone proof is:

```text
cargo metadata --locked --manifest-path ../sim-tooling/Cargo.toml --format-version 1
cargo tree --locked --manifest-path ../sim-tooling/Cargo.toml -e features
```

The meta-workspace proof is the reverse-edge check followed by regeneration:

```text
cargo tree --locked --manifest-path .meta-workspace/Cargo.toml \
  -p sim-lib-numbers-stats --invert --edges normal
sh bin/simctl meta-build
cargo tree --locked --manifest-path .meta-workspace/Cargo.toml -p xtask -e features
```

The standalone metadata and tree succeeded with the direct path ending
`xtask -> sim-lib-numbers-stats -> sim-lib-numbers-core`. The current exact
meta-workspace reverse tree contains only `sim-lib-numbers-stats` itself, proving
that the pre-edge graph has no path from the statistics crate back to `xtask`;
adding the accepted standalone edge therefore cannot close a cycle. Regeneration
is presently blocked before graph construction by unrelated accumulated
`repos.toml` pin/source-path drift in other roadmap repositories. The terminal
convergence gate must regenerate the workspace and repeat the final tree command
after those owned pins and manifests materialize.

On 2026-08-15, two clean dev builds used separate `mktemp` target directories
and a warm local Cargo source cache:

```text
/usr/bin/time -f '%e %M' cargo build --locked --manifest-path ../sim-tooling/Cargo.toml --target-dir <fresh-target>
```

The baseline was 10.90 s and 23,500 KiB maximum RSS; the direct-edge build was
11.01 s and 23,476 KiB. The measured delta is +0.11 s (+1.0%) and -24 KiB RSS
(measurement noise). The resolved tree grows from 78 to 80 unique rendered
package rows. Feature-tree comparison adds only the default features of
`sim-lib-numbers-stats` and `sim-lib-numbers-core`; their transitive dependencies
were already unified by tooling's existing graph. Reconsider placement if a
three-run median clean build regresses by more than 10% or 2.0 s, whichever is
larger, or if a non-default numbers feature becomes unified. This initial
paired measurement is the baseline; later gates use the stated three-run rule.

## Schema and version boundary

Benchmark artifacts use schema family `sim.bench` with integer revision `1`.
Every persisted record carries both fields. Additive optional fields retain the
revision; changing meaning, units, required fields, identity inputs, or
comparison semantics requires a new revision and an explicit decoder/migration
path. Unknown revisions fail closed. The detailed record shapes belong to
BENCH2.02 and may not weaken this version rule.

This product does not provide a profiler, production telemetry, continuous
monitoring, or a runtime clock. It does not put sampling clocks into product
runtime crates, interpret unlike-host results as an acceptance comparison, or
replace domain-specific correctness and acceptance tests. Benchmark time comes
only from an explicit tooling-owned monotonic-clock adapter.
