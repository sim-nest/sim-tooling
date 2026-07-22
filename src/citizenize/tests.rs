use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

#[test]
fn citizenize_scaffolds_fresh_struct_and_is_idempotent() {
    let fixture = fixture("fresh");
    write_fixture(
        &fixture,
        r#"
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlainRecord {
    pub name: String,
    pub values: Vec<i64>,
}
"#,
    );

    let report = citizenize_path(&fixture).unwrap();
    assert_eq!(report.candidates, 1);
    assert_eq!(report.files_changed, 2);

    let source = fs::read_to_string(fixture.join("src/lib.rs")).unwrap();
    assert!(source.contains("use sim_citizen_derive::Citizen;"));
    assert!(source.contains("#[derive(Citizen)]"));
    assert!(source.contains("#[citizen(symbol = \"widget/PlainRecord\", version = 1)]"));
    assert!(source.contains("// TODO: validate citizen example fixture for PlainRecord"));
    assert!(source.contains("pub values: Vec<i64>,"));
    assert!(!source.contains("#[citizen(list)]"));
    assert!(syn::parse_file(&source).is_ok());

    let manifest = fs::read_to_string(fixture.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("sim-citizen = \"0.1.1\""));
    assert!(manifest.contains("sim-citizen-derive = \"0.1.0\""));
    assert!(manifest.contains("sim-kernel = \"0.1.3\""));
    assert!(!manifest.contains("path ="));

    let second = citizenize_path(&fixture).unwrap();
    assert_eq!(second.candidates, 0);
    assert_eq!(second.files_changed, 0);
}

#[test]
fn citizenize_default_dependencies_are_publishable() {
    let fixture = fixture("published_deps");
    write_fixture(
        &fixture,
        r#"
pub struct PlainRecord {
    pub values: Vec<i64>,
}
"#,
    );

    citizenize_path(&fixture).unwrap();

    let manifest = fs::read_to_string(fixture.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("sim-citizen = \"0.1.1\""));
    assert!(manifest.contains("sim-citizen-derive = \"0.1.0\""));
    assert!(manifest.contains("sim-kernel = \"0.1.3\""));
    assert!(!manifest.contains("path ="));
}

#[test]
fn citizenize_local_path_dependencies_are_explicit() {
    let fixture = fixture("local_paths");
    write_fixture(
        &fixture,
        r#"
pub struct PlainRecord {
    pub values: Vec<i64>,
}
"#,
    );

    citizenize_path_with_mode(&fixture, DependencyMode::LocalPaths).unwrap();

    let manifest = fs::read_to_string(fixture.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("sim-citizen = { path = "));
    assert!(manifest.contains("sim-citizen-derive = { path = "));
    assert!(manifest.contains("sim-kernel = { path = "));
}

#[test]
fn citizenize_cli_local_paths_are_explicit() {
    let fixture = fixture("cli_local_paths");
    write_fixture(
        &fixture,
        r#"
pub struct PlainRecord {
    pub values: Vec<i64>,
}
"#,
    );

    crate::run(vec![
        "xtask".to_owned(),
        "citizenize".to_owned(),
        "--local-paths".to_owned(),
        fixture.display().to_string(),
    ])
    .unwrap();

    let manifest = fs::read_to_string(fixture.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("path ="));
}

#[test]
fn citizenize_leaves_existing_citizens_and_exemptions_unchanged() {
    let fixture = fixture("existing");
    let source = r#"
use sim_citizen_derive::Citizen;

#[derive(Clone, Debug, Default, PartialEq, Citizen)]
#[citizen(symbol = "widget/Ready", version = 1)]
pub struct Ready {
    pub name: String,
}

#[non_citizen(reason = "test handle", kind = "handle", descriptor = "test/SocketHandle")]
pub struct SocketHandle {
    pub id: String,
}
"#;
    write_fixture(&fixture, source);
    let before = fs::read_to_string(fixture.join("src/lib.rs")).unwrap();

    let report = citizenize_path(&fixture).unwrap();
    let after = fs::read_to_string(fixture.join("src/lib.rs")).unwrap();
    assert_eq!(report.candidates, 0);
    assert_eq!(report.files_changed, 0);
    assert_eq!(before, after);
}

