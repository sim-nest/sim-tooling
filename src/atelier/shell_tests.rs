use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;

use super::shell::{AtelierBackend, AtelierShellOptions, atelier_shell};

#[test]
fn shell_loads_site_index_navigation_and_repo_status() {
    let fixture = ShellFixture::new("navigation");
    fixture
        .repo("sim-web")
        .cargo("sim-web-shell")
        .readme("Codec Prism uses lisp json. Agent planner validator guard docs pin.")
        .recipe("recipes/01-basics/codec-prism/recipe.toml")
        .rust_lib("pub fn rust_source() {}\n")
        .git_clean();
    fixture
        .repo("sim-sdk")
        .cargo("sim-server")
        .readme("Validation command and server route.")
        .git_clean()
        .dirty_file("notes.txt");
    fixture.write_manifest(&[
        repo_row("sim-web", "sim-web", &["sim-web-shell"]),
        repo_row("sim-sdk", "sim-sdk", &["sim-server"]),
        repo_row("sim-missing", "sim-missing", &["sim-missing"]),
    ]);

    let report = fixture.shell(false);
    assert_eq!(report["schema"], "sim.atelier.shell.v1");
    assert_eq!(report["site"]["nodes"].as_array().unwrap().len(), 8);
    assert!(nav_items(&report, "repo").contains(&"sim-web".to_owned()));
    assert!(nav_items(&report, "crate").contains(&"sim-web-shell".to_owned()));
    assert!(nav_items(&report, "codec").contains(&"lisp".to_owned()));
    assert!(
        nav_items(&report, "agent-role").contains(&"validator".to_owned()),
        "agent role navigation is populated"
    );
    assert!(
        nav_items(&report, "guard-rule").contains(&"generated-docs-clean".to_owned()),
        "guard rules are navigable"
    );
    assert_eq!(strings_at(&report, "/startup/dirty_repos"), vec!["sim-sdk"]);
    assert_eq!(
        strings_at(&report, "/startup/missing_siblings"),
        vec!["sim-missing"]
    );
    assert!(report["radar"].as_array().unwrap().len() >= 5);
}

#[test]
fn shell_check_detects_stale_cache() {
    let fixture = ShellFixture::new("stale");
    fixture
        .repo("sim-web")
        .cargo("sim-web-shell")
        .readme("Codec lisp agent guard.")
        .git_clean();
    fixture.write_manifest(&[repo_row("sim-web", "sim-web", &["sim-web-shell"])]);
    let _ = fixture.shell(false);
    fs::write(fixture.root.join(".sim/atelier/shell.json"), "{}\n").unwrap();

    let err = fixture.shell_err(true);
    assert!(err.contains("stale"), "{err}");
}

#[test]
fn shell_marks_generated_docs_read_only() {
    let fixture = ShellFixture::new("docs-policy");
    fixture
        .repo("sim-web")
        .cargo("sim-web-shell")
        .readme("Recipe source docs and codec lisp.")
        .recipe("recipes/01-basics/open-doc/recipe.toml")
        .git_clean();
    fixture.write_manifest(&[repo_row("sim-web", "sim-web", &["sim-web-shell"])]);

    let report = fixture.shell(false);
    assert!(
        strings_at(&report, "/editor_policy/editable_docs")
            .contains(&"recipes/**/recipe.toml".to_owned())
    );
    assert!(
        strings_at(&report, "/editor_policy/read_only_generated_docs")
            .contains(&"docs/generated/".to_owned())
    );
}

#[test]
fn source_radar_default_matches_explicit_backend() {
    let fixture = ShellFixture::new("default-backend");
    fixture
        .repo("sim-web")
        .cargo("sim-web-shell")
        .readme("Codec lisp agent guard.")
        .git_clean();
    fixture.write_manifest(&[repo_row("sim-web", "sim-web", &["sim-web-shell"])]);

    let implicit = fixture.shell(false);
    let explicit = fixture.shell_with_backend(false, AtelierBackend::SourceRadar);
    assert_eq!(implicit, explicit);
    assert!(implicit.get("contract_native").is_none());
    assert!(!panel_ids(&implicit).contains(&"contract-native".to_owned()));
    assert!(implicit["startup"].get("backend").is_none());
}

