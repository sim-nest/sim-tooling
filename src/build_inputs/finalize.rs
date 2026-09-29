//! Finalizes path-dependent Cargo owner inputs without moving assembled trees.

use super::{
    bounded_read, canonical_directory, digest, materialize, parse, staging_path, sync_directory,
    write_new,
};
use serde_json::Value as Json;
use std::{collections::BTreeMap, fs, path::PathBuf};

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let mut special = BTreeMap::new();
    let mut materialize_args = args[..3].to_vec();
    materialize_args[2] = "materialize".into();
    let mut index = 3;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} requires a value"))?;
        if matches!(
            flag,
            "--assembled-materialization"
                | "--expected-assembled-materialization-sha256"
                | "--destination"
        ) {
            if special.insert(flag, value.clone()).is_some() {
                return Err(format!("build-inputs finalize repeats {flag}"));
            }
        } else {
            materialize_args.push(flag.into());
            materialize_args.push(value.clone());
        }
        index += 2;
    }
    let required = |name: &'static str| {
        special
            .get(name)
            .cloned()
            .ok_or_else(|| format!("build-inputs finalize requires {name}"))
    };
    let assembled_path = PathBuf::from(required("--assembled-materialization")?);
    let expected_assembled = required("--expected-assembled-materialization-sha256")?;
    if expected_assembled.len() != 64
        || !expected_assembled
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("assembled materialization digest is not SHA-256".into());
    }
    let destination = PathBuf::from(required("--destination")?);
    if destination.exists() {
        return Err("finalized materialization destination already exists".into());
    }
    let assembled = assembled_path
        .canonicalize()
        .map_err(|error| format!("{}: {error}", assembled_path.display()))?;
    let owner = assembled
        .parent()
        .ok_or("assembled materialization has no owner")?;
    if assembled != owner.join("materialization.json") {
        return Err("assembled materialization is not its tree owner's primary record".into());
    }
    let destination_parent = destination
        .parent()
        .ok_or("finalized materialization destination has no parent")?;
    if canonical_directory(destination_parent)? != owner {
        return Err("finalized materialization must remain with its assembled trees".into());
    }
    let assembled_bytes = bounded_read(&assembled)?;
    if digest(&assembled_bytes) != expected_assembled.to_ascii_lowercase() {
        return Err("assembled materialization identity differs".into());
    }
    let assembled_json: Json = serde_json::from_slice(&assembled_bytes)
        .map_err(|error| format!("assembled materialization is invalid: {error}"))?;
    if assembled_json.get("schema").and_then(Json::as_str)
        != Some("sim.build-input-materialization/v1")
    {
        return Err("assembled materialization schema differs".into());
    }

    let verification = staging_path(owner, &owner.join("finalization-verification"))?;
    materialize_args.push("--destination".into());
    materialize_args.push(verification.display().to_string());
    let options = parse(&materialize_args)?;
    if canonical_directory(&options.workspace)?.parent() != Some(owner)
        || canonical_directory(&options.vendor_root)?.parent() != Some(owner)
    {
        return Err("finalization inputs are not the assembled owner's trees".into());
    }
    materialize(options)?;
    let generated_path = verification.join("materialization.json");
    let generated_bytes = bounded_read(&generated_path)?;
    let generated_json: Json = serde_json::from_slice(&generated_bytes)
        .map_err(|error| format!("generated materialization is invalid: {error}"))?;
    for field in ["source", "vendor", "limits"] {
        if assembled_json.get(field) != generated_json.get(field) {
            return Err(format!(
                "finalization changed assembled {field}; verification retained at {}",
                verification.display()
            ));
        }
    }
    fs::remove_dir_all(&verification)
        .map_err(|error| format!("remove {}: {error}", verification.display()))?;
    write_new(&destination, &generated_bytes, false)?;
    sync_directory(owner)?;
    println!(
        "build-inputs: finalized exact Cargo owners at {}",
        destination.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn tree_comparison_is_structural_not_just_a_declared_identity() {
        let original = json!({
            "source": {"identity": "aa", "files": [{"path": "Cargo.toml"}]},
            "vendor": {"identity": "bb", "files": []},
            "limits": {"source": {"entries": 4}}
        });
        let mut substituted = original.clone();
        substituted["source"]["files"][0]["path"] = json!("substituted");
        assert_ne!(original.get("source"), substituted.get("source"));
        assert_eq!(original.get("vendor"), substituted.get("vendor"));
    }
}
