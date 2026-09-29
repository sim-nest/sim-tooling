# sim-check-pack-xtask

In one line: Run a native SIM conformance-pack check as its own isolated command, without pulling checker dependencies into the shared xtask build.

## What it gives you

`sim-check-pack-xtask` is the command-line front door to `sim-check-pack`,
built as a separate binary so the checker's own dependency graph (the
conformance-pack implementations, the kernel it exercises) never has to be
resolved or compiled as part of the shared `xtask` tool that every other
repository task uses.

## Why you will be glad

- Adding or upgrading a conformance pack never slows down or destabilizes the
  everyday `xtask` commands that have nothing to do with checking.
- The checker command keeps the same qualification and owner-currentness
  guarantees as when it lived inside `xtask`; moving it out changed where it
  builds, not what it verifies.
- A build failure in a conformance pack shows up exactly where it belongs,
  in this crate's own build, not as a mystery failure in an unrelated task.

## Where it fits

This crate owns invocation only: parsing the command and calling
`sim-check-pack`. It owns no checker law, no result storage, and no catalog
admission; those stay with `sim-check-pack` and the services that consume its
output.
