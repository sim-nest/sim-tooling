# xtask

xtask is the SIM constellation's internal build-and-dev tooling: it generates
documentation, agent cards, Index evidence, managed vault exports, the Atelier
developer surfaces, and validation reports across the code repos.

This is internal build tooling, not a product you install -- if you want to use
SIM, install the `sim` command from sim-run and see sim-say.

xtask is a package in the SIM constellation.

## Crates

- `xtask`

## Validation

These commands are the repository gate recorded in the constellation manifest.

```bash
cargo fmt --all --check && cargo test && cargo clippy --all-targets -- -D warnings && cargo doc --no-deps
cargo run --locked --offline --manifest-path crates/simdoc/Cargo.toml -- simdoc --check
cargo run -p xtask -- check-file-sizes
```

## File Size Gate

`cargo run -p xtask -- check-file-sizes` scans Rust source files and fails when
an entrypoint (`lib.rs`, `main.rs`, or `mod.rs`) exceeds 250 lines or any other
Rust source file exceeds 700 lines.

## Conformance Packs

`sim_check_pack::prepare_operation_local` is the authority-free
operator-bootstrap seam. It executes the same pure pack evaluation and binds
the exact evidence, support, native invocation, adapter/checker build graphs,
bootstrap grade, and policy used by live issuance. Its prepared value contains
no owner handle, currentness, revocation observation, or receipt. False or
incompletely supported facts refuse the whole preparation.

The `sim_check_pack_xtask::execute_with_owner_currentness` adapter invokes one exact
public conformance pack. Canonical sorted `key=value` evidence is limited to 16
KiB. The caller must supply the weak operation handle issued by the live SDK
boot owner through the in-process qualification boundary. The owner checks the
sealed evidence, selects currentness, issues an opaque qualified artifact, and
immediately re-resolves it against the same live generation. The adapter emits
one `check/result-v1` JSON value with the set, head, key, receipt, invocation,
and observations. Missing, revoked, dead-owner, foreign, or substituted inputs
fail closed.

The checker command is isolated from the documentation tool at
`cargo run --manifest-path crates/Cargo.toml -p sim-check-pack-xtask --
check-pack ...`. Its bare process interface cannot carry that qualified Rust
object and therefore refuses with `owner-currentness-required`. The owner
handle and opaque qualified artifact have no serialized construction path;
semantic admission and current re-resolution stay with the live qualification
owner.

## Citizenize

`cargo run -p xtask -- citizenize <crate-name-or-path>` rewrites public
candidate structs toward the citizen conventions and writes versioned
dependencies that are legal in a public repository. `--local-paths` switches
those dependencies to sibling checkout paths for deliberate local rewrites.

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

## Index Vault Export

`cargo run -p xtask -- index export --input <index.sx> --profile <profile> --vault-root <dir>`
projects a public SIM Index graph into a managed Markdown namespace inside a
user-selected vault root. Profiles are `portable`, `obsidian`, `seqlog`, and
`logseq`; `--namespace` defaults to `SIM-Index`, and `--granularity` defaults to
`compact`.

The exporter consumes the checked public Index graph and writes one managed
namespace for the selected profile. `--plan` prints the deterministic artifact
summary without reading or writing the namespace. `--verify` semantically
decodes a caller-owned existing bundle without writing. `--check` first proves
ownership and byte currency, then performs the same semantic verification.
Write mode updates only the managed namespace described by
its manifest, refuses edited managed notes, and leaves sibling user notes and
application configuration outside the namespace unchanged.

The command is deliberately the final owner in a four-layer composition:
`sim-index-core` supplies the canonical `IndexRowRef` inventory,
`sim-index-vault-core` projects and certifies claims,
`sim-codec-index-vault` composes the Markdown dialect and verifies bundles, and
tooling loads, reports, migrates, and materializes artifacts. Decode never
imports notes. Logseq support means its Markdown file graph, not its DB graph.

## Documentation Lanes

`cargo run --locked --offline --manifest-path crates/simdoc/Cargo.toml -- simdoc`
builds the public documentation lanes:

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
