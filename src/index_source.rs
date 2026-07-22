//! Source lookup for generated SIM Index specimen examples.

use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

use serde_json::Value as JsonValue;
use sim_index_core::DiscoveredSpecimen;
use toml::Value;

/// Resolved source text for a specimen row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SpecimenSource {
    pub(crate) path: String,
    pub(crate) language: String,
    pub(crate) text: String,
}

/// Maps public repo names from `repos.toml` to local checkout roots.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourceResolver {
    repos: BTreeMap<String, PathBuf>,
    enabled: bool,
}

impl SourceResolver {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn from_options(
        control_root: Option<&Path>,
        repos_manifest: Option<&Path>,
    ) -> Result<Self, String> {
        match (control_root, repos_manifest) {
            (None, None) => Ok(Self::empty()),
            (Some(_), None) => Err("--control-root requires --repos-manifest".to_owned()),
            (None, Some(_)) => Err("--repos-manifest requires --control-root".to_owned()),
            (Some(root), Some(manifest)) => Self::from_manifest(root, manifest),
        }
    }

    pub(crate) fn from_manifest(
        control_root: &Path,
        repos_manifest: &Path,
    ) -> Result<Self, String> {
        let text = fs::read_to_string(repos_manifest)
            .map_err(|err| format!("read {}: {err}", repos_manifest.display()))?;
        let value = text
            .parse::<Value>()
            .map_err(|err| format!("parse {}: {err}", repos_manifest.display()))?;
        let rows = value
            .get("repo")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{} has no [[repo]] rows", repos_manifest.display()))?;

        let mut repos = BTreeMap::new();
        for row in rows {
            if row.get("contains_code").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let Some(name) = row.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(local_path) = row.get("local_path").and_then(Value::as_str) else {
                continue;
            };
            repos.insert(name.to_owned(), resolve_path(control_root, local_path));
        }
        Ok(Self {
            repos,
            enabled: true,
        })
    }

    pub(crate) fn source_for(
        &self,
        specimen: &DiscoveredSpecimen,
    ) -> Result<Option<SpecimenSource>, String> {
        if !self.enabled {
            return Ok(None);
        }
        let Some(repo) = repo_from_specimen_id(specimen.id.as_str()) else {
            return Err(format!("cannot infer repo from specimen {}", specimen.id));
        };
        let Some(root) = self.repos.get(repo) else {
            return Err(format!(
                "specimen {} refers to repo {repo}, which is absent from repos.toml",
                specimen.id
            ));
        };
        reject_unsafe_relative_path(&specimen.path)?;
        let path = root.join(&specimen.path);
        let text =
            fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        Ok(Some(SpecimenSource {
            path: specimen.path.clone(),
            language: language_hint(&specimen.path, specimen.language.as_deref()),
            text,
        }))
    }

    pub(crate) fn package_for(&self, repo: &str, path: &str) -> Result<String, String> {
        if !self.enabled {
            return Err(
                "source resolver is disabled; pass --control-root and --repos-manifest".to_owned(),
            );
        }
        let Some(root) = self.repos.get(repo) else {
            return Err(format!("repo {repo} is absent from repos.toml"));
        };
        reject_unsafe_relative_path(path)?;
        let packages = repo_contract_packages(root, repo)?;
        let mut best = Vec::<&SourcePackage>::new();
        let mut best_len = None::<usize>;
        for package in &packages {
            if !package_contains_path(&package.root, path) {
                continue;
            }
            let len = package.root.len();
            match best_len {
                Some(current) if current > len => {}
                Some(current) if current == len => best.push(package),
                _ => {
                    best.clear();
                    best.push(package);
                    best_len = Some(len);
                }
            }
        }
        match best.as_slice() {
            [package] => Ok(package.name.clone()),
            [] => Err(format!(
                "repo {repo} has no generated package root for {path}"
            )),
            packages => Err(format!(
                "repo {repo} maps {path} to multiple generated package roots: {}",
                packages
                    .iter()
                    .map(|package| package.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourcePackage {
    name: String,
    root: String,
}

fn resolve_path(control_root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        control_root.join(path)
    }
}

fn repo_contract_packages(repo_root: &Path, repo: &str) -> Result<Vec<SourcePackage>, String> {
    let path = repo_root.join("docs/generated/repo-contract.json");
    let text =
        fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let value: JsonValue =
        serde_json::from_str(&text).map_err(|err| format!("parse {}: {err}", path.display()))?;
    let packages = value
        .get("packages")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("{} missing packages array", path.display()))?;
    packages
        .iter()
        .enumerate()
        .map(|(index, package)| {
            let label = format!("{} packages[{index}]", path.display());
            let name = package
                .get("name")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| format!("{label} missing name"))?;
            let root = package
                .get("root")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| format!("{label} missing root"))?;
            reject_unsafe_relative_path(root)
                .map_err(|err| format!("{label} has unsafe root for repo {repo}: {err}"))?;
            Ok(SourcePackage {
                name: name.to_owned(),
                root: root.to_owned(),
            })
        })
        .collect()
}

fn package_contains_path(root: &str, path: &str) -> bool {
    root.is_empty() || path == root || path.starts_with(&format!("{root}/"))
}

fn repo_from_specimen_id(id: &str) -> Option<&str> {
    let mut parts = id.split('/');
    match parts.next()? {
        "local" => parts.next(),
        "recipe" | "spec-test" => parts.next(),
        _ => None,
    }
}

fn reject_unsafe_relative_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(format!("unsafe relative path {}", path.display()));
    }
    Ok(())
}

