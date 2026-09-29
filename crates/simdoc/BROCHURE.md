# simdoc

In one line: Regenerate and check a repository's generated documentation from a toolchain and dependency closure that is verified byte for byte before it runs.

## What it gives you

`simdoc` is the one engine that produces a SIM repository's generated
documentation lanes: the repo contract, the crate catalog, the validation
matrix, the SIM Index fragment, and the rendered API docs. It runs from its
own resolver root, with its own committed lock, so its own build never shares
a dependency graph with the repository it documents. Before it runs, it
checks its source, its lock, and the exact toolchain against a digest the
repository commits, and it refuses to start if any of them differ.

## Why you will be glad

- A generated page can never quietly drift from the source it describes:
  `--check` re-encodes every lane and compares it byte for byte, never trusting
  a cache.
- A tampered or substituted compiler, `git`, or dependency cannot reach the
  documentation it produces; every input is verified before the engine is
  allowed to run.
- A path is never leaked into a generated file by accident: only a short,
  reviewed list of literal paths is allowed to appear in output at all.
- The full boundary the engine holds to is written down in one place
  (`docs/simdoc-trust-boundaries.md`), so a reviewer can check a new capability
  against a stated allowed set instead of guessing at intent.

## Where it fits

`simdoc` is the encoder every documented repository's own launcher calls; it
owns generation and verification only. It does not decide what belongs in a
repository's documentation policy (that is the launcher and the repository's
own manifest), and it does not publish, commit, or push anything itself.
