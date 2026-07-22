//! Private migration seed extraction for SIM Index authoring.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::index_fragment::slug_path;

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let options = SeedOptions::parse(&args)?;
    ensure_private_output(&options.out)?;
    let source = fs::read_to_string(&options.from)
        .map_err(|err| format!("read {}: {err}", options.from.display()))?;
    let content = seed_from_markdown(&options.from, &source);
    if let Some(parent) = options.out.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;
    }
    fs::write(&options.out, content)
        .map_err(|err| format!("write {}: {err}", options.out.display()))?;
    println!(
        "index seed: wrote private seed rows to {}",
        options.out.display()
    );
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SeedOptions {
    from: PathBuf,
    out: PathBuf,
}

impl SeedOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let program = args.first().map(String::as_str).unwrap_or("xtask");
        if args.get(1).map(String::as_str) != Some("index")
            || args.get(2).map(String::as_str) != Some("seed")
        {
            return Err(usage(program));
        }
        let mut from = None;
        let mut out = None;
        let mut index = 3;
        while index < args.len() {
            match args[index].as_str() {
                "--from" | "--source" => {
                    index += 1;
                    from = Some(PathBuf::from(
                        args.get(index).ok_or("--from requires a path")?.as_str(),
                    ));
                }
                "--out" => {
                    index += 1;
                    out = Some(PathBuf::from(
                        args.get(index).ok_or("--out requires a path")?.as_str(),
                    ));
                }
                "--audience" | "--facet" => {
                    index += 1;
                    let _ = args.get(index).ok_or("--audience requires a value")?;
                }
                other => {
                    return Err(format!(
                        "unknown index seed argument `{other}`; {}",
                        usage(program)
                    ));
                }
            }
            index += 1;
        }
        Ok(Self {
            from: from.ok_or("--from requires a path")?,
            out: out.ok_or("--out requires a path")?,
        })
    }
}

fn usage(program: &str) -> String {
    format!("usage: {program} index seed --from <markdown> --out .sim/index/<name>.seed.toml")
}

fn ensure_private_output(path: &Path) -> Result<(), String> {
    if path
        .components()
        .any(|component| component.as_os_str() == ".sim")
    {
        Ok(())
    } else {
        Err("index seed output must stay under .sim/ until reviewed".to_owned())
    }
}

fn seed_from_markdown(path: &Path, source: &str) -> String {
    let source_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("seed.md");
    let source_stem = source_name.strip_suffix(".md").unwrap_or(source_name);
    let mut out = String::new();
    out.push_str("schema = \"sim.features.seed\"\n");
    push_toml_string(&mut out, "source", &path.to_string_lossy());
    out.push_str("\n# Private migration seed. Review against discovered ids before import.\n");
    for (line_index, line) in source.lines().enumerate() {
        let Some(title) = markdown_heading(line) else {
            continue;
        };
        let slug = slug_path(title);
        if slug.is_empty() {
            continue;
        }
        out.push_str("\n[[seed]]\n");
        push_toml_string(
            &mut out,
            "id",
            &format!("seed/{}/{}", slug_path(source_stem), slug),
        );
        push_toml_string(&mut out, "title", title);
        out.push_str("source_line = ");
        out.push_str(&(line_index + 1).to_string());
        out.push('\n');
    }
    out
}

fn markdown_heading(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let title = trimmed
        .strip_prefix("### ")
        .or_else(|| trimmed.strip_prefix("## "))?;
    let title = title.trim();
    (!title.is_empty() && title.is_ascii()).then_some(title)
}

fn push_toml_string(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(" = ");
    push_quoted(out, value);
    out.push('\n');
}

fn push_quoted(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_output_is_private_and_heading_based() {
        let out = seed_from_markdown(
            Path::new("docs/workbench/FEATURES_4.md"),
            "# Title\n\n## Runtime loading\n\n### REPL surface\n",
        );

        assert!(out.contains("schema = \"sim.features.seed\""));
        assert!(out.contains("seed/features_4/runtime-loading"));
        assert!(out.contains("seed/features_4/repl-surface"));
        assert!(ensure_private_output(Path::new(".sim/index/features4.seed.toml")).is_ok());
        assert!(ensure_private_output(Path::new("docs/generated/features4.seed.toml")).is_err());
    }
}
