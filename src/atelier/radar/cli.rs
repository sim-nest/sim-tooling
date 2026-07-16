use std::path::PathBuf;

use super::{
    AtelierRadarOptions, DEFAULT_LIMIT, atelier_radar,
    render::{pretty_json, print_text_report, report_json},
};
use crate::atelier::io::display_io;

pub(super) fn run(args: Vec<String>) -> Result<(), String> {
    let mut options = AtelierRadarOptions {
        control_root: std::env::current_dir().map_err(display_io)?,
        ..AtelierRadarOptions::default()
    };
    options.query.limit = DEFAULT_LIMIT;

    let mut args = args.into_iter().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--control-root" => options.control_root = next_path(&mut args, "--control-root")?,
            "--index" => options.index_file = Some(next_path(&mut args, "--index")?),
            "--repo" => options.query.repo = Some(next_string(&mut args, "--repo")?),
            "--crate" => options.query.crate_name = Some(next_string(&mut args, "--crate")?),
            "--kind" => options.query.kind = Some(next_string(&mut args, "--kind")?),
            "--capability" => {
                options.query.capability = Some(next_string(&mut args, "--capability")?);
            }
            "--codec" => options.query.codec = Some(next_string(&mut args, "--codec")?),
            "--agent-role" => {
                options.query.agent_role = Some(next_string(&mut args, "--agent-role")?);
            }
            "--limit" => options.query.limit = next_usize(&mut args, "--limit")?,
            "--json" => options.json = true,
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown atelier-radar option: {other}"));
            }
            text if options.query.text.is_empty() => options.query.text = text.to_owned(),
            extra => return Err(format!("unexpected atelier-radar argument: {extra}")),
        }
    }
    if options.query.text.is_empty() {
        return Err("atelier-radar requires a query string".to_owned());
    }

    let report = atelier_radar(options.clone())?;
    if options.json {
        println!("{}", pretty_json(&report_json(&report))?);
    } else {
        print_text_report(&report);
    }
    Ok(())
}

fn print_usage() {
    println!(
        "usage: xtask atelier-radar <query> [--repo NAME] [--crate NAME] [--kind KIND] [--capability NAME] [--codec NAME] [--agent-role NAME] [--limit N] [--control-root PATH] [--index PATH] [--json]"
    );
}

fn next_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    next_string(args, flag).map(PathBuf::from)
}

fn next_usize(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<usize, String> {
    let value = next_string(args, flag)?;
    value
        .parse::<usize>()
        .map_err(|err| format!("{flag} requires a positive integer: {err}"))
}

fn next_string(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}
