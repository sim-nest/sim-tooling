#![allow(dead_code)]

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::{content_digest::content_digest, generated_artifact::ArtifactSet};

pub(crate) const MANIFEST_FILE: &str = ".sim-index-vault-manifest.json";
const MANIFEST_SCHEMA_V1: &str = "sim.index-vault-manifest.v1";
const MANIFEST_SCHEMA_V2: &str = "sim.index-vault-manifest.v2";
const CODEC_ID: &str = "codec/index-vault";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LegacyVaultManifest {
    pub(crate) namespace: String,
    pub(crate) profile: String,
    pub(crate) granularity: String,
    pub(crate) index_digest: String,
    pub(crate) coverage: BTreeMap<String, u64>,
    pub(crate) artifacts: BTreeMap<String, String>,
}

impl LegacyVaultManifest {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let value: Value =
            serde_json::from_slice(bytes).map_err(|e| format!("parse v1 manifest: {e}"))?;
        let object = value
            .as_object()
            .ok_or("v1 manifest root must be a JSON object")?;
        let allowed = [
            "schema",
            "namespace",
            "profile",
            "granularity",
            "index_digest",
            "coverage",
            "artifacts",
        ];
        if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
            return Err(format!("unknown required v1 manifest field `{key}`"));
        }
        if string_field(object, "schema")? != MANIFEST_SCHEMA_V1 {
            return Err("manifest is not schema v1".into());
        }
        let result = Self {
            namespace: string_field(object, "namespace")?.into(),
            profile: string_field(object, "profile")?.into(),
            granularity: string_field(object, "granularity")?.into(),
            index_digest: string_field(object, "index_digest")?.into(),
            coverage: number_map_field(object, "coverage")?,
            artifacts: string_map_field(object, "artifacts")?,
        };
        reject_empty("namespace", &result.namespace)?;
        reject_empty("profile", &result.profile)?;
        validate_digest("index_digest", &result.index_digest)?;
        for (path, digest) in &result.artifacts {
            reject_empty("artifact path", path)?;
            validate_digest(path, digest)?;
        }
        Ok(result)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultManifestSeed {
    pub(crate) profile: String,
    pub(crate) granularity: String,
    pub(crate) index_digest: String,
    pub(crate) projection_digest: String,
    pub(crate) bundle_root: String,
    pub(crate) coverage: BTreeMap<String, u64>,
}

impl VaultManifestSeed {
    pub(crate) fn new(
        profile: impl Into<String>,
        granularity: impl Into<String>,
        index_digest: impl Into<String>,
        coverage: BTreeMap<String, u64>,
        projection_digest: impl Into<String>,
        bundle_root: impl Into<String>,
    ) -> Result<Self, String> {
        let seed = Self {
            profile: profile.into(),
            granularity: granularity.into(),
            index_digest: index_digest.into(),
            coverage,
            projection_digest: projection_digest.into(),
            bundle_root: bundle_root.into(),
        };
        reject_empty("profile", &seed.profile)?;
        reject_empty("granularity", &seed.granularity)?;
        validate_digest("index_digest", &seed.index_digest)?;
        validate_digest("projection_digest", &seed.projection_digest)?;
        validate_digest("bundle_root", &seed.bundle_root)?;
        Ok(seed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VaultManifest {
    pub(crate) namespace: String,
    pub(crate) profile: String,
    pub(crate) granularity: String,
    pub(crate) index_digest: String,
    pub(crate) codec: String,
    pub(crate) projection_digest: String,
    pub(crate) bundle_root: String,
    pub(crate) coverage: BTreeMap<String, u64>,
    pub(crate) artifacts: BTreeMap<String, String>,
}

impl VaultManifest {
    pub(crate) fn for_artifacts(
        namespace: impl Into<String>,
        seed: &VaultManifestSeed,
        artifacts: &ArtifactSet,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            profile: seed.profile.clone(),
            granularity: seed.granularity.clone(),
            index_digest: seed.index_digest.clone(),
            codec: CODEC_ID.to_owned(),
            projection_digest: seed.projection_digest.clone(),
            bundle_root: seed.bundle_root.clone(),
            coverage: seed.coverage.clone(),
            artifacts: artifacts
                .iter()
                .map(|artifact| {
                    (
                        artifact.path_str().to_owned(),
                        sha256_digest(&artifact.bytes),
                    )
                })
                .collect(),
        }
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let value = serde_json::from_slice::<Value>(bytes)
            .map_err(|err| format!("parse manifest: {err}"))?;
        let object = value
            .as_object()
            .ok_or("manifest root must be a JSON object")?;
        let schema = string_field(object, "schema")?;
        if schema == MANIFEST_SCHEMA_V1 {
            return Err("managed namespace uses read-only manifest v1; rerun with --migrate-profile <exact-v1-profile> and a matching --profile v2 target".into());
        }
        if schema != MANIFEST_SCHEMA_V2 {
            return Err(format!("unsupported manifest schema `{schema}`"));
        }
        let allowed = [
            "schema",
            "codec",
            "profile",
            "granularity",
            "index_digest",
            "projection_digest",
            "bundle_root",
            "namespace",
            "coverage",
            "artifacts",
        ];
        if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
            return Err(format!("unknown required v2 manifest field `{key}`"));
        }
        let manifest = Self {
            namespace: string_field(object, "namespace")?.to_owned(),
            profile: string_field(object, "profile")?.to_owned(),
            granularity: string_field(object, "granularity")?.to_owned(),
            index_digest: string_field(object, "index_digest")?.to_owned(),
            codec: string_field(object, "codec")?.to_owned(),
            projection_digest: string_field(object, "projection_digest")?.to_owned(),
            bundle_root: string_field(object, "bundle_root")?.to_owned(),
            coverage: number_map_field(object, "coverage")?,
            artifacts: string_map_field(object, "artifacts")?,
        };
        manifest.validate_shape()?;
        Ok(manifest)
    }

    pub(crate) fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate_shape()?;
        let mut text = serde_json::to_string_pretty(&json!({
            "schema": MANIFEST_SCHEMA_V2,
            "codec": self.codec,
            "profile": self.profile,
            "granularity": self.granularity,
            "index_digest": self.index_digest,
            "projection_digest": self.projection_digest,
            "bundle_root": self.bundle_root,
            "namespace": self.namespace,
            "coverage": self.coverage,
            "artifacts": self.artifacts,
        }))
        .map_err(|err| format!("serialize manifest: {err}"))?;
        text.push('\n');
        Ok(text.into_bytes())
    }

