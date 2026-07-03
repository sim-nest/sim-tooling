use std::path::PathBuf;

use super::{
    io::display_io,
    site::{AtelierSiteOptions, atelier_site},
};

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let mut options = AtelierSiteOptions {
        control_root: std::env::current_dir().map_err(display_io)?,
        ..AtelierSiteOptions::default()
    };
    let mut print = true;
    let mut args = args.into_iter().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--control-root" => {
                options.control_root = next_path(&mut args, "--control-root")?;
            }
            "--repos-manifest" => {
                options.repos_manifest = Some(next_path(&mut args, "--repos-manifest")?);
            }
            "--editable-root" => {
                options
                    .editable_roots
                    .push(next_string(&mut args, "--editable-root")?);
            }
            "--cache" => {
                options.cache_path = Some(next_path(&mut args, "--cache")?);
            }
            "--check" => {
                options.check = true;
                print = false;
            }
            "--refresh-only" => {
                print = false;
            }
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown atelier-site option: {other}")),
        }
    }

    let report = atelier_site(options)?;
    if print {
        print!("{}", report.site.to_pretty_json()?);
    }
    if let Some(path) = report.cache_path {
        let status = if report.cache_changed {
            "updated"
        } else {
            "current"
        };
        eprintln!("atelier-site: cache {status}: {}", path.display());
    }
    Ok(())
}

fn print_usage() {
    println!(
        "usage: xtask atelier-site [--control-root PATH] [--repos-manifest PATH] [--editable-root PATH] [--cache PATH] [--check|--refresh-only]"
    );
}

fn next_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    next_string(args, flag).map(PathBuf::from)
}

fn next_string(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}
