use crate::{
    atelier, citizenize, crate_catalog, file_size_gate, generator_options, index_doctor,
    index_seed, repo_contract, simdoc, validation_matrix,
};

pub(crate) fn dispatch(args: Vec<String>) -> Result<(), String> {
    if matches!(args.as_slice(), [_, command, ..] if command == "simdoc") {
        return simdoc::run(args);
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
    if matches!(args.as_slice(), [_, command, subcommand, ..] if command == "index" && subcommand == "doctor")
    {
        return index_doctor::run(args);
    }
    if matches!(args.as_slice(), [_, command, subcommand, ..] if command == "index" && subcommand == "seed")
    {
        return index_seed::run(args);
    }

    match args.as_slice() {
        [_, command, ..] if command == "repo-contract" => {
            let options = generator_options::parse_repo_tool_args(&args, command)?;
            let report = repo_contract::repo_contract_for_repo(options.check, &options.repo)?;
            if options.check {
                println!("repo-contract: generated contract files are current");
                return Ok(());
            }
            println!(
                "repo-contract: {} package(s), {} artifact(s) changed",
                report.packages, report.artifacts_changed
            );
            Ok(())
        }
        [_, command, ..] if command == "validation-matrix" => {
            let options = generator_options::parse_repo_tool_args(&args, command)?;
            let report =
                validation_matrix::validation_matrix_for_repo(options.check, &options.repo)?;
            if options.check {
                println!("validation-matrix: generated matrix is current");
                return Ok(());
            }
            println!(
                "validation-matrix: {} row(s), {} artifact(s) changed",
                report.rows, report.artifacts_changed
            );
            Ok(())
        }
        [_, command, ..] if command == "crate-catalog" => {
            let options = generator_options::parse_repo_tool_args(&args, command)?;
            let report = crate_catalog(options.check, Some(options.repo))?;
            if options.check {
                println!("crate-catalog: metadata and generated files are current");
            } else {
                println!(
                    "crate-catalog: {} package(s), {} manifest(s), {} readme(s), {} catalog file(s)",
                    report.packages,
                    report.manifests_changed,
                    report.readmes_changed,
                    report.catalogs_changed
                );
            }
            Ok(())
        }
        [_, command, ..] if command == "citizenize" => citizenize::run(args),
        [program, ..] => Err(format!("usage: {program} <{USAGE_COMMANDS}>")),
        [] => Err(format!("usage: xtask <{USAGE_COMMANDS}>")),
    }
}

const USAGE_COMMANDS: &str = "repo-contract [--check] [--repo <path>]|validation-matrix [--check] [--repo <path>]|crate-catalog [--check] [--repo <path>]|citizenize [--local-paths] <crate-name-or-path>|simdoc [--check] [--rustdoc auto|skip|force]|index doctor --repo <path> --missing --out <path>|index seed --from <markdown> --out .sim/index/<name>.seed.toml|check-file-sizes [--repo-root <path>]|atelier-site [--check]|atelier-cassette [--check]|atelier-capsule [--check]|atelier-index [--check]|atelier-radar <query>|atelier-guard [--check]|atelier-tools [--check]|atelier-shell [--backend source-radar|contract-native] [--check]";