    pub(crate) fn validate_owner(&self, expected: &Self) -> Result<(), String> {
        if self.namespace != expected.namespace {
            return Err(format!(
                "managed namespace manifest names `{}`, expected `{}`",
                self.namespace, expected.namespace
            ));
        }
        if self.profile != expected.profile {
            return Err(format!(
                "managed namespace profile is `{}`, expected `{}`",
                self.profile, expected.profile
            ));
        }
        if self.granularity != expected.granularity {
            return Err(format!(
                "managed namespace granularity is `{}`, expected `{}`",
                self.granularity, expected.granularity
            ));
        }
        Ok(())
    }

    fn validate_shape(&self) -> Result<(), String> {
        reject_empty("namespace", &self.namespace)?;
        reject_empty("profile", &self.profile)?;
        reject_empty("granularity", &self.granularity)?;
        validate_digest("index_digest", &self.index_digest)?;
        if self.codec != CODEC_ID {
            return Err(format!("unsupported manifest codec `{}`", self.codec));
        }
        validate_digest("projection_digest", &self.projection_digest)?;
        validate_digest("bundle_root", &self.bundle_root)?;
        for (path, digest) in &self.artifacts {
            reject_empty("artifact path", path)?;
            validate_digest(path, digest)?;
        }
        Ok(())
    }
}

pub(crate) fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", content_digest(bytes))
}

fn reject_empty(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("manifest {name} must not be empty"));
    }
    Ok(())
}

fn validate_digest(name: &str, digest: &str) -> Result<(), String> {
    let Some(hex) = digest.strip_prefix("sha256:") else {
        return Err(format!("manifest {name} digest must start with `sha256:`"));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "manifest {name} digest must be a lowercase sha256 hex digest"
        ));
    }
    if hex.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(format!(
            "manifest {name} digest must be a lowercase sha256 hex digest"
        ));
    }
    Ok(())
}

fn string_field<'a>(object: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("manifest field `{name}` must be a string"))
}

fn string_map_field(
    object: &Map<String, Value>,
    name: &str,
) -> Result<BTreeMap<String, String>, String> {
    object_field(object, name)?
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_owned()))
                .ok_or_else(|| format!("manifest `{name}.{key}` must be a string"))
        })
        .collect()
}

fn number_map_field(
    object: &Map<String, Value>,
    name: &str,
) -> Result<BTreeMap<String, u64>, String> {
    object_field(object, name)?
        .iter()
        .map(|(key, value)| {
            value
                .as_u64()
                .map(|value| (key.clone(), value))
                .ok_or_else(|| format!("manifest `{name}.{key}` must be an unsigned integer"))
        })
        .collect()
}

fn object_field<'a>(
    object: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a Map<String, Value>, String> {
    object
        .get(name)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("manifest field `{name}` must be an object"))
}
