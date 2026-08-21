# JVM bytecode baseline

These canonical BENCH reports were measured on the registered `tiger` host from
distinct immutable executables: baseline runtime
`d2853cf158ce0e8071a4907eb184a318d9c6ed6c` and candidate runtime
`acb51c7a04479e89b272ea84cd3e6c6e7fecb8f5`. Their executable content keys,
build identities, exact commands, matching host fingerprint, 40 paired samples
per arm, every attempted invocation, executed iteration receipts, and all eight
counter rows are sealed into each report. No attributed value is estimated.

Reproduce either phase by building sim-runtime's
`bytecode_speed_baseline` example in release mode, generating a request with
the sim-tooling `jvm_baseline_request` example, and passing that request to
`cargo run -- bench run`. Verify the returned artifact with `bench show`; the
decoder recomputes its content key and all summaries from the raw samples.

Calibration uses a 10,000-iteration in-process probe. Cold preparation executes
82,667 preparations per measured process (median about 127 ms, making the
roughly 1.5 ms process setup a small fraction). Warm execution executes
1,220,940 dispatch iterations; its candidate median is about 33 ms and its
baseline median about 1.67 s. The warm candidate prepares and verifies once,
while both arms retain identical dispatch, allocation, resolution, root,
safepoint, and work-accounting counts. The earlier equal-arm and distinct but
under-calibrated inconclusive artifacts remain under `attempts/`.

The passing report identities are:

- cold preparation: `sha256:caa561cdac4a98558e17d52ed5cca04ee5399b829d00548e9fb80d5ac10ab880` (`pass`, +0.03%)
- warm execution: `sha256:9475b51c6fa8665ea38b79df0eb6fa92f95171a62ab9686d008842abe28906ac` (`pass`, -98.01%)
