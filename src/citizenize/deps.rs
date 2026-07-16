use std::{fs, path::PathBuf};

use super::{CrateInfo, DependencyMode, display_io};

pub(super) fn ensure_citizen_dependencies(
    krate: &CrateInfo,
    mode: DependencyMode,
) -> Result<bool, String> {
    let text = fs::read_to_string(&krate.manifest).map_err(display_io)?;
    let mut additions = Vec::new();
    if !has_dependency(&text, "sim-citizen") {
        additions.push(dependency_spec(krate, "sim-citizen", mode));
    }
    if !has_dependency(&text, "sim-citizen-derive") {
        additions.push(dependency_spec(krate, "sim-citizen-derive", mode));
    }
    if !has_dependency(&text, "sim-kernel") {
        additions.push(dependency_spec(krate, "sim-kernel", mode));
    }
    if additions.is_empty() {
        return Ok(false);
    }
    fs::write(&krate.manifest, insert_dependencies(&text, &additions)).map_err(display_io)?;
    Ok(true)
}

fn dependency_spec(krate: &CrateInfo, dep: &str, mode: DependencyMode) -> String {
    match mode {
        DependencyMode::Published => format!("{dep} = \"{}\"", published_version(dep)),
        DependencyMode::LocalPaths => {
            format!("{dep} = {{ path = \"{}\" }}", dependency_path(krate, dep))
        }
    }
}

fn published_version(dep: &str) -> &'static str {
    match dep {
        "sim-citizen" => "0.1.1",
        "sim-citizen-derive" => "0.1.0",
        "sim-kernel" => "0.1.3",
        _ => "0.1",
    }
}

fn has_dependency(manifest: &str, name: &str) -> bool {
    let mut in_dependencies = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed == "[dependencies]" {
            in_dependencies = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_dependencies = false;
        }
        if in_dependencies
            && (trimmed.starts_with(&format!("{name} ="))
                || trimmed.starts_with(&format!("{name}=")))
        {
            return true;
        }
    }
    false
}

fn insert_dependencies(manifest: &str, additions: &[String]) -> String {
    let mut out = String::new();
    let mut in_dependencies = false;
    let mut inserted = false;

    for line in manifest.lines() {
        let trimmed = line.trim();
        if in_dependencies && trimmed.starts_with('[') {
            push_additions(&mut out, additions);
            inserted = true;
            in_dependencies = false;
        }
        out.push_str(line);
        out.push('\n');
        if trimmed == "[dependencies]" {
            in_dependencies = true;
        }
    }

    if !inserted {
        if !in_dependencies {
            if !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str("[dependencies]\n");
        }
        push_additions(&mut out, additions);
    }

    out
}

fn push_additions(out: &mut String, additions: &[String]) {
    for addition in additions {
        out.push_str(addition);
        out.push('\n');
    }
}

fn dependency_path(krate: &CrateInfo, dep: &str) -> String {
    let dep_root = dependency_root(&krate.repo, dep);
    if let (Some(target_parent), Some(dep_parent)) = (krate.root.parent(), dep_root.parent())
        && target_parent == dep_parent
    {
        return format!("../{dep}");
    }
    dep_root.display().to_string()
}

fn dependency_root(repo: &std::path::Path, dep: &str) -> PathBuf {
    let local_crate_root = repo.join("crates").join(dep);
    if local_crate_root.join("Cargo.toml").is_file() {
        return local_crate_root;
    }

    let Some(parent) = repo.parent() else {
        return local_crate_root;
    };
    match dep {
        "sim-kernel" => parent.join("sim-kernel"),
        "sim-citizen" | "sim-citizen-derive" => parent.join("sim-citizen").join("crates").join(dep),
        _ => local_crate_root,
    }
}
