# sim-check-pack

`sim-check-pack` is the typed native invocation adapter between the public SIM
conformance packs and a live checker owner. It executes the exact native call
described by the native checker binding and returns the opaque owner-qualified
result; it does not own proof storage, revocation state, or process execution.
Every frozen fact must be paired with one independently produced support-member
identity. The resulting typed support and provenance bind that complete
definition, both exact resolver/build identities, and the native invocation.
The returned comparison projection exposes generic content identities without
allowing those values to be promoted into semantic roles.

`prepare_operation_local` is the authority-free operator-bootstrap seam. It
runs the same pure pack evaluation and binds the same evidence, support,
invocation, build graph, bootstrap grade and policy that live issuance later
consumes. The prepared value contains no owner handle, currentness decision,
revocation observation or receipt. `invoke_operation_local` is exactly
preparation followed by issuance through the SDK owner's live weak handle, so a
bootstrap producer can predict the expected result without minting a fake
owner or maintaining a second checker implementation.

The adapter and checker consume the same explicit resolver-lock selection when
embedded in another workspace: set `SIM_CONFORMANCE_PACKS_LOCK_FILE` to an
absolute regular file and `SIM_CONFORMANCE_PACKS_LOCK_SHA256` to its lowercase
digest. Supplying only one value or a mismatching digest fails the build before
code generation. A packaged owner lock remains the standalone fallback.

The maintenance check-pack command lives in the separate checker workspace,
not `xtask`: `cargo run --manifest-path crates/Cargo.toml -p
sim-check-pack-xtask -- check-pack ...` (`xtask check-pack` itself refuses
and points here). It cannot substitute for this native call or serialize
live owner authority.
