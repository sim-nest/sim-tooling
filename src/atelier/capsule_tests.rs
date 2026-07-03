use std::{fs, path::PathBuf, process::Command};

use serde_json::Value;

use super::{
    capsule::{AtelierCapsuleOptions, atelier_capsule, test_capsule_json},
    index_manifest::read_repos_manifest,
};

#[test]
fn capsule_cache_reports_preview_pins_and_replay_hash() {
    let fixture = CapsuleFixture::new("preview");
    fixture.repo("sim-tooling").cargo().git_clean();
    fixture.write_manifest(
        r#"[[repo]]
name = "sim-tooling"
kind = "code"
local_path = "sim-tooling"
contains_code = true
crate_names = ["xtask"]
source_paths = ["."]
validation_command = "cargo test"
docs_command = "cargo run -p xtask -- simdoc --check"
publish_to_github = false
commit = "0000000000000000000000000000000000000000"
"#,
    );

    let report = fixture.capsule(false);
    assert_eq!(report["schema"], "sim.atelier.change-capsule.v1");
    assert_eq!(
        report["capsule"]["dev_cassette"]["content_hash"],
        report["capsule"]["dev_cassette"]["replay_content_hash"]
    );
    assert_eq!(
        report["capsule"]["policy"]["preview_public_repos_before_pin"],
        true
    );
    assert!(
        report["capsule"]["pin_plan"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pin| pin["repo"] == "sim-tooling")
    );
}

#[test]
fn capsule_check_detects_stale_cache() {
    let fixture = CapsuleFixture::new("stale");
    fixture.repo("sim-tooling").cargo().git_clean();
    fixture.write_manifest(
        r#"[[repo]]
name = "sim-tooling"
kind = "code"
local_path = "sim-tooling"
contains_code = true
crate_names = ["xtask"]
source_paths = ["."]
validation_command = "cargo test"
docs_command = ""
publish_to_github = false
commit = "0000000000000000000000000000000000000000"
"#,
    );
    let _ = fixture.capsule(false);
    fs::write(
        fixture.root.join(".sim/atelier/change-capsule.json"),
        "{}\n",
    )
    .unwrap();

    let err = fixture.capsule_err(true);
    assert!(err.contains("stale"), "{err}");
}

#[test]
fn stale_pin_policy_marks_unpushed_commits_as_refused() {
    let fixture = CapsuleFixture::new("policy");
    fixture.repo("sim-tooling").cargo().git_clean();
    fixture.write_manifest(
        r#"[[repo]]
name = "sim-tooling"
kind = "code"
local_path = "sim-tooling"
contains_code = true
crate_names = ["xtask"]
source_paths = ["."]
validation_command = "cargo test"
docs_command = ""
publish_to_github = false
commit = "badbadbadbadbadbadbadbadbadbadbadbadbadb"
"#,
    );
    let repos = read_repos_manifest(&fixture.root, &fixture.root.join("repos.toml")).unwrap();
    let report = test_capsule_json(&repos);

    assert_eq!(
        report["capsule"]["policy"]["refuses_stale_pins"],
        Value::Bool(true)
    );
    assert_eq!(
        report["capsule"]["pin_plan"][0]["pushed_commit_exists"],
        Value::Bool(false)
    );
    assert_eq!(
        report["capsule"]["generated_artifacts"][0]["hand_edited"],
        Value::Bool(false)
    );
}

struct CapsuleFixture {
    root: PathBuf,
}

impl CapsuleFixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sim-tooling-capsule-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write_manifest(&self, text: &str) {
        fs::write(self.root.join("repos.toml"), text).unwrap();
    }

    fn repo(&self, name: &str) -> RepoFixture {
        RepoFixture {
            root: self.root.join(name),
        }
    }

    fn capsule(&self, check: bool) -> Value {
        atelier_capsule(AtelierCapsuleOptions {
            control_root: self.root.clone(),
            repos_manifest: Some(self.root.join("repos.toml")),
            cache_path: Some(self.root.join(".sim/atelier/change-capsule.json")),
            check,
        })
        .unwrap()
        .capsule
    }

    fn capsule_err(&self, check: bool) -> String {
        atelier_capsule(AtelierCapsuleOptions {
            control_root: self.root.clone(),
            repos_manifest: Some(self.root.join("repos.toml")),
            cache_path: Some(self.root.join(".sim/atelier/change-capsule.json")),
            check,
        })
        .unwrap_err()
    }
}

impl Drop for CapsuleFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct RepoFixture {
    root: PathBuf,
}

impl RepoFixture {
    fn cargo(&self) -> &Self {
        fs::create_dir_all(self.root.join("src")).unwrap();
        fs::write(
            self.root.join("Cargo.toml"),
            "[package]\nname = \"xtask\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(self.root.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
        self
    }

    fn git_clean(&self) {
        run_git(&self.root, &["init"]);
        run_git(&self.root, &["config", "user.email", "noreply@example.com"]);
        run_git(&self.root, &["config", "user.name", "Capsule Test"]);
        run_git(&self.root, &["add", "."]);
        run_git(&self.root, &["commit", "-m", "init"]);
    }
}

fn run_git(root: &PathBuf, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
