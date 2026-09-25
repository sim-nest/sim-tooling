// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::{
    env,
    os::unix::fs::symlink,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

// conformance: generators consume only ordinary files the repository's Git
// worktree owns; symlinks, nested repositories, and ignored files are never
// followed, and touching one refuses the run.

struct Fixture {
    root: PathBuf,
    outside: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = env::temp_dir().join(format!("{label}-{}-{stamp}", std::process::id()));
        let root = base.join("repo");
        let outside = base.join("private");
        fs::create_dir_all(root.join("recipes/public")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(
            root.join("recipes/public/recipe.toml"),
            "title = \"public\"\n",
        )
        .unwrap();
        fs::write(
            outside.join("recipe.toml"),
            "title = \"private /srv/private/x\"\n",
        )
        .unwrap();
        symlink(&outside, root.join("recipes/private")).unwrap();
        symlink(
            outside.join("recipe.toml"),
            root.join("recipes/linked.toml"),
        )
        .unwrap();
        fs::write(root.join(".gitignore"), "/ignored.toml\n").unwrap();
        fs::write(root.join("ignored.toml"), "secret\n").unwrap();
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/recipe.toml"), "title = \"nested\"\n").unwrap();
        git(&root, &["init", "--quiet"]);
        git(&root.join("nested"), &["init", "--quiet"]);
        Self {
            root: root.canonicalize().unwrap(),
            outside,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.root.parent().unwrap());
        let _ = &self.outside;
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn walks_list_only_owned_ordinary_files() {
    let fixture = Fixture::new("owned-walk");
    let scope = enter(&fixture.root).unwrap();

    let listed = files_under(&fixture.root.join("recipes"))
        .into_iter()
        .map(|path| {
            path.strip_prefix(&fixture.root)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(listed, ["recipes/public/recipe.toml"]);
    let everything = files_under(&fixture.root);
    assert!(
        everything
            .iter()
            .all(|path| !path.starts_with(fixture.root.join("nested")))
    );
    assert!(
        everything
            .iter()
            .all(|path| !path.ends_with("ignored.toml"))
    );
    assert!(read_to_string(fixture.root.join("recipes/public/recipe.toml")).is_ok());
    assert!(!consumed_path("/srv/private/x"));
    scope.finish().unwrap();
}

#[test]
fn touching_any_file_the_worktree_does_not_own_refuses_the_run() {
    let fixture = Fixture::new("owned-refuse");
    for (relative, why) in [
        ("recipes/private/recipe.toml", "reached through a symlink"),
        ("recipes/linked.toml", "is not an ordinary file"),
        ("ignored.toml", "is not a file of this Git worktree"),
        ("nested/recipe.toml", "is not a file of this Git worktree"),
    ] {
        let scope = enter(&fixture.root).unwrap();
        // The caller tolerates the failed read; the scope still refuses.
        let tolerated = read_to_string(fixture.root.join(relative)).unwrap_or_default();
        assert!(tolerated.is_empty());
        let err = scope.finish().unwrap_err();
        assert!(
            err.contains(relative) && err.contains(why),
            "{relative}: {err}"
        );
    }
}

#[test]
fn consumed_content_admits_its_own_paths_and_nested_scopes_share_state() {
    let fixture = Fixture::new("owned-consumed");
    fs::write(
        fixture.root.join("notes.md"),
        "see /usr/share/doc/example\n",
    )
    .unwrap();
    let scope = enter(&fixture.root).unwrap();
    let nested = enter(&fixture.root).unwrap();
    read_to_string(fixture.root.join("notes.md")).unwrap();
    nested.finish().unwrap();
    assert!(consumed_path("/usr/share/doc/example"));
    let _ = read_to_string(fixture.root.join("ignored.toml"));
    assert!(scope.finish().is_err());
}
