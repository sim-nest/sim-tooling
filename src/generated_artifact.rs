use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GeneratedArtifact {
    pub(crate) path: PathBuf,
    pub(crate) bytes: Vec<u8>,
}

impl GeneratedArtifact {
    pub(crate) fn new(path: impl Into<PathBuf>, bytes: impl Into<Vec<u8>>) -> Result<Self, String> {
        let path = path.into();
        validate_relative_path(&path)?;
        Ok(Self {
            path,
            bytes: bytes.into(),
        })
    }

    pub(crate) fn path_str(&self) -> &str {
        self.path
            .to_str()
            .expect("generated artifact paths are validated as UTF-8")
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ArtifactSet {
    pub(crate) artifacts: Vec<GeneratedArtifact>,
}

impl ArtifactSet {
    pub(crate) fn new(mut artifacts: Vec<GeneratedArtifact>) -> Result<Self, String> {
        for artifact in &artifacts {
            validate_relative_path(&artifact.path)?;
        }
        artifacts.sort_by(|left, right| left.path_str().cmp(right.path_str()));

        let mut folded_paths = BTreeMap::<String, &str>::new();
        for artifact in &artifacts {
            let path = artifact.path_str();
            let folded = path.to_lowercase();
            if let Some(previous) = folded_paths.insert(folded, path) {
                if previous == path {
                    return Err(format!("duplicate generated artifact path `{path}`"));
                }
                return Err(format!(
                    "case-fold collision between generated artifact paths `{previous}` and `{path}`"
                ));
            }
        }

        Ok(Self { artifacts })
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &GeneratedArtifact> {
        self.artifacts.iter()
    }
}

fn validate_relative_path(path: &Path) -> Result<(), String> {
    let Some(text) = path.to_str() else {
        return Err("generated artifact path must be valid UTF-8".to_owned());
    };
    if text.is_empty() {
        return Err("generated artifact path must not be empty".to_owned());
    }
    if path.is_absolute() || looks_like_windows_absolute(text) {
        return Err(format!(
            "generated artifact path must be relative: `{text}`"
        ));
    }
    if text.contains('\\') {
        return Err(format!(
            "generated artifact path must use normalized `/` separators: `{text}`"
        ));
    }
    if text.split('/').any(|part| part.is_empty() || part == ".") {
        return Err(format!(
            "generated artifact path must be normalized: `{text}`"
        ));
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "generated artifact path must not traverse: `{text}`"
        ));
    }
    Ok(())
}

fn looks_like_windows_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(path: &str) -> GeneratedArtifact {
        GeneratedArtifact::new(path, path.as_bytes()).unwrap()
    }

    fn unchecked_artifact(path: &str) -> GeneratedArtifact {
        GeneratedArtifact {
            path: PathBuf::from(path),
            bytes: Vec::new(),
        }
    }

    #[test]
    fn artifact_set_orders_paths() {
        let set =
            ArtifactSet::new(vec![artifact("z.md"), artifact("a.md"), artifact("m.md")]).unwrap();

        assert_eq!(
            set.iter()
                .map(GeneratedArtifact::path_str)
                .collect::<Vec<_>>(),
            ["a.md", "m.md", "z.md"]
        );
    }

    #[test]
    fn artifact_rejects_absolute_and_non_normalized_paths() {
        for path in [
            "/tmp/index.md",
            "C:/tmp/index.md",
            "../index.md",
            "notes/../index.md",
            "./index.md",
            "notes//index.md",
            "notes\\index.md",
        ] {
            assert!(
                GeneratedArtifact::new(path, Vec::new()).is_err(),
                "accepted {path}"
            );
        }
    }

    #[test]
    fn artifact_set_rejects_duplicate_and_case_fold_collisions() {
        assert!(ArtifactSet::new(vec![artifact("a.md"), artifact("a.md")]).is_err());
        assert!(
            ArtifactSet::new(vec![artifact("Features/a.md"), artifact("features/a.md")]).is_err()
        );
    }

    #[test]
    fn artifact_set_revalidates_absolute_and_traversing_paths() {
        for path in ["/tmp/index.md", "C:/tmp/index.md", "../index.md"] {
            assert!(
                ArtifactSet::new(vec![unchecked_artifact(path)]).is_err(),
                "accepted {path}"
            );
        }
    }
}
