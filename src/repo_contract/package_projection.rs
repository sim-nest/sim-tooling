use super::*;

pub(super) fn targets(repo: &Path, package: &Value) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for target in package["targets"].as_array().into_iter().flatten() {
        out.push(json!({
            "name": target["name"].as_str().unwrap_or_default(),
            "kind": string_array(&target["kind"]),
            "crate_types": string_array(&target["crate_types"]),
            "src": rel_path(repo, &PathBuf::from(target["src_path"].as_str().unwrap_or_default()))?,
        }));
    }
    Ok(out)
}

pub(super) fn dependencies(package: &Value, workspace_names: &BTreeSet<&str>) -> Vec<Value> {
    let mut out = Vec::new();
    for dep in package["dependencies"].as_array().into_iter().flatten() {
        let Some(name) = dep["name"].as_str() else {
            continue;
        };
        if dep["source"].is_null() && workspace_names.contains(name) {
            out.push(json!({
                "package": name,
                "kind": dep["kind"].as_str().unwrap_or("normal"),
                "optional": dep["optional"].as_bool().unwrap_or(false),
                "rename": dep["rename"].as_str(),
                "target": dep["target"].as_str(),
            }));
        }
    }
    out.sort_by(|left, right| {
        left["kind"]
            .as_str()
            .cmp(&right["kind"].as_str())
            .then(left["package"].as_str().cmp(&right["package"].as_str()))
    });
    out
}

pub(super) fn features(package: &Value, workspace_names: &BTreeSet<&str>) -> Vec<Value> {
    let mut out = Vec::new();
    let Some(features) = package["features"].as_object() else {
        return out;
    };
    for (name, edges) in features {
        let raw_edges = string_array(edges);
        let workspace_edges = raw_edges
            .iter()
            .filter_map(|edge| workspace_feature_edge(edge, workspace_names))
            .collect::<Vec<_>>();
        out.push(json!({
            "name": name,
            "edges": raw_edges,
            "workspace_edges": workspace_edges,
        }));
    }
    out
}

fn workspace_feature_edge(edge: &str, workspace_names: &BTreeSet<&str>) -> Option<Value> {
    let without_dep = edge.strip_prefix("dep:").unwrap_or(edge);
    let (package, feature) = without_dep
        .split_once('/')
        .map_or((without_dep, None), |(package, feature)| {
            (package, Some(feature))
        });
    workspace_names.contains(package).then(|| {
        json!({
            "package": package,
            "feature": feature,
        })
    })
}

pub(super) fn target_kinds(package: &Value) -> Vec<String> {
    package["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|target| string_array(&target["kind"]))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn preferred_crate_name(package: &Value) -> Option<&str> {
    package["targets"]
        .as_array()?
        .iter()
        .find(|target| {
            string_array(&target["kind"])
                .iter()
                .any(|kind| kind == "lib")
        })
        .or_else(|| package["targets"].as_array()?.first())?["name"]
        .as_str()
}

pub(super) fn docs_summary(package: &Value) -> Option<String> {
    let target = package["targets"].as_array()?.iter().find(|target| {
        string_array(&target["kind"])
            .iter()
            .any(|kind| kind == "lib" || kind == "bin")
    })?;
    let text = fs::read_to_string(target["src_path"].as_str()?).ok()?;
    let docs = crate_docs(&text);
    let summary = clean_summary(docs.split("\n\n").find(|part| !part.trim().is_empty())?);
    (!summary.is_empty()).then_some(summary)
}

fn crate_docs(text: &str) -> String {
    let mut docs = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(doc) = trimmed.strip_prefix("//!") {
            docs.push(doc.trim_start().to_owned());
        } else if !trimmed.starts_with("#!") && !trimmed.is_empty() && !docs.is_empty() {
            break;
        }
    }
    docs.join("\n")
}

pub(super) fn description(package: &Value) -> String {
    package["description"]
        .as_str()
        .map(clean_summary)
        .filter(|summary| !summary.is_empty())
        .unwrap_or_else(|| {
            format!(
                "SIM workspace package for {}.",
                package["name"]
                    .as_str()
                    .unwrap_or("unknown")
                    .replace('-', " ")
            )
        })
}

fn clean_summary(input: &str) -> String {
    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ")
        .replace(['`', '[', ']'], "")
        .chars()
        .map(|ch| if ch.is_ascii() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn publish(package: &Value) -> String {
    match &package["publish"] {
        Value::Array(registries) if registries.is_empty() => "false",
        Value::Array(_) => "restricted",
        Value::Null => "true",
        _ => "unknown",
    }
    .to_owned()
}

fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

pub(super) fn rel_path(repo: &Path, path: &Path) -> Result<String, String> {
    Ok(path
        .strip_prefix(repo)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/"))
}

pub(super) fn display_io(err: io::Error) -> String {
    err.to_string()
}

pub(super) fn git_output(repo: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|text| !text.is_empty())
}

pub(super) fn stable_hash(repo: &Path, paths: &[PathBuf]) -> String {
    let mut bytes = Vec::new();
    for path in paths {
        let rel = rel_path(repo, path).unwrap_or_else(|_| path.to_string_lossy().into_owned());
        bytes.extend_from_slice(rel.as_bytes());
        bytes.push(0);
        if let Ok(file_bytes) = fs::read(path) {
            bytes.extend_from_slice(&file_bytes);
        }
        bytes.push(0);
    }
    fnv1a64_hex(&bytes)
}
