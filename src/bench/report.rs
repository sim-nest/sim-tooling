//! Durable, self-contained benchmark reports written through Table/Dir paths.

use super::{
    BenchContentKey, BenchSpec, MetricDirection,
    compare::{ComparisonReport, ComparisonSample, RobustComparisonPolicy, compare},
    env::{CompatibilityPolicy, EnvironmentProbe},
    run::{Arm, RunPhase, SampleRecord, SampleStatus},
};
use crate::content_digest::content_digest;
use serde::{Deserialize, Serialize};
use sim_table_core::TablePath;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

/// Current complete report schema.
pub const REPORT_SCHEMA_REVISION: u32 = 3;

/// Durable evidence for every scheduled measured attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptRecord {
    /// Comparison arm invoked.
    pub arm: Arm,
    /// Lifecycle phase of the attempt.
    pub phase: RunPhase,
    /// Position in the realized schedule.
    pub schedule_index: u32,
    /// Calibrated count requested by BENCH.
    pub requested_iterations: u64,
    /// Count acknowledged by a valid workload receipt.
    pub executed_iterations: Option<u64>,
    /// Duration retained for the attempt regardless of status.
    pub duration_ns: u64,
    /// Completion, failure, or timeout classification.
    pub status: SampleStatus,
    /// Optional counters retained without aggregation.
    pub counters: std::collections::BTreeMap<String, u64>,
    /// Requested-versus-achieved isolation evidence.
    pub isolation: String,
}

impl From<&SampleRecord> for AttemptRecord {
    fn from(sample: &SampleRecord) -> Self {
        Self {
            arm: sample.arm,
            phase: sample.phase,
            schedule_index: sample.schedule_index,
            requested_iterations: sample.iterations,
            executed_iterations: sample.executed_iterations,
            duration_ns: sample.duration_ns,
            status: sample.status.clone(),
            counters: sample.counters.clone(),
            isolation: "CPU affinity was not requested".into(),
        }
    }
}

/// Raw counters emitted by one measured workload invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterSample {
    /// Position in the interleaved measured schedule.
    pub sample_index: u32,
    /// Comparison arm that produced the counters.
    pub arm: Arm,
    /// Exact named counts emitted by the workload adapter.
    pub counters: std::collections::BTreeMap<String, u64>,
}

/// One self-contained benchmark artifact: declaration, environments, raw data,
/// policies, summaries, exclusions, effects, and decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchReport {
    /// Report schema revision.
    pub schema_revision: u32,
    /// Digest of every other field in canonical codec form.
    pub content_key: BenchContentKey,
    /// Benchmark declaration.
    pub spec: BenchSpec,
    /// Baseline host and build evidence.
    pub baseline_environment: EnvironmentProbe,
    /// Candidate host and build evidence.
    pub candidate_environment: EnvironmentProbe,
    /// Raw, indexed baseline values.
    pub baseline_samples: Vec<ComparisonSample>,
    /// Raw, indexed candidate values.
    pub candidate_samples: Vec<ComparisonSample>,
    /// Unaggregated workload counters retained for attribution.
    pub counter_samples: Vec<CounterSample>,
    /// Every measured attempt, including failed and timed-out rows.
    pub attempts: Vec<AttemptRecord>,
    /// Metric preference used to interpret change.
    pub direction: MetricDirection,
    /// Statistical policy used to derive aggregates.
    pub comparison_policy: RobustComparisonPolicy,
    /// Material environment fields required to match.
    pub environment_policy: CompatibilityPolicy,
    /// Derived summaries, effects, exclusions, and decision.
    pub comparison: ComparisonReport,
}