#[test]
fn contract_native_backend_adds_cached_evidence() {
    let fixture = ShellFixture::new("contract-native");
    fixture
        .repo("sim-agent-net")
        .cargo("sim-lib-agent")
        .readme("Agent contract grammar guard validation.")
        .git_clean();
    fixture.write_manifest(&[repo_row(
        "sim-agent-net",
        "sim-agent-net",
        &["sim-lib-agent"],
    )]);

    let report = fixture.shell_with_backend(false, AtelierBackend::ContractNative);
    assert_eq!(report["startup"]["backend"], "contract-native");
    assert!(panel_ids(&report).contains(&"contract-native".to_owned()));
    assert_eq!(
        report["contract_native"]["schema"],
        "sim.atelier.contract-native.v1"
    );
    assert_eq!(report["contract_native"]["contract_deck"]["cards"], 4);
    assert_eq!(report["contract_native"]["projection"]["tokens"], 114);
    assert_eq!(
        report["contract_native"]["grammar"]["dialect"],
        "shapegrammar"
    );
    assert_eq!(
        report["contract_native"]["route_attempts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        strings_at(&report, "/contract_native/guard_denials").is_empty(),
        "guard denials are object rows, not string rows"
    );
    let denials = report["contract_native"]["guard_denials"]
        .as_array()
        .unwrap();
    assert!(
        denials
            .iter()
            .any(|denial| denial["id"].as_str() == Some("meta-workspace-edit"))
    );
    assert!(
        denials
            .iter()
            .any(|denial| denial["id"].as_str() == Some("cross-repo-write"))
    );
    assert!(
        denials
            .iter()
            .any(|denial| denial["id"].as_str() == Some("github-outward-action"))
    );
    assert!(
        report["contract_native"]["cassette_hash"]
            .as_str()
            .unwrap()
            .starts_with("fnv1a64:")
    );
}

fn nav_items(report: &Value, kind: &str) -> Vec<String> {
    report["navigation"]
        .as_array()
        .unwrap()
        .iter()
        .find(|section| section["kind"].as_str() == Some(kind))
        .and_then(|section| section["items"].as_array())
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn panel_ids(report: &Value) -> Vec<String> {
    report["panels"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|panel| panel["id"].as_str())
        .map(str::to_owned)
        .collect()
}

fn strings_at(report: &Value, pointer: &str) -> Vec<String> {
    report
        .pointer(pointer)
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn repo_row(name: &str, local_path: &str, crates: &[&str]) -> String {
    format!(
        r#"[[repo]]
name = "{name}"
kind = "code"
visibility = "public"
forgejo_remote = "https://example.invalid/sim/{name}.git"
github_remote = ""
local_path = "{local_path}"
contains_code = true
crate_names = [{crates}]
source_paths = ["."]
validation_command = "cargo fmt --check"
docs_command = "cargo run -p xtask -- simdoc --check"
publish_to_github = false
history_policy = "sanitized"
commit = "0000000000000000000000000000000000000000"
"#,
        crates = crates
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

struct ShellFixture {
    root: PathBuf,
}

impl ShellFixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sim-tooling-shell-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn write_manifest(&self, rows: &[String]) {
        fs::write(self.root.join("repos.toml"), rows.join("\n")).unwrap();
    }

    fn repo(&self, name: &str) -> RepoFixture {
        RepoFixture {
            root: self.root.join(name),
        }
    }

    fn shell(&self, check: bool) -> Value {
        self.shell_with_backend(check, AtelierBackend::SourceRadar)
    }

    fn shell_with_backend(&self, check: bool, backend: AtelierBackend) -> Value {
        atelier_shell(AtelierShellOptions {
            control_root: self.root.clone(),
            repos_manifest: Some(self.root.join("repos.toml")),
            cache_path: Some(self.root.join(".sim/atelier/shell.json")),
            backend,
            check,
        })
        .unwrap()
        .shell
    }

    fn shell_err(&self, check: bool) -> String {
        atelier_shell(AtelierShellOptions {
            control_root: self.root.clone(),
            repos_manifest: Some(self.root.join("repos.toml")),
            cache_path: Some(self.root.join(".sim/atelier/shell.json")),
            backend: AtelierBackend::SourceRadar,
            check,
        })
        .unwrap_err()
    }
}

impl Drop for ShellFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Clone)]
struct RepoFixture {
    root: PathBuf,
}

impl RepoFixture {
    fn cargo(&self, name: &str) -> Self {
        fs::create_dir_all(self.root.join("src")).unwrap();
        fs::write(
            self.root.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        )
        .unwrap();
        self.clone()
    }

    fn readme(&self, text: &str) -> Self {
        fs::create_dir_all(&self.root).unwrap();
        fs::write(self.root.join("README.md"), text).unwrap();
        self.clone()
    }

    fn rust_lib(&self, text: &str) -> Self {
        fs::write(self.root.join("src/lib.rs"), text).unwrap();
        self.clone()
    }

    fn recipe(&self, path: &str) -> Self {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "id = \"demo\"\ncodec = \"lisp\"\n").unwrap();
        self.clone()
    }

    fn dirty_file(&self, path: &str) -> Self {
        fs::write(self.root.join(path), "dirty\n").unwrap();
        self.clone()
    }

    fn git_clean(&self) -> Self {
        run_git(&self.root, &["init"]);
        run_git(&self.root, &["config", "user.email", "noreply@example.com"]);
        run_git(&self.root, &["config", "user.name", "Atelier Test"]);
        run_git(&self.root, &["add", "."]);
        run_git(&self.root, &["commit", "-m", "init"]);
        self.clone()
    }
}

fn run_git(root: &Path, args: &[&str]) {
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
