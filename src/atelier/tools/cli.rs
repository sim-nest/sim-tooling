use std::path::PathBuf;

use super::{AtelierToolsOptions, atelier_tools, render};
use crate::atelier::io::display_io;

pub(super) fn run(args: Vec<String>) -> Result<(), String> {
    let mut options = AtelierToolsOptions {
        control_root: std::env::current_dir().map_err(display_io)?,
        ..AtelierToolsOptions::default()
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
            "--repo" => {
                options.repo_filter = Some(next_string(&mut args, "--repo")?);
            }
            "--cache" => {
                options.cache_path = Some(next_path(&mut args, "--cache")?);
            }
            "--json" => {
                print = true;
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
            other => return Err(format!("unknown atelier-tools option: {other}")),
        }
    }

    let manifest_path = options
        .repos_manifest
        .clone()
        .unwrap_or_else(|| options.control_root.join("repos.toml"));
    let print_options = options.clone();
    let report = atelier_tools(options)?;
    if print {
        print!(
            "{}",
            render::pretty_json(&render::catalog_json(
                &print_options,
                &manifest_path,
                &report.descriptors,
            ))?
        );
    }
    let status = if report.cache_changed {
        "updated"
    } else {
        "current"
    };
    eprintln!(
        "atelier-tools: {} descriptor(s), cache {status}: {}",
        report.descriptors.len(),
        report.cache_file.display()
    );
    Ok(())
}

fn print_usage() {
    println!(
        "usage: xtask atelier-tools [--control-root PATH] [--repos-manifest PATH] [--repo NAME] [--cache PATH] [--json] [--check] [--refresh-only]"
    );
}

fn next_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    next_string(args, flag).map(PathBuf::from)
}

fn next_string(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}
