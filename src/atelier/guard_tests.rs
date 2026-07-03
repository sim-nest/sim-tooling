use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use super::{AtelierGuardOptions, GuidelineSeverity, atelier_guard};

#[test]
fn clean_fixture_reports_no_findings() {
    let fixture = GuardFixture::new("clean");
    fixture.code_repo("sim-clean", false);
    fixture.code_free_repo("repo-control", false);
    fixture.code_free_repo("repo-docs", false);
    fixture.write_manifest(&[
        fixture.repo_row("sim-clean", "code", true, "src", false),
        fixture.repo_row("repo-control", "private", false, "", false),
        fixture.repo_row("repo-docs", "frontpage", false, "", false),
    ]);

    let report = fixture.guard();

    assert_eq!(report.rules.len(), 9);
    assert!(report.findings.is_empty());
    assert_eq!(report.error_count(), 0);
}

#[test]
fn failing_fixture_reports_each_error_rule() {
    let fixture = GuardFixture::new("errors");
    fixture.code_free_repo("repo-control", true);
    fixture.code_free_repo("repo-docs", false);
    fixture.code_repo("sim-alpha", true);
    fixture.write_file("sim-alpha/README.md", &format!("{}\n", '\u{00e9}'));
    fixture.write_file(
        "sim-alpha/Cargo.toml",
        "[package]\nname = \"sim-alpha\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nsim-kernel = { path = \"/workspace/sim-kernel\" }\n",
    );
    fixture.write_file("sim-alpha/src/large.rs", &lines(701));
    fixture.git_init_commit("sim-alpha");
    fixture.write_file("sim-alpha/docs/humans/README.md", "# edited by hand\n");
    fixture.write_manifest(&[
        fixture.repo_row(
            "sim-alpha",
            "code",
            true,
            ".meta-workspace/packages/sim-alpha",
            true,
        ),
        fixture.repo_row("repo-control", "private", false, "", false),
        fixture.repo_row("repo-docs", "frontpage", false, "", false),
    ]);

    let report = fixture.guard();
    let error_ids = report
        .findings
        .iter()
        .filter(|finding| finding.severity == GuidelineSeverity::Error)
        .map(|finding| finding.rule_id.as_str())
        .collect::<Vec<_>>();

    for expected in [
        "ascii-source-markdown",
        "generated-docs-clean",
        "code-free-control-repos",
        "no-local-public-path-deps",
        "meta-workspace-not-source",
        "no-github-work",
        "rust-file-size-policy",
    ] {
        assert!(
            error_ids.contains(&expected),
            "missing expected error rule {expected}: {error_ids:?}"
        );
    }
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.gated_capability == "PlanPin")
    );
}

#[test]
fn warning_fixture_reports_present_tense_file_size_and_kernel_boundary() {
    let fixture = GuardFixture::new("warnings");
    fixture.code_repo("sim-docs", false);
    let flagged_word = ["Fut", "ure"].concat();
    fixture.write_file(
        "sim-docs/README.md",
        &format!("# Docs\n\n{flagged_word} roadmap language.\n"),
    );
    fixture.write_file("sim-docs/src/wide.rs", &lines(501));
    fixture.code_repo("sim-kernel", false);
    fixture.write_file(
        "sim-kernel/src/lib.rs",
        "// parse_json is concrete parser behavior.\n",
    );
    fixture.write_manifest(&[
        fixture.repo_row("sim-docs", "code", true, "src", false),
        fixture.repo_row("sim-kernel", "code", true, "src", false),
    ]);

    let report = fixture.guard();
    let warning_ids = report
        .findings
        .iter()
        .filter(|finding| finding.severity == GuidelineSeverity::Warning)
        .map(|finding| finding.rule_id.as_str())
        .collect::<Vec<_>>();

    for expected in [
        "present-tense-public-docs",
        "rust-file-size-policy",
        "kernel-boundary-warning",
    ] {
        assert!(
            warning_ids.contains(&expected),
            "missing expected warning rule {expected}: {warning_ids:?}"
        );
    }
    assert_eq!(report.error_count(), 0);
}

fn lines(count: usize) -> String {
    (0..count)
        .map(|index| format!("pub const LINE_{index}: usize = {index};\n"))
        .collect()
}

struct GuardFixture {
    root: PathBuf,
}

impl GuardFixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sim-tooling-guard-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn guard(&self) -> super::AtelierGuardReport {
        atelier_guard(AtelierGuardOptions {
            control_root: self.root.clone(),
            repos_manifest: Some(self.root.join("repos.toml")),
            ..AtelierGuardOptions::default()
        })
        .unwrap()
    }

    fn code_repo(&self, name: &str, dirty_docs: bool) {
        self.write_file(
            &format!("{name}/Cargo.toml"),
            &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        );
        self.write_file(&format!("{name}/src/lib.rs"), "pub fn sample() {}\n");
        if dirty_docs {
            self.write_file(&format!("{name}/docs/humans/README.md"), "# generated\n");
        }
    }

    fn code_free_repo(&self, name: &str, with_rust: bool) {
        fs::create_dir_all(self.root.join(name)).unwrap();
        if with_rust {
            self.write_file(&format!("{name}/src/lib.rs"), "pub fn forbidden() {}\n");
        }
    }

    fn repo_row(
        &self,
        name: &str,
        kind: &str,
        contains_code: bool,
        source_path: &str,
        publish_to_github: bool,
    ) -> String {
        let source_paths = if source_path.is_empty() {
            "[]".to_owned()
        } else {
            format!("[\"{source_path}\"]")
        };
        format!(
            "[[repo]]\nname = \"{name}\"\nkind = \"{kind}\"\nlocal_path = \"{name}\"\ncontains_code = {contains_code}\ncrate_names = []\nsource_paths = {source_paths}\nvalidation_command = \"cargo test\"\ndocs_command = \"cargo run -p xtask -- simdoc --check\"\npublish_to_github = {publish_to_github}\ncommit = \"aaaa\"\n\n"
        )
    }

    fn write_manifest(&self, rows: &[String]) {
        fs::write(self.root.join("repos.toml"), rows.join("")).unwrap();
    }

    fn write_file(&self, relative: &str, text: &str) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn git_init_commit(&self, repo: &str) {
        let path = self.root.join(repo);
        git(&path, &["init", "-q"]);
        git(&path, &["add", "."]);
        git(
            &path,
            &[
                "-c",
                "user.name=Example",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-q",
                "-m",
                "fixture",
            ],
        );
    }
}

impl Drop for GuardFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git(path: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(path)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}
