use super::{CitizenizeReport, DependencyMode, citizenize_arg_with_mode};

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let (target, dependency_mode) = parse_args(&args[2..])?;
    let report = citizenize_arg_with_mode(target, dependency_mode)?;
    print_report(report);
    Ok(())
}

fn parse_args(args: &[String]) -> Result<(&str, DependencyMode), String> {
    let mut dependency_mode = DependencyMode::Published;
    let mut target = None;

    for arg in args {
        match arg.as_str() {
            "--local-paths" => dependency_mode = DependencyMode::LocalPaths,
            "-h" | "--help" => return Err(usage()),
            other if other.starts_with('-') => {
                return Err(format!("unknown citizenize argument `{other}`"));
            }
            other => {
                if target.replace(other).is_some() {
                    return Err("citizenize accepts exactly one target".to_owned());
                }
            }
        }
    }

    target
        .map(|target| (target, dependency_mode))
        .ok_or_else(usage)
}

fn print_report(report: CitizenizeReport) {
    println!(
        "citizenize: {} candidate(s), {} file(s) changed",
        report.candidates, report.files_changed
    );
}

fn usage() -> String {
    "usage: xtask citizenize [--local-paths] <crate-name-or-path>".to_owned()
}
