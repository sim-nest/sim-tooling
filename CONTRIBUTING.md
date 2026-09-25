# Contributing

Thanks for your interest in SIM. This repo is one crate group in a constellation
of repos that build together; contributions of all sizes are welcome.

## Building and testing

This repo is self-contained and builds against the published SIM crates on
crates.io -- no extra tooling or sibling checkouts are required:

- Clone this repo and run the validation and docs gates below.
- Cross-repo dependencies resolve from crates.io; dependencies within this repo
  resolve locally.

## What a pull request must pass

Every PR runs these gates in CI (`.github/workflows/ci.yml`) on the toolchain
pinned in `rust-toolchain.toml`, and they must be green before merge:

- `cargo fmt --all --check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- `cargo doc --no-deps` (with `RUSTDOCFLAGS=-D warnings`)
- `cargo fmt --manifest-path crates/simdoc/Cargo.toml --check`
- `cargo test --locked --manifest-path crates/simdoc/Cargo.toml`
- `cargo clippy --locked --manifest-path crates/simdoc/Cargo.toml --all-targets -- -D warnings`
- `cargo run --locked --manifest-path crates/simdoc/Cargo.toml -- simdoc --check`
- `cargo run -p xtask -- check-file-sizes`

`simdoc` is the documentation and contract engine. It is its own resolver root
(`crates/simdoc/Cargo.lock`), so it is always run with `--manifest-path` and
`--locked`; `cargo run -p xtask -- simdoc` refuses and points here. The xtask
`repo-contract`, `crate-catalog`, and `validation-matrix` commands run the same
locked engine, and `cargo test` exercises their `--check` modes.

Please keep source and Markdown ASCII-only, and add or update tests for behavior
you change. Public APIs carry `#![deny(missing_docs)]`; document new public items.

## Sign your work (DCO)

We use the Developer Certificate of Origin, not a CLA. Add a `Signed-off-by` line
to each commit certifying you wrote the change or have the right to submit it:

```
git commit -s -m "your message"
```

This adds `Signed-off-by: Your Name <you@example.com>`. That is all we need; there
is no copyright-assignment agreement to sign.

## License

By contributing you agree that your contributions are licensed under the
repository's MPL-2.0 license (see `LICENSE`).

## Filing issues

Use the issue templates. A small reproducible example beats a long description.
Security-sensitive reports go through `SECURITY.md`, not public issues.
