//! The simdoc task: build or check the documentation lanes.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cardspine_state::{CardSpineState, file_lane_digest, lane_digest, lanes_to_reencode};
use crate::repo_contract::contract_artifacts;
use crate::simdoc_rustdoc::run_api_docs;
use crate::{CardSpine, DocEncoder, DocPosition};

pub fn run(args: Vec<String>) -> Result<(), String> {
    let options = SimdocOptions::parse(&args)?;
    simdoc(&options.root, options.check, options.rustdoc)
}

fn simdoc(root: &Path, check: bool, rustdoc: RustdocMode) -> Result<(), String> {
    if rustdoc == RustdocMode::Skip {
        println!("simdoc: rustdoc skipped");
    } else {
        run_api_docs(root, rustdoc == RustdocMode::Force)?;
    }
    run_recipe_gate(root)?;

    let expected = expected_files(root)?;
    let card_lanes_encoded = expected.card_lanes_encoded();
    if check {
        check_files(root, &expected.files)?;
        persist_state(root, &expected)?;
        if card_lanes_encoded == 0 {
            println!("simdoc: card content ids unchanged; no card-backed re-encode");
        }
        println!("simdoc: generated documentation lanes are current");
    } else {
        write_files(root, &expected.files)?;
        persist_state(root, &expected)?;
        if card_lanes_encoded == 0 {
            println!("simdoc: card content ids unchanged; reused card-backed lanes");
        }
        println!("simdoc: generated documentation lanes refreshed");
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SimdocOptions {
    root: PathBuf,
    check: bool,
    rustdoc: RustdocMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RustdocMode {
    Auto,
    Skip,
    Force,
}

impl SimdocOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let program = args.first().map(String::as_str).unwrap_or("xtask");
        if args.get(1).map(String::as_str) != Some("simdoc") {
            return Err(usage(program));
        }

        let mut root = None;
        let mut check = false;
        let mut rustdoc = RustdocMode::Auto;
        let mut index = 2;
        while index < args.len() {
            match args[index].as_str() {
                "--check" => check = true,
                "--repo-root" => {
                    index += 1;
                    let Some(path) = args.get(index) else {
                        return Err("--repo-root requires a path".to_owned());
                    };
                    root = Some(PathBuf::from(path));
                }
                "--rustdoc" => {
                    index += 1;
                    let Some(mode) = args.get(index) else {
                        return Err("--rustdoc requires auto, skip, or force".to_owned());
                    };
                    rustdoc = match mode.as_str() {
                        "auto" => RustdocMode::Auto,
                        "skip" => RustdocMode::Skip,
                        "force" => RustdocMode::Force,
                        other => {
                            return Err(format!(
                                "unknown --rustdoc mode `{other}`; expected auto, skip, or force"
                            ));
                        }
                    };
                }
                other => {
                    return Err(format!(
                        "unknown simdoc argument `{other}`; {}",
                        usage(program)
                    ));
                }
            }
            index += 1;
        }

        Ok(Self {
            root: root.map(Ok).unwrap_or_else(|| {
                env::current_dir().map_err(|err| format!("current dir: {err}"))
            })?,
            check,
            rustdoc,
        })
    }
}

fn usage(program: &str) -> String {
    format!("usage: {program} simdoc [--repo-root PATH] [--check] [--rustdoc auto|skip|force]")
}

fn run_recipe_gate(root: &Path) -> Result<(), String> {
    let checker = root.join("scripts/check-recipes.sh");
    if !checker.exists() {
        return Ok(());
    }
    let status = Command::new("sh")
        .arg(checker)
        .current_dir(root)
        .status()
        .map_err(|err| format!("recipe gate: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("recipe gate failed with status {status}"))
    }
}

fn expected_files(root: &Path) -> Result<ExpectedFiles, String> {
    let repo = repo_name(root);
    let contract_files = contract_artifacts(root)?.files;
    let spine = CardSpine::for_repo(root)?;
    let state = CardSpineState::read(root)?;
    let reencode = state
        .as_ref()
        .map(|state| lanes_to_reencode(&spine, state))
        .unwrap_or_default();
    let encoder = DocEncoder;
    let mut encoded = BTreeMap::new();
    let mut planner = CardLanePlanner {
        root,
        state: state.as_ref(),
        reencode: &reencode,
        encoded: &mut encoded,
        encoder: &encoder,
        spine: &spine,
    };

    let files = vec![
        planner.file(DocPosition::AgentCards, "docs/agents/cards.jsonl"),
        planner.file(DocPosition::CardIndex, "docs/agents/card-index.json"),
        planner.file(DocPosition::HumanReadme, "docs/humans/README.md"),
        GeneratedFile::new(
            "docs/diagrams/src/README.md",
            format!("# Diagram Sources\n\nPlace editable diagram sources for `{repo}` here.\n"),
        ),
        GeneratedFile::new(
            "docs/diagrams/generated/README.md",
            format!(
                "# Generated Diagrams\n\nGenerated diagram images for `{repo}` are written here.\n"
            ),
        ),
        generated_contract_file(&contract_files, "provenance.json")?,
        generated_contract_file(&contract_files, "repo-contract.json")?,
        generated_contract_file(&contract_files, "rustdoc-index.json")?,
        generated_contract_file(&contract_files, "card-index.json")?,
        generated_contract_file(&contract_files, "feature-map.json")?,
    ];
    Ok(ExpectedFiles { spine, files })
}

fn generated_contract_file(
    contract_files: &BTreeMap<&'static str, String>,
    name: &'static str,
) -> Result<GeneratedFile, String> {
    let contents = contract_files
        .get(name)
        .ok_or_else(|| format!("repo-contract generator did not produce {name}"))?;
    Ok(GeneratedFile::new(
        format!("docs/generated/{name}"),
        contents.clone(),
    ))
}

struct CardLanePlanner<'a> {
    root: &'a Path,
    state: Option<&'a CardSpineState>,
    reencode: &'a [DocPosition],
    encoded: &'a mut BTreeMap<DocPosition, String>,
    encoder: &'a DocEncoder,
    spine: &'a CardSpine,
}

impl CardLanePlanner<'_> {
    fn file(&mut self, position: DocPosition, path: &str) -> GeneratedFile {
        if can_reuse_card_lane(self.root, self.state, self.reencode, position, path) {
            return GeneratedFile::skipped(path, position);
        }
        let contents = self
            .encoded
            .entry(position)
            .or_insert_with(|| self.encoder.encode(self.spine, position))
            .clone();
        GeneratedFile::card(path, position, contents)
    }
}

