use std::path::PathBuf;

use serde_json::{Value, json};
use sim_cookbook::fnv1a64_hex;

use super::io::{check_cache, display_io, write_cache};

const DEFAULT_CACHE: &str = ".sim/atelier/dev-cassette.json";

#[derive(Clone, Debug, PartialEq, Eq)]
struct AtelierCassetteOptions {
    control_root: PathBuf,
    cache_path: Option<PathBuf>,
    check: bool,
}

impl Default for AtelierCassetteOptions {
    fn default() -> Self {
        Self {
            control_root: PathBuf::from("."),
            cache_path: None,
            check: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AtelierCassetteReport {
    summary: Value,
    cache_path: PathBuf,
    cache_changed: bool,
}

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let mut options = AtelierCassetteOptions {
        control_root: std::env::current_dir().map_err(display_io)?,
        ..AtelierCassetteOptions::default()
    };
    let mut print = true;
    let mut args = args.into_iter().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--control-root" => {
                options.control_root = next_path(&mut args, "--control-root")?;
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
            other => return Err(format!("unknown atelier-cassette option: {other}")),
        }
    }

    let report = atelier_cassette(options)?;
    if print {
        print!("{}", pretty_json(&report.summary)?);
    }
    let status = if report.cache_changed {
        "updated"
    } else {
        "current"
    };
    eprintln!(
        "atelier-cassette: cache {status}: {}",
        report.cache_path.display()
    );
    Ok(())
}

fn print_usage() {
    println!(
        "usage: xtask atelier-cassette [--control-root PATH] [--cache PATH] [--check|--refresh-only]"
    );
}

fn atelier_cassette(options: AtelierCassetteOptions) -> Result<AtelierCassetteReport, String> {
    let summary = dev_cassette_summary();
    let cache_path = options
        .cache_path
        .unwrap_or_else(|| options.control_root.join(DEFAULT_CACHE));
    let content = pretty_json(&summary)?;
    let cache_changed = if options.check {
        check_cache(&cache_path, &content, "xtask atelier-cassette")?
    } else {
        write_cache(&cache_path, &content)?
    };
    Ok(AtelierCassetteReport {
        summary,
        cache_path,
        cache_changed,
    })
}

fn dev_cassette_summary() -> Value {
    let events = vec![
        event(0, "edit", "editor", "interactive", "src/lib.rs"),
        event(1, "validate", "validator", "offline-render", "cargo test"),
    ];
    json!({
        "schema": "sim.atelier.dev-cassette.summary.v1",
        "stream_cassette_format": "stream/cassette/v1",
        "media_family": "ide/event/*",
        "events": events,
        "content_hash": content_hash(&events),
        "fault_diagnostics": [
            "stream/fault/drop",
            "dev/diagnostic/dropped-chunks"
        ],
        "redaction": {
            "absolute_paths": "[redacted path]",
            "fixture_root": "fixtures/streams/golden"
        }
    })
}

fn event(
    sequence: u64,
    kind: &'static str,
    node: &'static str,
    latency: &'static str,
    payload: &'static str,
) -> Value {
    json!({
        "sequence": sequence,
        "media": format!("ide/event/{kind}"),
        "atelier_node": node,
        "latency_class": latency,
        "payload": redact_path(payload),
    })
}

fn redact_path(value: &str) -> String {
    if value.starts_with('/') || value.contains("/home/") || value.contains("\\users\\") {
        "[redacted path]".to_owned()
    } else {
        value.to_owned()
    }
}

fn content_hash(events: &[Value]) -> String {
    format!("fnv1a64:{}", fnv1a64_hex(format!("{events:?}").as_bytes()))
}

fn pretty_json(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|mut text| {
            text.push('\n');
            text
        })
        .map_err(|err| format!("render atelier cassette json: {err}"))
}

fn next_path(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    next_string(args, flag).map(PathBuf::from)
}

fn next_string(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

#[cfg(test)]
mod tests {
    use super::{content_hash, dev_cassette_summary, event, redact_path};

    #[test]
    fn cassette_summary_hash_is_stable_for_same_events() {
        let events = vec![
            event(0, "edit", "editor", "interactive", "src/lib.rs"),
            event(1, "validate", "validator", "offline-render", "cargo test"),
        ];

        assert_eq!(content_hash(&events), content_hash(&events));
        assert_eq!(
            dev_cassette_summary()["content_hash"],
            serde_json::Value::String(content_hash(&events))
        );
    }

    #[test]
    fn cassette_summary_redacts_absolute_paths() {
        assert_eq!(redact_path("/workspace/example-repo"), "[redacted path]");
        assert_eq!(redact_path("src/lib.rs"), "src/lib.rs");
    }
}
