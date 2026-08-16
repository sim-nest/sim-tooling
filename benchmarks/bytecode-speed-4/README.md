# JVM bytecode baseline

These canonical BENCH_2 reports were measured on the registered `tiger` host at
sim-runtime source `862d4dde7c73957482b0497d2714654db906a4c0`. They retain 40
interleaved raw duration samples and all eight explicit counter rows per report.
No attributed value is estimated.

Reproduce either phase by building sim-runtime's
`bytecode_speed_baseline` example in release mode, generating a request with
the sim-tooling `jvm_baseline_request` example, and passing that request to
`cargo run -- bench run`. Verify the returned artifact with `bench show`; the
decoder recomputes its content key and all summaries from the raw samples.

The captured report identities are:

- cold preparation: `sha256:4bc7d2383522c22f9b937ad4552259deb61dffdfca2cd223b0b5fe10a50a0360`
- warm execution: `sha256:3410df991be855eebeb1b3acdd54440168575470e643b6c93ba4dc606376ae7d`
