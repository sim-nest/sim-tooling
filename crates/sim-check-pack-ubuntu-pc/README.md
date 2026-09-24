# sim-check-pack-ubuntu-pc

`sim-check-pack-ubuntu-pc` is the narrow Ubuntu composition between the
platform owner's opaque native-acceptance corpus and `sim-check-pack`'s neutral
`operation/local` checker adapter.

The crate cannot parse native logs, execute a job, issue currentness, or accept
caller-authored facts and support identities. It accepts only an opaque
`VerifiedOperationLocalCorpus`, classifies the roles the platform owner has
actually established, and refuses the complete check until every one of the 22
required facts has a typed native witness. A successful composition derives
both the canonical evidence and each support identity from the contributing
acceptance identities, their verified platform bindings, the corpus and
definition identities, and the exact producer/checker build inputs.

The platform corpus admits four baseline roles (completed success,
deliberate divergence, before-CAS incompletion and before-acknowledgement
incompletion) and then extensions: bounded stops (timeout and cancellation),
the formatter mutation, the owner validation/docs command pair, the owner
refusal of an interpolated command, and the post-gate witness (three native
sites plus a typed pending projection member). Together these carry typed
native witnesses for all 22 facts. Until the authoritative native batch is
produced and reviewed, those witnesses exist only in source and rehearsal:
no receipt from this crate is evidence of a native run by itself.
