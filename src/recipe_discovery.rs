// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Repository and recipe discovery shared by documentation Card construction.

use std::fs;
use std::path::Path;

pub(crate) fn repo_name(root: &Path) -> String {
    root.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("REPO_NAME")
        .to_owned()
}

pub(crate) fn collect_recipe_files(root: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    visit_for_recipes(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn visit_for_recipes(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|err| format!("read {}: {err}", dir.display()))? {
        let entry = entry.map_err(|err| format!("read {}: {err}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry
            .file_type()
            .map_err(|err| format!("stat {}: {err}", path.display()))?
            .is_dir()
        {
            if matches!(
                name.as_ref(),
                ".git"
                    | ".meta-workspace"
                    | ".sim"
                    | "target"
                    | "generated-reports"
                    | "split-reports"
            ) {
                continue;
            }
            visit_for_recipes(root, &path, files)?;
        } else if is_recipe_path(&path) {
            files.push(relative_slash(root, &path)?);
        }
    }
    Ok(())
}

fn is_recipe_path(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .map(|part| part == "recipes")
            .unwrap_or(false)
    })
}

fn relative_slash(root: &Path, path: &Path) -> Result<String, String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|err| format!("relative path {}: {err}", path.display()))?;
    Ok(rel
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}
