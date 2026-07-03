# xtask

xtask is a package in the SIM constellation.

## Crates

- `xtask`

## Validation

These commands run in the constellation workspace; only `sim-kernel` builds from a lone clone today (see `DEVELOPING.md` in `sim-sdk`). A single-repo build lands with the first crates.io publish.

```bash
cargo fmt --check && cargo test && cargo clippy -- -D warnings && cargo doc --no-deps
cargo run -p xtask -- simdoc --check
```

## Atelier Site

`cargo run -p xtask -- atelier-site` emits the SIM Atelier Studio Site graph and
refreshes `.sim/atelier/site.json`. The graph places editor, guard, index,
agent, validation, docs, pin, and shell nodes on SUP Site concepts and keeps
`.meta-workspace/` out of editable source roots.

## Atelier Cassette

`cargo run -p xtask -- atelier-cassette` emits the Dev Cassette summary used by
Atelier tooling and refreshes `.sim/atelier/dev-cassette.json`. The summary
names the `ide/event/*` media family, the stream cassette format, redaction
policy, content hash, and dropped-chunks fault diagnostic.

## Atelier Index

`cargo run -p xtask -- atelier-index --repos-manifest <path-to>/repos.toml`
emits the Constellation Index and refreshes `.sim/atelier/index/index.json`.
The index enumerates repos, source paths, validation commands, README text,
recipes, generated simdoc fragments, and Rust item docs as F1 document chunks
with stable ids.

## Atelier Capsule

`cargo run -p xtask -- atelier-capsule` emits the Change Capsule cache and
refreshes `.sim/atelier/change-capsule.json`. The cache records repo previews,
patches, validation and docs placements, generated-artifact policy, pin plans,
front-page changes, replay hashes, and the fairness facet used by capsule views.

## Atelier Radar

`cargo run -p xtask -- atelier-radar "validation command"` queries the
Constellation Index and returns ranked hints with live source spans and
confidence scores. Optional filters restrict results by repo, crate, kind,
capability, codec, or agent role.

## Atelier Guard

`cargo run -p xtask -- atelier-guard` runs the Guideline Firewall over the
constellation manifest. The report names each rule id, location, severity,
evidence, and gated capability; `--check` exits nonzero when error findings are
present.

## Atelier Tools

`cargo run -p xtask -- atelier-tools` emits the typed agent tool catalog and
refreshes `.sim/atelier/tools.json`. The catalog describes `simctl`, validation,
docs, pin, and docs-regeneration tools with guard capabilities and DevEnvelope
evidence fields for command, exit status, and log path.

## Atelier Shell

`cargo run -p xtask -- atelier-shell` emits the Atelier shell aggregate and
refreshes `.sim/atelier/shell.json`. The aggregate loads the Site graph,
Constellation Index, tool catalog, Retrieval Radar panels, Guideline Firewall
report, navigation sections, validation status, repo state, and editor policy.

## Documentation Lanes

`cargo run -p xtask -- simdoc` builds the public documentation lanes:

- API docs: `target/doc/`
- Agent cards: `docs/agents/cards.jsonl` and `docs/agents/card-index.json`
- Human docs: `docs/humans/`
- Diagrams: `docs/diagrams/src/` and `docs/diagrams/generated/`

The same command writes split contract files under `docs/generated/`. Everything
under `docs/` is generated; do not hand-edit it.

### Rustdoc conventions

Public API documentation in `src/` follows one house style:

- Every public item opens with a one-line summary sentence, then context.
- Each report type is framed by the command that produces it and what it
  reports; command entry points state their inputs and the report they return.
- Cross-reference with intra-doc links, and link back to this README rather than
  restating it.

The public API is documentation-gated: `lib.rs` denies `missing_docs`, so every
public item and field must be documented for the crate to build.

### Examples and recipes

xtask's usage examples are its command-line invocations (the Validation and
Documentation Lanes sections above) and its rustdoc. xtask ships no `recipes/`
tree: it is the tool that generates recipe cards from other repos' `recipes/`
directories, and hosts no runnable SIM recipes of its own.
