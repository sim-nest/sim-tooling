# sim-check-pack

In one line: Invoke an exact native SIM conformance pack without serializing the checker owner's authority.

## What it gives you

`sim-check-pack` is the narrow code adapter between a boot-selected checker
owner and the public conformance-pack implementation. It binds the native call,
the sealed subject and input closure, every independently identified support
member, both exact build graphs, and bootstrap grade, then returns the SDK's
opaque live-owner-qualified result.

## Why you will be glad

- Native calls cannot masquerade as maintenance commands that never ran.
- Missing support and resolver-graph substitution fail closed.
- A diagnostic JSON value cannot be promoted back into checker authority.
- Dropping or replacing the owner invalidates earlier qualified results.

## Where it fits

The crate owns invocation only. `sim-conformance-packs` owns checker law and
live currentness, while the installed acceptance service owns catalog
admission and retention. It owns no process launcher, proof database, or
revocation service.
