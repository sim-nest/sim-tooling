//! Stage the merged SIM Index graph for runtime embedding.

use std::{
    fs,
    path::{Path, PathBuf},
};

use sim_codec_index::{IndexCodec, IndexForm};

/// Runs `xtask index snapshot`.
pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = SnapshotOptions::parse(&args)?;
    let source = read_checked_source(&options.input)?;
    write_or_check_snapshot(&options.out, &source, options.check)?;
    if options.check {
        println!("index snapshot: runtime snapshot is current");
    } else {
        println!(
            "index snapshot: copied {} to {}",
            options.input.display(),
            options.out.display()
        );
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SnapshotOptions {
    input: PathBuf,
    out: PathBuf,
    check: bool,
}

impl SnapshotOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let program = args.first().map(String::as_str).unwrap_or("xtask");
        if !matches!(args.get(1).map(String::as_str), Some("index"))
            || !matches!(args.get(2).map(String::as_str), Some("snapshot"))
        {
            return Err(usage(program));
        }

        let mut input = None;
        let mut out = None;
        let mut check = false;
        let mut index = 3;
        while index < args.len() {
            match args[index].as_str() {
                "--input" => {
                    index += 1;
                    input = Some(PathBuf::from(
                        args.get(index).ok_or("--input requires a path")?,
                    ));
                }
                "--out" => {
                    index += 1;
                    out = Some(PathBuf::from(
                        args.get(index).ok_or("--out requires a path")?,
                    ));
                }
                "--check" => check = true,
                "-h" | "--help" => return Err(usage(program)),
                other => {
                    return Err(format!(
                        "unknown index snapshot argument `{other}`; {}",
                        usage(program)
                    ));
                }
            }
            index += 1;
        }

        Ok(Self {
            input: input
                .ok_or_else(|| format!("index snapshot requires --input; {}", usage(program)))?,
            out: out.ok_or_else(|| format!("index snapshot requires --out; {}", usage(program)))?,
            check,
        })
    }
}

fn usage(program: &str) -> String {
    format!("usage: {program} index snapshot --input <index.sx> --out <path> [--check]")
}

fn read_checked_source(path: &Path) -> Result<String, String> {
    let source =
        fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let form = if source.trim_start().starts_with('{') {
        IndexForm::Json
    } else {
        IndexForm::Sx
    };
    IndexCodec
        .decode(form, &source)
        .map_err(|err| format!("decode {}: {err}", path.display()))?;
    Ok(source)
}

fn write_or_check_snapshot(path: &Path, expected: &str, check: bool) -> Result<(), String> {
    if check {
        let current =
            fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
        if current == expected {
            return Ok(());
        }
        return Err(format!(
            "stale runtime index snapshot: {}; run `sh bin/simctl index` and refresh the runtime snapshot",
            path.display()
        ));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;
    }
    fs::write(path, expected).map_err(|err| format!("write {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::{env, fs, path::PathBuf};

    use sim_codec_index::{IndexCodec, IndexForm};
    use sim_index_core::IndexDoc;
    use sim_kernel::EncodePosition;

    use super::*;

    #[test]
    fn snapshot_writes_checked_source() {
        let dir = test_dir("write");
        fs::create_dir_all(&dir).unwrap();
        let input = dir.join("index.sx");
        let output = dir.join("embedded").join("index.sx");
        let source = fixture_index_source();
        fs::write(&input, &source).unwrap();

        let args = vec![
            "xtask".to_owned(),
            "index".to_owned(),
            "snapshot".to_owned(),
            "--input".to_owned(),
            input.display().to_string(),
            "--out".to_owned(),
            output.display().to_string(),
        ];
        run(args).unwrap();

        assert_eq!(fs::read_to_string(output).unwrap(), source);
    }

    #[test]
    fn snapshot_check_rejects_stale_output() {
        let dir = test_dir("check");
        fs::create_dir_all(&dir).unwrap();
        let input = dir.join("index.sx");
        let output = dir.join("snapshot.sx");
        fs::write(&input, fixture_index_source()).unwrap();
        fs::write(&output, "(expr:map)").unwrap();

        let args = vec![
            "xtask".to_owned(),
            "index".to_owned(),
            "snapshot".to_owned(),
            "--input".to_owned(),
            input.display().to_string(),
            "--out".to_owned(),
            output.display().to_string(),
            "--check".to_owned(),
        ];
        let err = run(args).unwrap_err();

        assert!(err.contains("stale runtime index snapshot"));
    }

    fn fixture_index_source() -> String {
        IndexCodec
            .encode(
                &IndexDoc::public("snapshot-test"),
                EncodePosition::Data,
                IndexForm::Sx,
            )
            .unwrap()
    }

    fn test_dir(label: &str) -> PathBuf {
        let mut path = env::temp_dir();
        path.push(format!(
            "sim-tooling-index-snapshot-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }
}
