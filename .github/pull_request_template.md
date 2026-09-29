## What this changes

<!-- One or two sentences on the change and why. -->

## Checklist

- [ ] `cargo fmt --all --check` passes
- [ ] `cargo test` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo doc --no-deps` passes
- [ ] `cargo fmt --manifest-path crates/simdoc/Cargo.toml --check` passes
      (`crates/simdoc` is its own resolver root; `cargo run --locked
      --manifest-path crates/simdoc/Cargo.toml -- simdoc --check` is the
      full check, but it only runs on the toolchain pinned in root
      `Cargo.toml` -- see CONTRIBUTING.md)
- [ ] `cargo run -p xtask -- check-file-sizes` passes
- [ ] Tests added/updated for the behavior changed
- [ ] Source and Markdown are ASCII-only
- [ ] Commits are signed off (DCO: `git commit -s`)
