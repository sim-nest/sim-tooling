// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    os::unix::fs::PermissionsExt,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::tools;
use syn::{Attribute, Expr, ExprCall, ItemFn, ItemMod, visit::Visit};

use super::*;

// conformance: simdoc launches only the toolchain it was built with, found
// without PATH, and never anything through a shell.

fn scratch(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = env::temp_dir().join(format!("{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn running_cargo() -> PathBuf {
    PathBuf::from(env::var_os("CARGO").expect("cargo runs the tests"))
}

#[test]
fn the_toolchain_that_built_simdoc_is_accepted() {
    let tools = Tools::verify(&running_cargo(), &BUILT_WITH, &GIT_LOCATIONS).unwrap();
    let command = tools.cargo();
    assert_eq!(command.get_program(), running_cargo().as_os_str());
    assert_eq!(tools::tools().unwrap().bin, tools.bin);
}

#[test]
fn a_counterfeit_toolchain_reporting_the_right_release_is_refused() {
    let root = scratch("tools-counterfeit");
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir_all(root.join("lib")).unwrap();
    let report = format!(
        "#!/bin/sh\necho 'release: {}'\necho 'commit-hash: {}'\n",
        BUILT_WITH.cargo_release, BUILT_WITH.cargo_commit
    );
    for tool in ["cargo", "rustc", "rustdoc"] {
        let path = root.join("bin").join(tool);
        fs::write(&path, &report).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(root.join("lib/librustc_driver-x.so"), "").unwrap();

    let err = Tools::verify(&root.join("bin/cargo"), &BUILT_WITH, &GIT_LOCATIONS).unwrap_err();
    assert!(
        err.contains("not the one this simdoc was built with"),
        "{err}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_toolchain_that_reports_other_commits_is_refused() {
    for expected in [
        ExpectedToolchain {
            rustc_commit: "0000000000000000000000000000000000000000",
            ..BUILT_WITH
        },
        ExpectedToolchain {
            cargo_commit: "0000000000000000000000000000000000000000",
            ..BUILT_WITH
        },
        ExpectedToolchain {
            rustc_release: "0.0.1",
            ..BUILT_WITH
        },
    ] {
        let err = Tools::verify(&running_cargo(), &expected, &GIT_LOCATIONS).unwrap_err();
        assert!(err.contains("this simdoc was built with"), "{err}");
    }
}

#[test]
fn a_program_that_is_not_an_absolute_path_is_never_looked_up() {
    let err = Tools::verify(Path::new("cargo"), &BUILT_WITH, &GIT_LOCATIONS).unwrap_err();
    assert!(err.contains("not an absolute path"), "{err}");
}

#[test]
fn a_git_that_is_not_root_owned_and_sealed_is_refused() {
    let dir = scratch("tools-git");
    let git = dir.join("git");
    fs::write(&git, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
    let location = git.to_string_lossy().into_owned();
    let err = Tools::verify(&running_cargo(), &BUILT_WITH, &[location.as_str()]).unwrap_err();
    assert!(err.contains("no root-owned git"), "{err}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn children_run_with_a_cleared_environment_and_a_path_of_the_toolchain_only() {
    let tools = Tools::verify(&running_cargo(), &BUILT_WITH, &GIT_LOCATIONS).unwrap();
    for command in [tools.cargo(), tools.rustc(), tools.git()] {
        let envs = command
            .get_envs()
            .map(|(name, value)| (name.to_owned(), value.map(ToOwned::to_owned)))
            .collect::<std::collections::BTreeMap<_, _>>();
        let path = envs[&OsString::from("PATH")].clone().unwrap();
        assert_eq!(
            path,
            OsString::from(format!("{}:{SYSTEM_PATH}", tools.bin.display()))
        );
        for wrapper in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"] {
            assert_eq!(envs[&OsString::from(wrapper)], Some(OsString::new()));
        }
        assert_eq!(
            envs[&OsString::from("RUSTC")],
            Some(tools.rustc.clone().into_os_string())
        );
        assert_eq!(
            envs[&OsString::from("RUSTDOC")],
            Some(tools.rustdoc.clone().into_os_string())
        );
        // `env_clear` leaves no variable outside the ones named here.
        let allowed = PASSED_ENVIRONMENT
            .iter()
            .copied()
            .chain([
                "PATH",
                "CARGO",
                "RUSTC",
                "RUSTDOC",
                "RUSTC_WRAPPER",
                "RUSTC_WORKSPACE_WRAPPER",
                "GIT_CONFIG_GLOBAL",
                "GIT_CONFIG_NOSYSTEM",
                "GIT_TERMINAL_PROMPT",
            ])
            .collect::<Vec<_>>();
        for name in envs.keys() {
            assert!(allowed.contains(&name.to_str().unwrap()), "{name:?}");
        }
    }
}

/// Every process spawn in simdoc's non-test code must be in `tools.rs`.
#[derive(Default)]
struct Spawns {
    found: Vec<String>,
}

fn is_test_attribute(attribute: &Attribute) -> bool {
    let text = quote::quote!(#attribute).to_string().replace(' ', "");
    text == "#[test]" || text == "#[cfg(test)]"
}

impl<'ast> Visit<'ast> for Spawns {
    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if !module.attrs.iter().any(is_test_attribute) {
            syn::visit::visit_item_mod(self, module);
        }
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        if !function.attrs.iter().any(is_test_attribute) {
            syn::visit::visit_item_fn(self, function);
        }
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(path) = &*call.func {
            let names = path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            if names.len() >= 2 && names[names.len() - 2..] == ["Command", "new"] {
                self.found.push(names.join("::"));
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_code_outside_tools_spawns_a_process_or_a_shell() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    source_files(&src, &mut files);
    assert!(files.len() > 30, "the source tree was not found");
    let mut offenders = Vec::new();
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        if name == "tools.rs" || name.ends_with("_tests.rs") || name.ends_with("_fixture.rs") {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap();
        let parsed = syn::parse_file(&text).unwrap_or_else(|err| panic!("{name}: {err}"));
        let mut spawns = Spawns::default();
        spawns.visit_file(&parsed);
        for spawn in spawns.found {
            offenders.push(format!("{name}: {spawn}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "process spawned outside tools.rs: {offenders:?}"
    );
}