impl BenchReport {
    /// Derives and seals a report from its raw inputs.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        spec: BenchSpec,
        baseline_environment: EnvironmentProbe,
        candidate_environment: EnvironmentProbe,
        baseline_samples: Vec<ComparisonSample>,
        candidate_samples: Vec<ComparisonSample>,
        direction: MetricDirection,
        comparison_policy: RobustComparisonPolicy,
        environment_policy: CompatibilityPolicy,
    ) -> Result<Self, String> {
        Self::new_attributed(
            spec,
            baseline_environment,
            candidate_environment,
            baseline_samples,
            candidate_samples,
            Vec::new(),
            direction,
            comparison_policy,
            environment_policy,
        )
    }

    /// Derives and seals a report while retaining every measured workload counter.
    #[allow(clippy::too_many_arguments)]
    pub fn new_attributed(
        spec: BenchSpec,
        baseline_environment: EnvironmentProbe,
        candidate_environment: EnvironmentProbe,
        baseline_samples: Vec<ComparisonSample>,
        candidate_samples: Vec<ComparisonSample>,
        counter_samples: Vec<CounterSample>,
        direction: MetricDirection,
        comparison_policy: RobustComparisonPolicy,
        environment_policy: CompatibilityPolicy,
    ) -> Result<Self, String> {
        Self::new_attributed_with_attempts(
            spec,
            baseline_environment,
            candidate_environment,
            baseline_samples,
            candidate_samples,
            counter_samples,
            Vec::new(),
            direction,
            comparison_policy,
            environment_policy,
        )
    }

    /// Derives and seals a report while preserving every measured attempt.
    #[allow(clippy::too_many_arguments)]
    pub fn new_attributed_with_attempts(
        spec: BenchSpec,
        baseline_environment: EnvironmentProbe,
        candidate_environment: EnvironmentProbe,
        baseline_samples: Vec<ComparisonSample>,
        candidate_samples: Vec<ComparisonSample>,
        counter_samples: Vec<CounterSample>,
        attempts: Vec<AttemptRecord>,
        direction: MetricDirection,
        comparison_policy: RobustComparisonPolicy,
        environment_policy: CompatibilityPolicy,
    ) -> Result<Self, String> {
        let comparison = compare(
            &baseline_samples,
            &candidate_samples,
            direction.clone(),
            comparison_policy,
            &environment_policy,
            &baseline_environment,
            &candidate_environment,
        )?;
        let value = Self {
            schema_revision: REPORT_SCHEMA_REVISION,
            content_key: BenchContentKey(String::new()),
            spec,
            baseline_environment,
            candidate_environment,
            baseline_samples,
            candidate_samples,
            counter_samples,
            attempts,
            direction,
            comparison_policy,
            environment_policy,
            comparison,
        };
        // The repository's canonical JSON codec is the persistence boundary.
        // Normalize derived floating-point values through it before sealing so
        // identity is stable even when the codec's decimal parser rounds a
        // computed value to its persisted representation.
        let mut value: Self = serde_json::from_slice(
            &serde_json::to_vec(&value)
                .map_err(|e| format!("normalize report with codec/json: {e}"))?,
        )
        .map_err(|e| format!("normalize report with codec/json: {e}"))?;
        value.content_key = value.expected_key()?;
        Ok(value)
    }

    fn expected_key(&self) -> Result<BenchContentKey, String> {
        let mut identity = self.clone();
        identity.content_key = BenchContentKey(String::new());
        let bytes = serde_json::to_vec(&identity)
            .map_err(|e| format!("encode report identity with codec/json: {e}"))?;
        Ok(BenchContentKey(format!(
            "sha256:{}",
            content_digest(&bytes)
        )))
    }

    /// Verifies content identity and recomputes every aggregate from raw samples.
    pub fn verify(&self) -> Result<(), String> {
        if self.schema_revision != REPORT_SCHEMA_REVISION {
            return Err(format!(
                "unsupported report schema revision {}",
                self.schema_revision
            ));
        }
        let expected_key = self.expected_key()?;
        if self.content_key != expected_key {
            return Err(format!(
                "report content key {} does not match canonical contents {}",
                self.content_key.0, expected_key.0
            ));
        }
        for sample in &self.counter_samples {
            if sample.counters.is_empty() {
                return Err(format!("counter sample {} is empty", sample.sample_index));
            }
            for name in sample.counters.keys() {
                if !self.spec.metrics.iter().any(|metric| metric.name == *name) {
                    return Err(format!(
                        "counter {name} is not declared by the benchmark spec"
                    ));
                }
            }
        }
        for attempt in &self.attempts {
            if matches!(attempt.status, SampleStatus::Completed)
                && attempt.executed_iterations != Some(attempt.requested_iterations)
            {
                return Err(format!(
                    "attempt {} has no exact iteration receipt",
                    attempt.schedule_index
                ));
            }
        }
        let actual = compare(
            &self.baseline_samples,
            &self.candidate_samples,
            self.direction.clone(),
            self.comparison_policy,
            &self.environment_policy,
            &self.baseline_environment,
            &self.candidate_environment,
        )?;
        let actual: ComparisonReport = serde_json::from_slice(
            &serde_json::to_vec(&actual)
                .map_err(|e| format!("normalize comparison with codec/json: {e}"))?,
        )
        .map_err(|e| format!("normalize comparison with codec/json: {e}"))?;
        if actual != self.comparison {
            return Err("comparison aggregates do not match retained raw samples".to_owned());
        }
        Ok(())
    }
}

