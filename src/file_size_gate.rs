use std::{
    fs, io,
    path::{Path, PathBuf},
};

const ENTRYPOINT_HARD_LIMIT: usize = 250;
const GENERAL_HARD_LIMIT: usize = 700;

pub(crate) fn run(args: Vec<String>) -> Result<(), String> {
    let root = parse_args(&args)?;
    check_rust_file_sizes(&root)?;
    println!("check-file-sizes: Rust source files are within hard limits");
    Ok(())
}

pub(crate) fn check_rust_file_sizes(root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files).map_err(display_io)?;
    files.sort();

    let mut failures = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).map_err(display_io)?;
        let lines = text.lines().count();
        let limit = hard_limit(&file);
        if lines > limit {
            failures.push(format!(
                "{} has {lines} line(s), hard limit is {limit}",
                display_path(root, &file)
            ));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Rust file-size hard limit exceeded:\n{}",
            failures.join("\n")
        ))
    }
}

fn parse_args(args: &[String]) -> Result<PathBuf, String> {
    let mut root = std::env::current_dir().map_err(display_io)?;
    let mut args = args.iter().skip(2);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repo-root" => {
                root = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--repo-root requires a value".to_owned())?;
            }
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("unknown check-file-sizes option: {other}")),
        }
    }
    Ok(root)
}

fn collect_rust_files(root: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if path.is_dir() {
            if should_skip_dir(name) {
                continue;
            }
            collect_rust_files(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".github" | ".sim" | "docs" | "target" | ".meta-workspace"
    )
}

fn hard_limit(path: &Path) -> usize {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "lib.rs" | "main.rs" | "mod.rs"))
    {
        ENTRYPOINT_HARD_LIMIT
    } else {
        GENERAL_HARD_LIMIT
    }
}

fn display_path(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string()
}

fn usage() -> String {
    "usage: xtask check-file-sizes [--repo-root PATH]".to_owned()
}

fn display_io(err: io::Error) -> String {
    err.to_string()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn accepts_files_at_hard_limits() {
        let root = fixture("accepts");
        write_lines(&root.join("src/lib.rs"), ENTRYPOINT_HARD_LIMIT);
        write_lines(&root.join("src/tool.rs"), GENERAL_HARD_LIMIT);

        check_rust_file_sizes(&root).unwrap();
    }

    #[test]
    fn rejects_entrypoints_and_general_files_over_hard_limits() {
        let root = fixture("rejects");
        write_lines(&root.join("src/lib.rs"), ENTRYPOINT_HARD_LIMIT + 1);
        write_lines(&root.join("src/tool.rs"), GENERAL_HARD_LIMIT + 1);

        let err = check_rust_file_sizes(&root).unwrap_err();
        assert!(err.contains("src/lib.rs has 251 line(s)"));
        assert!(err.contains("src/tool.rs has 701 line(s)"));
    }

    fn fixture(name: &str) -> PathBuf {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let root = std::env::temp_dir().join(format!("sim-file-size-gate-{name}-{millis}"));
        fs::create_dir_all(root.join("src")).unwrap();
        root
    }

    fn write_lines(path: &Path, count: usize) {
        let text = (0..count)
            .map(|index| format!("// line {index}\n"))
            .collect::<String>();
        fs::write(path, text).unwrap();
    }
}
