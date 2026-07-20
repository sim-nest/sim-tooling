use std::path::PathBuf;

use super::{AtelierBackend, AtelierShellOptions, atelier_shell, render::pretty_json};
use crate::atelier::io::display_io;

pub(super) fn run(args: Vec<String>) -> Result<(), String> {
    let mut options = AtelierShellOptions {
        control_root: std::env::current_dir().map_err(display_io)?,
        ..AtelierShellOptions::default()
    };
    let mut print = true;
    let mut args = args.into_iter().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--control-root" => options.control_root = next_path(&mut args, "--control-root")?,
            "--repos-manifest" => {
                options.repos_manifest = Some(next_path(&mut args, "--repos-manifest")?);
            }
            "--cache" => options.cache_path = Some(next_path(&mut args, "--cache")?),
            "--backend" => options.backend = next_backend(&mut args, "--backend")?,
            arg if arg.starts_with("--backend=") => {
                options.backend = parse_backend(&arg["--backend=".len()..])?;
            }
            "--check" => {
                options.check = true;
                print = false;
            }
            "--refresh-only" => print = false,
            "--json" => print = true,
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown atelier-shell option: {other}")),
        }
    }

    let report = atelier_shell(options)?;
    if print {
        print!("{}", pretty_json(&report.shell)?);
    }
    let status = if report.cache_changed {
        "updated"
    } else {
        "current"
    };
    eprintln!(
        "atelier-shell: cache {status}: {}",
        report.cache_file.display()
    );
    Ok(())
}

fn print_usage() {
    println!(
        "usage: xtask atelier-shell [--control-root PATH] [--repos-manifest PATH] [--cache PATH] [--backend source-radar|contract-native] [--check|--refresh-only|--json]"
    );
}

fn next_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn next_backend(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<AtelierBackend, String> {
    let value = args
        .next()
        .ok_or_else(|| format!("{flag} requires a value"))?;
    parse_backend(&value)
}

fn parse_backend(value: &str) -> Result<AtelierBackend, String> {
    AtelierBackend::parse(value).ok_or_else(|| {
        format!("unknown atelier-shell backend `{value}`; expected source-radar or contract-native")
    })
}