fn language_hint(path: &str, declared: Option<&str>) -> String {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("rs") => "rust".to_owned(),
        Some("toml") => "toml".to_owned(),
        Some("md") => "markdown".to_owned(),
        Some("siml") => "lisp".to_owned(),
        _ => declared.unwrap_or("text").to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use sim_index_core::{SpecimenId, SubjectId};

    use super::*;

    #[test]
    fn resolver_reads_public_repo_specimen_source() {
        let root = temp_root("sim-tooling-index-source");
        let repo = root.join("sim-demo");
        fs::create_dir_all(repo.join("recipes/open")).unwrap();
        fs::write(repo.join("recipes/open/recipe.toml"), "title = \"Open\"\n").unwrap();
        fs::write(
            root.join("repos.toml"),
            "[[repo]]\nname = \"sim-demo\"\ncontains_code = true\nlocal_path = \"sim-demo\"\n",
        )
        .unwrap();
        let resolver = SourceResolver::from_manifest(&root, &root.join("repos.toml")).unwrap();
        let specimen = DiscoveredSpecimen {
            id: SpecimenId::new("recipe/sim-demo/open"),
            subject: SubjectId::new("crate/sim-demo"),
            kind: "recipe".to_owned(),
            path: "recipes/open/recipe.toml".to_owned(),
            language: None,
            runnable: true,
            checked: true,
            checked_by: Some("xtask check-recipes".to_owned()),
            doc_anchor: None,
        };

        let source = resolver.source_for(&specimen).unwrap().unwrap();

        assert_eq!(source.language, "toml");
        assert_eq!(source.text, "title = \"Open\"\n");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_resolver_leaves_specimen_unread() {
        let specimen = DiscoveredSpecimen {
            id: SpecimenId::new("recipe/sim-demo/open"),
            subject: SubjectId::new("crate/sim-demo"),
            kind: "recipe".to_owned(),
            path: "recipes/open/recipe.toml".to_owned(),
            language: None,
            runnable: true,
            checked: true,
            checked_by: None,
            doc_anchor: None,
        };

        assert!(
            SourceResolver::empty()
                .source_for(&specimen)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn package_for_selects_longest_generated_contract_root() {
        let root = temp_root("sim-tooling-package-for");
        let repo = root.join("sim-demo");
        fs::create_dir_all(repo.join("docs/generated")).unwrap();
        fs::write(
            repo.join("docs/generated/repo-contract.json"),
            r#"{
  "schema": "sim.repo-contract.v1",
  "packages": [
    { "name": "sim-demo", "root": "" },
    { "name": "sim-demo-core", "root": "crates/sim-demo-core" },
    { "name": "sim-demo-core-inner", "root": "crates/sim-demo-core/inner" }
  ]
}
"#,
        )
        .unwrap();
        fs::write(
            root.join("repos.toml"),
            "[[repo]]\nname = \"sim-demo\"\ncontains_code = true\nlocal_path = \"sim-demo\"\n",
        )
        .unwrap();
        let resolver = SourceResolver::from_manifest(&root, &root.join("repos.toml")).unwrap();

        assert_eq!(
            resolver
                .package_for("sim-demo", "crates/sim-demo-core/inner/src/lib.rs")
                .unwrap(),
            "sim-demo-core-inner"
        );
        assert_eq!(
            resolver.package_for("sim-demo", "README.md").unwrap(),
            "sim-demo"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn package_for_rejects_unsafe_member_paths() {
        let resolver = SourceResolver {
            repos: BTreeMap::from([("sim-demo".to_owned(), PathBuf::from("."))]),
            enabled: true,
        };

        let err = resolver
            .package_for("sim-demo", "../sim-private/secret.rs")
            .unwrap_err();

        assert!(err.contains("unsafe relative path"));
    }

    fn temp_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("{name}-{}-{stamp}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
}
