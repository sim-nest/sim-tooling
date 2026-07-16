use serde_json::{Value, json};

use super::{AtelierToolDescriptor, AtelierToolsOptions, SCHEMA};

pub(super) fn catalog_json(
    options: &AtelierToolsOptions,
    manifest_path: &std::path::Path,
    descriptors: &[AtelierToolDescriptor],
) -> Value {
    json!({
        "schema": SCHEMA,
        "source_policy": {
            "repos_manifest": manifest_display(&options.control_root, manifest_path),
            "generated_roots": [".meta-workspace/"],
            "editable_roots_include_meta_workspace": false,
            "github_mirror_operations_allowed": false,
            "control_repo_rust_code_allowed": false,
        },
        "summary": {
            "descriptors": descriptors.len(),
            "repo_scoped": descriptors.iter().filter(|descriptor| descriptor.repo.is_some()).count(),
        },
        "descriptors": descriptors.iter().map(AtelierToolDescriptor::to_json).collect::<Vec<_>>(),
    })
}

pub(super) fn pretty_json(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|mut text| {
            text.push('\n');
            text
        })
        .map_err(|err| format!("render atelier tools json: {err}"))
}

fn manifest_display(control_root: &std::path::Path, manifest_path: &std::path::Path) -> String {
    manifest_path
        .strip_prefix(control_root)
        .unwrap_or(manifest_path)
        .display()
        .to_string()
}
