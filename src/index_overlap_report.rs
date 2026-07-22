//! Parser for the structured source-overlap report.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

const REPORT_SCHEMA: &str = "sim.overlap-report/v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OverlapReport {
    pub(crate) complete: bool,
    pub(crate) roots_scanned: u64,
    pub(crate) clusters: Vec<CloneCluster>,
}

impl OverlapReport {
    fn empty() -> Self {
        Self {
            complete: false,
            roots_scanned: 0,
            clusters: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CloneCluster {
    pub(crate) id: String,
    pub(crate) owner: String,
    pub(crate) replacement: String,
    pub(crate) members: Vec<OverlapMember>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OverlapMember {
    pub(crate) repo: String,
    pub(crate) path: String,
    pub(crate) line: u64,
    pub(crate) symbol: String,
    pub(crate) classification: SourceClassification,
    pub(crate) reason: Option<String>,
    pub(crate) owner: String,
    pub(crate) replacement: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceClassification {
    Candidate,
    Keep,
    Delegated,
    Regression,
}

impl SourceClassification {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "candidate" => Ok(Self::Candidate),
            "keep" => Ok(Self::Keep),
            "delegated" => Ok(Self::Delegated),
            "regression" => Ok(Self::Regression),
            other => Err(format!("unknown source classification {other}")),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Keep => "keep",
            Self::Delegated => "delegated",
            Self::Regression => "regression",
        }
    }
}

pub(crate) fn read_overlap_report(
    clusters: Option<&PathBuf>,
    strict: bool,
) -> Result<OverlapReport, String> {
    let Some(path) = clusters.map(PathBuf::as_path) else {
        if strict {
            return Err("index overlap --strict requires --clusters <report.json>".to_owned());
        }
        return Ok(OverlapReport::empty());
    };
    let report = read_report(path)?;
    if !report.complete {
        return Err(format!(
            "{} is not a complete overlap report",
            path.display()
        ));
    }
    if report.roots_scanned == 0 {
        return Err(format!("{} scanned zero source roots", path.display()));
    }
    Ok(report)
}

fn read_report(path: &Path) -> Result<OverlapReport, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|err| format!("parse {}: {err}", path.display()))?;
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} missing schema", path.display()))?;
    if schema != REPORT_SCHEMA {
        return Err(format!(
            "{} has schema {schema}, expected {REPORT_SCHEMA}",
            path.display()
        ));
    }
    let complete = value
        .get("complete")
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{} missing complete boolean", path.display()))?;
    let roots_scanned = value
        .get("roots_scanned")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{} missing roots_scanned count", path.display()))?;
    let clusters = value
        .get("clusters")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{} missing clusters array", path.display()))?
        .iter()
        .enumerate()
        .map(|(index, cluster)| read_cluster(index, cluster))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OverlapReport {
        complete,
        roots_scanned,
        clusters,
    })
}

fn read_cluster(index: usize, cluster: &Value) -> Result<CloneCluster, String> {
    let id = required_string(cluster, "id", &format!("clusters[{index}]"))?;
    let owner = required_string(cluster, "owner", &id)?;
    let replacement = required_string(cluster, "replacement", &id)?;
    let members = cluster
        .get("members")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{id} missing members array"))?
        .iter()
        .enumerate()
        .map(|(member_index, member)| read_member(&id, member_index, member, &owner, &replacement))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CloneCluster {
        id,
        owner,
        replacement,
        members,
    })
}

fn read_member(
    cluster: &str,
    index: usize,
    member: &Value,
    cluster_owner: &str,
    cluster_replacement: &str,
) -> Result<OverlapMember, String> {
    let label = format!("{cluster} members[{index}]");
    let repo = required_string(member, "repo", &label)?;
    let path = required_string(member, "path", &label)?;
    let line = member
        .get("line")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{label} missing line"))?;
    let symbol = required_string(member, "symbol", &label)?;
    let classification =
        SourceClassification::parse(&required_string(member, "classification", &label)?)?;
    let reason = member
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if matches!(
        classification,
        SourceClassification::Keep | SourceClassification::Delegated
    ) && reason.as_deref().unwrap_or("").trim().is_empty()
    {
        return Err(format!("{label} classification requires a reason"));
    }
    Ok(OverlapMember {
        repo,
        path,
        line,
        symbol,
        classification,
        reason,
        owner: optional_string(member, "owner").unwrap_or_else(|| cluster_owner.to_owned()),
        replacement: optional_string(member, "replacement")
            .unwrap_or_else(|| cluster_replacement.to_owned()),
    })
}

fn required_string(value: &Value, field: &str, label: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{label} missing {field}"))
}

fn optional_string(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_owned)
}
