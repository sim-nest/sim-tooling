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
sites plus a typed pending projection member). Together these are the typed
witness shapes the 22 facts require; the projection-final-image site stays a
pending member, never satisfied here, until the projection-boundary packet
completes it. The before-acknowledgement role enters as the platform's
recorded provider-bound attestation: the product collector attests PID 1's
terminals and installation currency once, live, and the composer
(`examples/native_corpus_compose.rs`) re-derives every content fact from
ordinary copies and checks it against that record, so the corpus is
re-derivable after teardown and after a reboot. Until the authoritative
native batch is produced and reviewed, those witnesses exist only in source
and rehearsal: no receipt from this crate is evidence of a native run by
itself.
