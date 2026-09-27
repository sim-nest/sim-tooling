# sim-check-pack-ubuntu-pc

In one line: Turn only a platform owner's verified Ubuntu native evidence into the exact conformance-pack input the checker accepts, or refuse atomically and name what is missing.

## What it gives you

`sim-check-pack-ubuntu-pc` is the narrow composition between the platform
owner's opaque native-acceptance corpus and `sim-check-pack`'s neutral
`operation/local` checker adapter. It classifies the roles the platform owner
has actually established and derives the canonical evidence, and every
support identity, only from contributing acceptance identities, their
verified platform bindings, and the exact producer and checker build inputs.
It cannot parse a native log, execute a job, issue currentness, or accept a
caller-authored fact or support identity.

## Why you will be glad

- A composition either accounts for every one of the twenty-two required
  checker facts with a typed native witness, or it refuses the whole check and
  names exactly which facts are missing; there is no partial or best-effort
  result.
- Every fact and identity traces back to a verified acceptance binding, so a
  fabricated or reordered input cannot pass as evidence.
- The corpus is re-derivable from ordinary copies after teardown and after a
  reboot, so a composed result can be checked again independently rather than
  trusted on its own say-so.
- A missing native role, not just a missing fact, is refused before any
  evidence is built, so a caller sees the real gap instead of a downstream
  failure.

## Where it fits

This crate owns Ubuntu-specific composition only. `sim-check-pack` owns the
neutral checker adapter this crate feeds, and the platform owner's own crates
own execution, native acceptance, and currentness. It owns no process
launcher, no proof database, and no revocation service.