fn can_reuse_card_lane(
    root: &Path,
    state: Option<&CardSpineState>,
    reencode: &[DocPosition],
    position: DocPosition,
    path: &str,
) -> bool {
    let Some(state) = state else {
        return false;
    };
    if reencode.contains(&position) {
        return false;
    }
    let Some(current_digest) = file_lane_digest(root, path) else {
        return false;
    };
    state.lane_digests.get(path) == Some(&current_digest)
}

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
                ".git" | ".meta-workspace" | "target" | "generated-reports" | "split-reports"
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

fn write_files(root: &Path, files: &[GeneratedFile]) -> Result<(), String> {
    for file in files {
        let Some(contents) = &file.contents else {
            continue;
        };
        let path = root.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create {}: {err}", parent.display()))?;
        }
        fs::write(&path, contents).map_err(|err| format!("write {}: {err}", path.display()))?;
    }
    Ok(())
}

fn check_files(root: &Path, files: &[GeneratedFile]) -> Result<(), String> {
    let mut stale = Vec::new();
    for file in files {
        let Some(contents) = &file.contents else {
            continue;
        };
        let path = root.join(&file.path);
        match fs::read_to_string(&path) {
            Ok(current) if current == *contents => {}
            Ok(_) => stale.push(file.path.clone()),
            Err(_) => stale.push(file.path.clone()),
        }
    }
    if stale.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "stale generated doc artifacts: {}; run `cargo run -p xtask -- simdoc`",
            stale.join(", ")
        ))
    }
}

fn persist_state(root: &Path, expected: &ExpectedFiles) -> Result<(), String> {
    let mut lane_digests = BTreeMap::new();
    for file in &expected.files {
        let digest = match &file.contents {
            Some(contents) => lane_digest(contents),
            None => file_lane_digest(root, &file.path)
                .ok_or_else(|| format!("read {} for cardspine state", file.path))?,
        };
        lane_digests.insert(file.path.clone(), digest);
    }
    CardSpineState::from_parts(expected.spine.content_ids(), lane_digests).write(root)
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

struct ExpectedFiles {
    spine: CardSpine,
    files: Vec<GeneratedFile>,
}

impl ExpectedFiles {
    fn card_lanes_encoded(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.position.is_some() && file.contents.is_some())
            .map(|file| file.position.unwrap())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

struct GeneratedFile {
    path: String,
    contents: Option<String>,
    position: Option<DocPosition>,
}

impl GeneratedFile {
    fn new(path: impl Into<String>, contents: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            contents: Some(contents.into()),
            position: None,
        }
    }

    fn card(path: impl Into<String>, position: DocPosition, contents: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            contents: Some(contents.into()),
            position: Some(position),
        }
    }

    fn skipped(path: impl Into<String>, position: DocPosition) -> Self {
        Self {
            path: path.into(),
            contents: None,
            position: Some(position),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{RustdocMode, SimdocOptions};

    #[test]
    fn simdoc_options_accept_repo_root_and_check() {
        let args = vec![
            "xtask".to_owned(),
            "simdoc".to_owned(),
            "--repo-root".to_owned(),
            "../sim-cli".to_owned(),
            "--check".to_owned(),
        ];

        let options = SimdocOptions::parse(&args).unwrap();

        assert_eq!(options.root, PathBuf::from("../sim-cli"));
        assert!(options.check);
        assert_eq!(options.rustdoc, RustdocMode::Auto);
    }

    #[test]
    fn simdoc_options_accept_rustdoc_skip() {
        let args = vec![
            "xtask".to_owned(),
            "simdoc".to_owned(),
            "--check".to_owned(),
            "--rustdoc".to_owned(),
            "skip".to_owned(),
        ];

        let options = SimdocOptions::parse(&args).unwrap();

        assert!(options.check);
        assert_eq!(options.rustdoc, RustdocMode::Skip);
    }

    #[test]
    fn simdoc_options_reject_unknown_flag() {
        let args = vec![
            "xtask".to_owned(),
            "simdoc".to_owned(),
            "--unknown".to_owned(),
        ];

        let err = SimdocOptions::parse(&args).unwrap_err();

        assert!(err.contains("unknown simdoc argument"));
    }
}