/// Canonical JSON codec for report artifacts.
pub struct ReportCodec;
impl ReportCodec {
    /// Encodes a verified report.
    pub fn encode(report: &BenchReport) -> Result<Vec<u8>, String> {
        report.verify()?;
        serde_json::to_vec(report).map_err(|e| format!("encode report with codec/json: {e}"))
    }
    /// Decodes, verifies, and requires byte-identical canonical re-encoding.
    pub fn decode(bytes: &[u8]) -> Result<BenchReport, String> {
        let report: BenchReport = serde_json::from_slice(bytes)
            .map_err(|e| format!("decode report with codec/json: {e}"))?;
        report.verify()?;
        if Self::encode(&report)? != bytes {
            return Err("report is not canonical codec output".to_owned());
        }
        Ok(report)
    }
}

/// Atomic artifact operations implemented by a Table/Dir adapter.
pub trait AtomicReportDir {
    /// Atomically replaces an artifact at a canonical Table path.
    fn replace(&self, path: &TablePath, bytes: &[u8]) -> Result<(), String>;
    /// Reads an artifact under a strict byte limit.
    fn read_bounded(&self, path: &TablePath, max_bytes: usize) -> Result<Vec<u8>, String>;
    /// Returns canonical artifact paths under a strict entry limit.
    fn browse(&self, max_entries: usize) -> Result<Vec<TablePath>, String>;
}

/// Writes one report through an injected Table/Dir adapter.
pub fn write_report(
    dir: &impl AtomicReportDir,
    path: &TablePath,
    report: &BenchReport,
) -> Result<(), String> {
    dir.replace(path, &ReportCodec::encode(report)?)
}
/// Reads and verifies one bounded report.
pub fn read_report(
    dir: &impl AtomicReportDir,
    path: &TablePath,
    max_bytes: usize,
) -> Result<BenchReport, String> {
    ReportCodec::decode(&dir.read_bounded(path, max_bytes)?)
}

/// Filesystem Table/Dir adapter using same-directory temp, sync, and rename.
#[derive(Clone, Debug)]
pub struct FsReportDir {
    root: PathBuf,
}
impl FsReportDir {
    /// Opens and canonicalizes a report directory.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        fs::create_dir_all(root.as_ref()).map_err(|e| format!("create report dir: {e}"))?;
        Ok(Self {
            root: fs::canonicalize(root.as_ref())
                .map_err(|e| format!("canonicalize report dir: {e}"))?,
        })
    }
    fn resolve(&self, path: &TablePath) -> Result<PathBuf, String> {
        if path.is_root() {
            return Err("report path must name an artifact".to_owned());
        }
        let mut out = self.root.clone();
        for segment in path.segments() {
            out.push(segment);
        }
        Ok(out)
    }
}
impl AtomicReportDir for FsReportDir {
    fn replace(&self, path: &TablePath, bytes: &[u8]) -> Result<(), String> {
        let target = self.resolve(path)?;
        let parent = target.parent().ok_or("report path has no parent")?;
        fs::create_dir_all(parent).map_err(|e| format!("create report parent: {e}"))?;
        let name = target
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or("invalid report name")?;
        let temp = parent.join(format!(".{name}.{}.tmp", std::process::id()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|e| format!("create report temp: {e}"))?;
            file.write_all(bytes)
                .map_err(|e| format!("write report temp: {e}"))?;
            file.sync_all()
                .map_err(|e| format!("sync report temp: {e}"))?;
            fs::rename(&temp, &target).map_err(|e| format!("replace report: {e}"))?;
            OpenOptions::new()
                .read(true)
                .open(parent)
                .and_then(|f| f.sync_all())
                .map_err(|e| format!("sync report dir: {e}"))
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    fn read_bounded(&self, path: &TablePath, max_bytes: usize) -> Result<Vec<u8>, String> {
        let target = self.resolve(path)?;
        let len = fs::metadata(&target)
            .map_err(|e| format!("inspect report: {e}"))?
            .len();
        if len > max_bytes as u64 {
            return Err(format!(
                "report is {len} bytes; browse limit is {max_bytes}"
            ));
        }
        fs::read(target).map_err(|e| format!("read report: {e}"))
    }
    fn browse(&self, max_entries: usize) -> Result<Vec<TablePath>, String> {
        let mut out = Vec::new();
        browse_paths(&self.root, &self.root, max_entries, &mut out)?;
        out.sort_by_key(TablePath::to_absolute_reference);
        Ok(out)
    }
}
fn browse_paths(
    root: &Path,
    current: &Path,
    limit: usize,
    out: &mut Vec<TablePath>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(current)
        .map_err(|e| format!("browse report dir: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("browse report entry: {e}"))?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            browse_paths(root, &path, limit, out)?;
        } else if !entry.file_name().to_string_lossy().starts_with('.') {
            if out.len() == limit {
                return Err(format!("report browse exceeds {limit} entries"));
            }
            let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
            out.push(
                TablePath::from_segments(relative.iter().map(|p| p.to_string_lossy()))
                    .map_err(|e| format!("invalid report Table path: {e:?}"))?,
            );
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "report/tests.rs"]
mod tests;