#[test]
fn citizenize_skips_callable_and_read_constructor_impls() {
    let fixture = fixture("skip_impls");
    let source = r#"
pub struct HostFn {
    pub id: String,
}

impl sim_kernel::Callable for HostFn {
    fn call(
        &self,
        _cx: &mut sim_kernel::Cx,
        _args: sim_kernel::Args,
    ) -> sim_kernel::Result<sim_kernel::Value> {
        unreachable!()
    }
}

pub struct ManualCitizen {
    pub id: String,
}

impl sim_kernel::ReadConstructor for ManualCitizen {
    fn symbol(&self) -> sim_kernel::Symbol {
        sim_kernel::Symbol::new("manual")
    }

    fn args_shape(&self, cx: &mut sim_kernel::Cx) -> sim_kernel::Result<sim_kernel::ShapeRef> {
        cx.factory().nil()
    }

    fn construct_read(
        &self,
        _cx: &mut sim_kernel::Cx,
        _args: Vec<sim_kernel::Value>,
    ) -> sim_kernel::Result<sim_kernel::Value> {
        unreachable!()
    }
}
"#;
    write_fixture(&fixture, source);
    let before = fs::read_to_string(fixture.join("src/lib.rs")).unwrap();
    let report = citizenize_path(&fixture).unwrap();
    let after = fs::read_to_string(fixture.join("src/lib.rs")).unwrap();
    assert_eq!(report.candidates, 0);
    assert_eq!(report.files_changed, 0);
    assert_eq!(before, after);
}

#[test]
fn citizenize_fixture_compiles() {
    let fixture = fixture("compile");
    write_fixture(
        &fixture,
        r#"
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlainRecord {
    pub name: String,
    pub values: Vec<i64>,
}
"#,
    );
    citizenize_path(&fixture).unwrap();
    let manifest = fs::read_to_string(fixture.join("Cargo.toml")).unwrap();
    assert!(!manifest.contains("path ="));
    let status = Command::new("cargo")
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture.join("Cargo.toml"))
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
#[ignore = "local path mode depends on sibling source checkouts"]
fn citizenize_local_path_fixture_compiles() {
    let fixture = fixture("local_compile");
    write_fixture(
        &fixture,
        r#"
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlainRecord {
    pub name: String,
    pub values: Vec<i64>,
}
"#,
    );
    citizenize_path_with_mode(&fixture, DependencyMode::LocalPaths).unwrap();
    patch_transitive_kernel_dependency(&fixture);
    let status = Command::new("cargo")
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture.join("Cargo.toml"))
        .status()
        .unwrap();
    assert!(status.success());
}

fn fixture(name: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let dir = std::env::temp_dir().join(format!(
        "sim-citizenize-{name}-{}-{millis}",
        std::process::id()
    ));
    fs::create_dir_all(dir.join("src")).unwrap();
    dir
}

fn write_fixture(root: &Path, source: &str) {
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "sim-lib-widget"
version = "0.1.0"
edition = "2024"

"#,
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), source.trim_start()).unwrap();
}

fn patch_transitive_kernel_dependency(root: &Path) {
    let manifest_path = root.join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    if manifest.contains("[patch.crates-io]") {
        return;
    }
    let Some(kernel_path) = dependency_path_in_manifest(&manifest, "sim-kernel") else {
        return;
    };
    fs::write(
        manifest_path,
        format!(
            "{manifest}\n[patch.crates-io]\nsim-kernel = {{ path = \"{}\" }}\n",
            kernel_path.replace('\\', "\\\\")
        ),
    )
    .unwrap();
}

fn dependency_path_in_manifest(manifest: &str, name: &str) -> Option<String> {
    manifest.lines().find_map(|line| {
        let trimmed = line.trim_start();
        trimmed
            .starts_with(&format!("{name} ="))
            .then(|| path_dependency(trimmed))
            .flatten()
    })
}

fn path_dependency(line: &str) -> Option<String> {
    let (_, after) = line.split_once("path")?;
    let (_, after) = after.split_once('=')?;
    let after = after.trim_start();
    let after = after.strip_prefix('"')?;
    let (path, _) = after.split_once('"')?;
    Some(path.to_owned())
}
