use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

pub(super) fn narrow_workspace_manifest(
    bytes: &[u8],
    selected: &BTreeSet<String>,
    manifest_closure: &BTreeSet<String>,
) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let mut root: toml::Value = toml::from_str(text).map_err(|error| error.to_string())?;
    let table = root
        .as_table_mut()
        .ok_or("workspace manifest is not a table")?;
    let workspace = table
        .get_mut("workspace")
        .and_then(toml::Value::as_table_mut)
        .ok_or("workspace manifest has no workspace table")?;
    workspace.insert(
        "members".into(),
        toml::Value::Array(
            selected
                .iter()
                .map(|name| toml::Value::String(format!("packages/{name}")))
                .collect(),
        ),
    );
    workspace.remove("default-members");
    workspace.insert(
        "exclude".into(),
        toml::Value::Array(
            manifest_closure
                .difference(selected)
                .map(|name| toml::Value::String(format!("packages/{name}")))
                .collect(),
        ),
    );
    if let Some(dependencies) = workspace
        .get_mut("dependencies")
        .and_then(toml::Value::as_table_mut)
    {
        dependencies.retain(|_, value| retained_dependency(value, manifest_closure));
    }
    if let Some(patches) = table.get_mut("patch").and_then(toml::Value::as_table_mut) {
        for (_, registry) in patches.iter_mut() {
            if let Some(entries) = registry.as_table_mut() {
                entries.retain(|_, value| retained_dependency(value, manifest_closure));
            }
        }
        patches.retain(|_, value| value.as_table().is_some_and(|entries| !entries.is_empty()));
    }
    let rendered = toml::to_string_pretty(&root).map_err(|error| error.to_string())?;
    Ok(rendered.into_bytes())
}

pub(super) fn attach_resolver_package_to_workspace(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let mut root: toml::Value = toml::from_str(text).map_err(|error| error.to_string())?;
    let package = root
        .as_table_mut()
        .and_then(|table| table.get_mut("package"))
        .and_then(toml::Value::as_table_mut)
        .ok_or("resolver manifest has no package table")?;
    package.insert("workspace".into(), toml::Value::String("../..".into()));
    toml::to_string_pretty(&root)
        .map(String::into_bytes)
        .map_err(|error| error.to_string())
}

fn retained_dependency(value: &toml::Value, selected: &BTreeSet<String>) -> bool {
    let Some(path) = value
        .as_table()
        .and_then(|table| table.get("path"))
        .and_then(toml::Value::as_str)
    else {
        return true;
    };
    Path::new(path)
        .strip_prefix("packages")
        .ok()
        .and_then(|path| path.components().next())
        .and_then(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .is_some_and(|name| selected.contains(name))
}
