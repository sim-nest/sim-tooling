use std::{fs, path::PathBuf};

use super::{AtelierToolAction, AtelierToolsOptions, atelier_tools};

#[test]
fn simctl_descriptors_cover_control_surface_without_mirror() {
    let fixture = ToolsFixture::new("simctl");
    fixture.write_manifest(&[fixture.repo_row(
        "sim-agent-net",
        true,
        "cargo test --workspace",
        "cargo run -p xtask -- simdoc --check",
    )]);

    let report = fixture.tools();
    let ids = report
        .descriptors
        .iter()
        .map(|descriptor| descriptor.id.as_str())
        .collect::<Vec<_>>();

    for expected in [
        "simctl/clone",
        "simctl/meta-build",
        "simctl/audit",
        "simctl/no-github-check",
        "simctl/site",
        "simctl/repos",
        "simctl/atelier-site",
        "simctl/atelier-index",
        "simctl/atelier-radar",
        "simctl/atelier-guard",
    ] {
        assert!(ids.contains(&expected), "missing {expected}: {ids:?}");
    }
    assert!(!ids.contains(&"simctl/mirror"));
}

#[test]
fn validation_and_docs_descriptors_use_exact_manifest_commands() {
    let fixture = ToolsFixture::new("commands");
    fixture.write_manifest(&[fixture.repo_row(
        "sim-tooling",
        true,
        "cargo fmt --check && cargo clippy -- -D warnings",
        "cargo run -p xtask -- simdoc --check",
    )]);

    let report = fixture.tools();
    let validation = descriptor(&report, "validation/sim-tooling");
    assert_eq!(validation.action, AtelierToolAction::Validate);
    assert_eq!(
        validation.command,
        "cargo fmt --check && cargo clippy -- -D warnings"
    );
    assert_eq!(validation.evidence_kind, "validate");
    assert_eq!(validation.envelope_command, validation.command);
    assert_eq!(validation.envelope_exit_status_field, "exit-status");
    assert!(
        validation
            .envelope_log_path
            .ends_with("validation-sim-tooling.log")
    );

    let docs = descriptor(&report, "docs/sim-tooling");
    assert_eq!(docs.action, AtelierToolAction::Docs);
    assert_eq!(docs.command, "cargo run -p xtask -- simdoc --check");
    assert_eq!(docs.guard_capability, "RegenDocs(sim-tooling)");
    assert_eq!(docs.evidence_kind, "docs");
    assert!(docs.envelope_log_path.ends_with("docs-sim-tooling.log"));
}

#[test]
fn pin_descriptors_require_planpin_and_pushed_commit() {
    let fixture = ToolsFixture::new("pin");
    fixture.write_manifest(&[fixture.repo_row(
        "sim-agent-net",
        true,
        "cargo test",
        "cargo run -p xtask -- simdoc --check",
    )]);

    let report = fixture.tools();
    for id in [
        "pin/pin-propose/sim-agent-net",
        "pin/pin-preview/sim-agent-net",
        "pin/pin-apply/sim-agent-net",
    ] {
        let descriptor = descriptor(&report, id);
        assert_eq!(descriptor.guard_capability, "PlanPin");
        assert!(descriptor.requires_pushed_commit);
        assert_eq!(descriptor.evidence_kind, "pin");
    }
}

#[test]
fn docs_regeneration_refuses_generated_public_doc_hand_edits() {
    let fixture = ToolsFixture::new("docs-policy");
    fixture.write_manifest(&[fixture.repo_row(
        "sim-agent-net",
        true,
        "cargo test",
        "cargo run -p xtask -- simdoc --check",
    )]);

    let report = fixture.tools();
    let descriptor = descriptor(&report, "docs-regeneration/sim-agent-net");
    assert_eq!(descriptor.action, AtelierToolAction::DocsRegenerate);
    assert!(descriptor.refuses_generated_doc_hand_edit);
    assert_eq!(descriptor.guard_capability, "RegenDocs(sim-agent-net)");
}

#[test]
fn repo_filter_keeps_fixed_simctl_descriptors() {
    let fixture = ToolsFixture::new("filter");
    fixture.write_manifest(&[
        fixture.repo_row("sim-agent-net", true, "cargo test", "cargo doc"),
        fixture.repo_row("sim-tooling", true, "cargo clippy", "cargo doc --no-deps"),
    ]);

    let report = atelier_tools(AtelierToolsOptions {
        control_root: fixture.root.clone(),
        repos_manifest: Some(fixture.root.join("repos.toml")),
        repo_filter: Some("sim-tooling".to_owned()),
        ..AtelierToolsOptions::default()
    })
    .unwrap();

    assert!(descriptor_opt(&report, "simctl/atelier-guard").is_some());
    assert!(descriptor_opt(&report, "validation/sim-tooling").is_some());
    assert!(descriptor_opt(&report, "validation/sim-agent-net").is_none());
}

fn descriptor<'a>(
    report: &'a super::AtelierToolsReport,
    id: &str,
) -> &'a super::AtelierToolDescriptor {
    descriptor_opt(report, id).unwrap_or_else(|| panic!("missing descriptor {id}"))
}

fn descriptor_opt<'a>(
    report: &'a super::AtelierToolsReport,
    id: &str,
) -> Option<&'a super::AtelierToolDescriptor> {
    report
        .descriptors
        .iter()
        .find(|descriptor| descriptor.id == id)
}

struct ToolsFixture {
    root: PathBuf,
}

impl ToolsFixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sim-tooling-tools-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn tools(&self) -> super::AtelierToolsReport {
        atelier_tools(AtelierToolsOptions {
            control_root: self.root.clone(),
            repos_manifest: Some(self.root.join("repos.toml")),
            ..AtelierToolsOptions::default()
        })
        .unwrap()
    }

    fn repo_row(
        &self,
        name: &str,
        contains_code: bool,
        validation_command: &str,
        docs_command: &str,
    ) -> String {
        format!(
            "[[repo]]\nname = \"{name}\"\nkind = \"code\"\nlocal_path = \"{name}\"\ncontains_code = {contains_code}\ncrate_names = []\nsource_paths = [\"src\"]\nvalidation_command = \"{validation_command}\"\ndocs_command = \"{docs_command}\"\npublish_to_github = false\ncommit = \"aaaa\"\n\n"
        )
    }

    fn write_manifest(&self, rows: &[String]) {
        fs::write(self.root.join("repos.toml"), rows.join("")).unwrap();
    }
}

impl Drop for ToolsFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
