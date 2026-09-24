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

The initial platform corpus proves completed success, deliberate divergence,
and durable before-CAS incompletion. Those roles establish the success and
failure observations; they do not imply the other 20 facts. The refusal names
those missing native roles so later platform-owner additions close the same
contract without adding a caller-label seam.
