// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use crate::{
    atelier, bench, build_inputs, citizenize, file_size_gate, index_check, index_doctor,
    index_find, index_fixpoint, index_merge, index_overlap, index_render, index_route, index_seed,
    index_snapshot, index_vault, platform_inventory, sealed_resources, simdoc_route,
};

pub(crate) fn dispatch(args: Vec<String>) -> Result<(), String> {
    if matches!(args.as_slice(), [_, command, ..] if command == "bench") {
        return bench::cli::run(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "simdoc") {
        return Err(
            "simdoc moved to its isolated resolver root; run `cargo run --locked --offline --manifest-path crates/simdoc/Cargo.toml -- simdoc ...`"
                .to_owned(),
        );
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-site") {
        return atelier::run(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-cassette") {
        return atelier::run_cassette(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-capsule") {
        return atelier::run_capsule(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-index") {
        return atelier::run_index(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-radar") {
        return atelier::run_radar(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-guard") {
        return atelier::run_guard(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-tools") {
        return atelier::run_tools(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "atelier-shell") {
        return atelier::run_shell(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "check-file-sizes") {
        return file_size_gate::run(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "check-pack") {
        return Err(
            "check-pack moved to the checker workspace; run `cargo run --manifest-path crates/Cargo.toml -p sim-check-pack-xtask -- check-pack ...`"
                .to_owned(),
        );
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "build-inputs") {
        return build_inputs::run(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "sealed-resources") {
        return sealed_resources::run(args);
    }
    if matches!(args.as_slice(), [_, command, subcommand, ..] if command == "index" && subcommand == "doctor")
    {
        return index_doctor::run(args);
    }
    if matches!(args.as_slice(), [_, command, subcommand, ..] if command == "index" && subcommand == "seed")
    {
        return index_seed::run(args);
    }
    if matches!(args.as_slice(), [_, command, subcommand, ..] if command == "index" && subcommand == "merge")
    {
        return index_merge::run(args);
    }
    if matches!(args.as_slice(), [_, command, subcommand, ..] if command == "index" && subcommand == "fixpoint")
    {
        return index_fixpoint::run(args);
    }
    if matches!(args.as_slice(), [_, command, subcommand, ..] if command == "index" && subcommand == "render")
    {
        return index_render::run_render(args);
    }
    if matches!(args.as_slice(), [_, command, subcommand, ..] if command == "index" && subcommand == "export")
    {
        return index_vault::run(args);
    }
    if matches!(args.as_slice(), [_, command, subcommand, ..] if command == "index" && subcommand == "find")
    {
        return index_find::run(args);
    }
    if matches!(args.as_slice(), [_, command, subcommand, ..] if command == "index" && subcommand == "route")
    {
        return index_route::run(args);
    }
    if matches!(args.as_slice(), [_, command, subcommand, ..] if command == "index" && subcommand == "overlap")
    {
        return index_overlap::run(args);
    }
    if matches!(args.as_slice(), [_, command, subcommand, ..] if command == "index" && subcommand == "snapshot")
    {
        return index_snapshot::run(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "index-check") {
        return index_check::run(args);
    }
    if matches!(args.as_slice(), [_, command, ..] if command == "platform-inventory") {
        return platform_inventory::run(args);
    }

    match args.as_slice() {
        // The `xtask-repo-contract-v1` interface (write, `--check`, and
        // `--emit ... --out-dir`) is served by the locked simdoc engine with
        // the arguments passed through unchanged.
        [_, command, rest @ ..] if command == "repo-contract" => {
            simdoc_route::run_forwarded("repo-contract", rest)
        }
        // The contract-derived generators run inside the locked simdoc
        // engine, with the arguments passed through.
        [_, command, rest @ ..] if command == "validation-matrix" || command == "crate-catalog" => {
            simdoc_route::run_forwarded(command, rest)
        }
        [_, command, ..] if command == "citizenize" => citizenize::run(args),
        [program, ..] => Err(format!("usage: {program} <{USAGE_COMMANDS}>")),
        [] => Err(format!("usage: xtask <{USAGE_COMMANDS}>")),
    }
}

const USAGE_COMMANDS: &str = concat!(
    "repo-contract [--check] [--repo <path>]",
    "|validation-matrix [--check] [--repo <path>]",
    "|crate-catalog [--check] [--repo <path>]",
    "|citizenize [--local-paths] <crate-name-or-path>",
    "|check-pack (moved to sim-check-pack-xtask)",
    "|build-inputs <select|derive|materialize|finalize> ...",
    "|sealed-resources <capture --plan <plan.json> --expected-plan-sha256 <hex> --selection <new-selection.json>|materialize --selection <selection.json> --expected-selection-sha256 <hex> --destination <new-path>> --owner-root <path>...",
    "|index doctor --repo <path> --missing --out <path>",
    "|index seed --from <markdown> --out .sim/index/<name>.seed.toml",
    "|index merge --fragment <path>... --out <path> [--check]",
    "|index fixpoint --input <index.sx> --fragment <path>... --self-feature-repo <path> [--strict <selectors>]",
    "|index render --input <index.sx> --out <dir> [--check]",
    "|index export --input <index.sx> --profile <profile> --vault-root <dir> [--namespace <relative-path>] [--granularity compact|full] [--plan|--check]",
    "|index find --input <index.sx> [--json] [--surface <kind-or-id>] [--declaration-kind <kind>] [--implements <protocol>] [--resolved|--unresolved] [--feature <id-or-key>] [<query>]",
    "|index route --input <index.sx> [--json] <task>",
    "|index overlap --input <index.sx> [--clusters <report.json>] [--policy <policy.toml>] [--exceptions <classifications.tsv>] [--control-root <path> --repos-manifest <path>] [--json] [--strict]",
    "|index snapshot --input <index.sx> --out <path> [--check]",
    "|index-check --repo <path> [--strict <category:value,...>]",
    "|check-file-sizes [--repo-root <path>]",
    "|atelier-site [--check]|atelier-cassette [--check]|atelier-capsule [--check]",
    "|atelier-index [--check]|atelier-radar <query>|atelier-guard [--check]",
    "|atelier-tools [--check]|atelier-shell [--backend source-radar|contract-native] [--check]",
);
