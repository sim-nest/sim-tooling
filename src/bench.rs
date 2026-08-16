//! Declarative benchmark specifications and content-addressed artifact records.
//!
//! These types keep benchmark intent out of command-line flags. Constructors
//! validate the complete record and derive a stable key from canonical JSON.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::content_digest::content_digest;

#[path = "bench/cli.rs"]
pub mod cli;
#[path = "bench/compare.rs"]
pub mod compare;
#[path = "bench/env.rs"]
pub mod env;
#[path = "bench/exec.rs"]
pub mod exec;
#[path = "bench/jvm_baseline.rs"]
pub mod jvm_baseline;
#[path = "bench/report.rs"]
pub mod report;
#[path = "bench/run.rs"]
pub mod run;

#[cfg(test)]
#[path = "bench/neutral_allocation_conformance.rs"]
mod neutral_allocation_conformance;
#[cfg(test)]
#[path = "bench/neutral_cpu_conformance.rs"]
mod neutral_cpu_conformance;
#[cfg(test)]
#[path = "bench/neutral_specimen_fixture.rs"]
mod neutral_specimen_fixture;

/// Current revision of the benchmark artifact schema.
pub const BENCH_SCHEMA_REVISION: u32 = 1;

/// A SHA-256 identity for the semantic contents of a benchmark record.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BenchContentKey(pub String);

/// Identity of the workload being exercised.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkloadIdentity {
    /// Stable workload name.
    pub name: String,
    /// Workload contract revision.
    pub revision: String,
    /// Workload-specific, deterministically ordered parameters.
    pub parameters: BTreeMap<String, Value>,
}

/// Identity of the executable build under measurement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildIdentity {
    /// Source revision, normally a full commit id.
    pub source_revision: String,
    /// Rust target triple.
    pub target: String,
    /// Cargo profile used for the build.
    pub profile: String,
    /// Enabled features, in canonical lexical order.
    pub features: Vec<String>,
    /// Toolchain identity reported by the build.
    pub toolchain: String,
}

/// Unit attached to every reported metric value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetricUnit {
    /// Elapsed nanoseconds.
    Nanoseconds,
    /// Bytes of storage or memory.
    Bytes,
    /// Dimensionless count.
    Count,
    /// Operations completed per second.
    OperationsPerSecond,
    /// Dimensionless ratio, represented as a finite non-negative value.
    Ratio,
}

/// Whether smaller or larger values are preferable for a metric.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetricDirection {
    /// Smaller values are better.
    LowerIsBetter,
    /// Larger values are better.
    HigherIsBetter,
}

/// Declaration of one metric collected by a benchmark.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricSpec {
    /// Stable metric name.
    pub name: String,
    /// Unit of every observation and summary value.
    pub unit: MetricUnit,
    /// Direction used for comparisons.
    pub direction: MetricDirection,
}

/// Reproducible warm-up and measurement schedule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamplingPlan {
    /// Unrecorded samples executed before measurement.
    pub warmup_samples: u32,
    /// Recorded samples. Must be non-zero.
    pub measured_samples: u32,
    /// Deterministic seed supplied to the workload and sampler.
    pub seed: u64,
}

/// Limits used to classify a comparison.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComparisonPolicy {
    /// Maximum permitted relative regression (for example `0.05` for 5%).
    pub required_threshold: Option<f64>,
    /// Minimum relative change worth reporting. Must not exceed the threshold.
    pub noise_floor: f64,
    /// Confidence level required for a decision, in the open interval `(0, 1)`.
    pub confidence_level: f64,
}

/// Host properties required before a benchmark may run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentRequirements {
    /// Required operating-system name, if constrained.
    pub operating_system: Option<String>,
    /// Required CPU architecture, if constrained.
    pub architecture: Option<String>,
    /// Minimum logical CPU count.
    pub minimum_logical_cpus: u32,
    /// Minimum available memory in bytes.
    pub minimum_memory_bytes: u64,
    /// Required host capabilities, in canonical lexical order.
    pub capabilities: Vec<String>,
    /// Whether the run must be isolated from network access.
    pub network_isolation: bool,
}

/// Complete declaration of what a benchmark measures and how it is judged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchSpec {
    /// Artifact schema revision.
    pub schema_revision: u32,
    /// Content key of all semantic fields. Human comments are excluded.
    pub content_key: BenchContentKey,
    /// Workload identity and parameters.
    pub workload: WorkloadIdentity,
    /// Executable build identity.
    pub build: BuildIdentity,
    /// Metrics emitted by each sample.
    pub metrics: Vec<MetricSpec>,
    /// Sampling schedule and seed.
    pub sampling_plan: SamplingPlan,
    /// Comparison and decision limits.
    pub comparison_policy: ComparisonPolicy,
    /// Properties required of the execution environment.
    pub environment: EnvironmentRequirements,
    /// Human annotation; deliberately excluded from the content key.
    pub comment: Option<String>,
}

#[derive(Serialize)]
struct BenchSpecIdentity<'a> {
    schema_revision: u32,
    workload: &'a WorkloadIdentity,
    build: &'a BuildIdentity,
    metrics: &'a [MetricSpec],
    sampling_plan: &'a SamplingPlan,
    comparison_policy: &'a ComparisonPolicy,
    environment: &'a EnvironmentRequirements,
}

impl BenchSpec {
    /// Validates and constructs a content-addressed benchmark specification.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workload: WorkloadIdentity,
        mut build: BuildIdentity,
        metrics: Vec<MetricSpec>,
        sampling_plan: SamplingPlan,
        comparison_policy: ComparisonPolicy,
        mut environment: EnvironmentRequirements,
        comment: Option<String>,
    ) -> Result<Self, String> {
        build.features.sort();
        build.features.dedup();
        environment.capabilities.sort();
        environment.capabilities.dedup();
        let mut spec = Self {
            schema_revision: BENCH_SCHEMA_REVISION,
            content_key: BenchContentKey(String::new()),
            workload,
            build,
            metrics,
            sampling_plan,
            comparison_policy,
            environment,
            comment,
        };
        spec.validate()?;
        spec.content_key = content_key(&spec.identity())?;
        Ok(spec)
    }

    fn identity(&self) -> BenchSpecIdentity<'_> {
        BenchSpecIdentity {
            schema_revision: self.schema_revision,
            workload: &self.workload,
            build: &self.build,
            metrics: &self.metrics,
            sampling_plan: &self.sampling_plan,
            comparison_policy: &self.comparison_policy,
            environment: &self.environment,
        }
    }

    fn validate(&self) -> Result<(), String> {
        let mut errors = Vec::new();
        required("workload.name", &self.workload.name, &mut errors);
        required("workload.revision", &self.workload.revision, &mut errors);
        required(
            "build.source_revision",
            &self.build.source_revision,
            &mut errors,
        );
        required("build.target", &self.build.target, &mut errors);
        required("build.profile", &self.build.profile, &mut errors);
        required("build.toolchain", &self.build.toolchain, &mut errors);
        if self.metrics.is_empty() {
            errors.push("metrics must contain at least one metric".to_owned());
        }
        for (index, metric) in self.metrics.iter().enumerate() {
            required(&format!("metrics[{index}].name"), &metric.name, &mut errors);
        }
        if self.sampling_plan.measured_samples == 0 {
            errors.push("sampling_plan.measured_samples must be greater than zero".to_owned());
        }
        let policy = &self.comparison_policy;
        if !policy.noise_floor.is_finite() || policy.noise_floor < 0.0 {
            errors.push("comparison_policy.noise_floor must be finite and non-negative".to_owned());
        }
        if !policy.confidence_level.is_finite()
            || policy.confidence_level <= 0.0
            || policy.confidence_level >= 1.0
        {
            errors.push(
                "comparison_policy.confidence_level must be finite and between zero and one"
                    .to_owned(),
            );
        }
        if let Some(threshold) = policy.required_threshold {
            if !threshold.is_finite() || threshold < 0.0 {
                errors.push(
                    "comparison_policy.required_threshold must be finite and non-negative"
                        .to_owned(),
                );
            }
            if self.sampling_plan.measured_samples == 0 {
                errors.push("comparison_policy.required_threshold requires sampling_plan.measured_samples greater than zero".to_owned());
            }
            if policy.noise_floor > threshold {
                errors.push("comparison_policy.noise_floor must not exceed comparison_policy.required_threshold".to_owned());
            }
        }
        if self.environment.minimum_logical_cpus == 0 {
            errors.push("environment.minimum_logical_cpus must be greater than zero".to_owned());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

/// One metric value observed during one measured sample.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RawObservation {
    /// Artifact schema revision.
    pub schema_revision: u32,
    /// Content key of this observation.
    pub content_key: BenchContentKey,
    /// Specification that authorized the run.
    pub spec_key: BenchContentKey,
    /// Zero-based measured sample index.
    pub sample_index: u32,
    /// Metric name declared by the specification.
    pub metric: String,
    /// Metric unit declared by the specification.
    pub unit: MetricUnit,
    /// Finite observed value.
    pub value: f64,
}

impl RawObservation {
    /// Validates and constructs an observation.
    pub fn new(
        spec_key: BenchContentKey,
        sample_index: u32,
        metric: String,
        unit: MetricUnit,
        value: f64,
    ) -> Result<Self, String> {
        if metric.trim().is_empty() {
            return Err("metric must not be empty".to_owned());
        }
        if !value.is_finite() {
            return Err("value must be finite".to_owned());
        }
        let mut record = Self {
            schema_revision: BENCH_SCHEMA_REVISION,
            content_key: BenchContentKey(String::new()),
            spec_key,
            sample_index,
            metric,
            unit,
            value,
        };
        record.content_key = content_key(&(
            &record.schema_revision,
            &record.spec_key,
            record.sample_index,
            &record.metric,
            &record.unit,
            record.value,
        ))?;
        Ok(record)
    }
}

/// Statistical summary of the observations for one metric.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchSummary {
    /// Artifact schema revision.
    pub schema_revision: u32,
    /// Content key of this summary.
    pub content_key: BenchContentKey,
    /// Specification summarized.
    pub spec_key: BenchContentKey,
    /// Metric name.
    pub metric: String,
    /// Metric unit.
    pub unit: MetricUnit,
    /// Number of observations represented.
    pub sample_count: u32,
    /// Arithmetic mean.
    pub mean: f64,
    /// Median observation.
    pub median: f64,
    /// Sample standard deviation.
    pub standard_deviation: f64,
}

impl BenchSummary {
    /// Validates and constructs a metric summary.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        spec_key: BenchContentKey,
        metric: String,
        unit: MetricUnit,
        sample_count: u32,
        mean: f64,
        median: f64,
        standard_deviation: f64,
    ) -> Result<Self, String> {
        if metric.trim().is_empty() {
            return Err("metric must not be empty".to_owned());
        }
        if sample_count == 0 {
            return Err("sample_count must be greater than zero".to_owned());
        }
        if !mean.is_finite()
            || !median.is_finite()
            || !standard_deviation.is_finite()
            || standard_deviation < 0.0
        {
            return Err(
                "summary values must be finite and standard_deviation non-negative".to_owned(),
            );
        }
        let mut record = Self {
            schema_revision: BENCH_SCHEMA_REVISION,
            content_key: BenchContentKey(String::new()),
            spec_key,
            metric,
            unit,
            sample_count,
            mean,
            median,
            standard_deviation,
        };
        record.content_key = content_key(&(
            &record.schema_revision,
            &record.spec_key,
            &record.metric,
            &record.unit,
            record.sample_count,
            record.mean,
            record.median,
            record.standard_deviation,
        ))?;
        Ok(record)
    }
}

/// Classification emitted after comparing a candidate summary with a baseline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DecisionOutcome {
    /// Candidate satisfies the required policy.
    Pass,
    /// Candidate violates the required policy.
    Fail,
    /// Evidence is insufficient for a statistically supported decision.
    Inconclusive,
}

/// Content-addressed result of applying a comparison policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchDecision {
    /// Artifact schema revision.
    pub schema_revision: u32,
    /// Content key of this decision.
    pub content_key: BenchContentKey,
    /// Candidate summary being judged.
    pub candidate_summary_key: BenchContentKey,
    /// Baseline summary used for comparison.
    pub baseline_summary_key: BenchContentKey,
    /// Relative change after applying the metric direction.
    pub relative_change: f64,
    /// Statistical confidence of the comparison.
    pub confidence: f64,
    /// Policy outcome.
    pub outcome: DecisionOutcome,
}

impl BenchDecision {
    /// Validates and constructs a comparison decision.
    pub fn new(
        candidate_summary_key: BenchContentKey,
        baseline_summary_key: BenchContentKey,
        relative_change: f64,
        confidence: f64,
        outcome: DecisionOutcome,
    ) -> Result<Self, String> {
        if !relative_change.is_finite() {
            return Err("relative_change must be finite".to_owned());
        }
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err("confidence must be finite and between zero and one".to_owned());
        }
        let mut record = Self {
            schema_revision: BENCH_SCHEMA_REVISION,
            content_key: BenchContentKey(String::new()),
            candidate_summary_key,
            baseline_summary_key,
            relative_change,
            confidence,
            outcome,
        };
        record.content_key = content_key(&(
            &record.schema_revision,
            &record.candidate_summary_key,
            &record.baseline_summary_key,
            record.relative_change,
            record.confidence,
            &record.outcome,
        ))?;
        Ok(record)
    }
}

fn content_key<T: Serialize>(value: &T) -> Result<BenchContentKey, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("encode benchmark identity as canonical JSON: {error}"))?;
    Ok(BenchContentKey(format!(
        "sha256:{}",
        content_digest(&bytes)
    )))
}

fn required(field: &str, value: &str, errors: &mut Vec<String>) {
    if value.trim().is_empty() {
        errors.push(format!("{field} must not be empty"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(samples: u32, seed: u64, comment: &str) -> Result<BenchSpec, String> {
        BenchSpec::new(
            WorkloadIdentity {
                name: "parse-tree".to_owned(),
                revision: "1".to_owned(),
                parameters: BTreeMap::new(),
            },
            BuildIdentity {
                source_revision: "0123456789abcdef".to_owned(),
                target: "x86_64-unknown-linux-gnu".to_owned(),
                profile: "release".to_owned(),
                features: vec!["z".to_owned(), "a".to_owned()],
                toolchain: "rustc 1.90.0".to_owned(),
            },
            vec![MetricSpec {
                name: "latency".to_owned(),
                unit: MetricUnit::Nanoseconds,
                direction: MetricDirection::LowerIsBetter,
            }],
            SamplingPlan {
                warmup_samples: 2,
                measured_samples: samples,
                seed,
            },
            ComparisonPolicy {
                required_threshold: Some(0.05),
                noise_floor: 0.01,
                confidence_level: 0.95,
            },
            EnvironmentRequirements {
                operating_system: Some("linux".to_owned()),
                architecture: Some("x86_64".to_owned()),
                minimum_logical_cpus: 1,
                minimum_memory_bytes: 1024,
                capabilities: vec!["timer".to_owned()],
                network_isolation: true,
            },
            Some(comment.to_owned()),
        )
    }

    #[test]
    fn zero_samples_with_required_threshold_names_both_fields() {
        let error = spec(0, 7, "invalid").unwrap_err();
        assert!(error.contains("sampling_plan.measured_samples"));
        assert!(error.contains("comparison_policy.required_threshold"));
    }

    #[test]
    fn comments_do_not_change_keys_but_seeds_do() {
        let first = spec(10, 7, "first comment").unwrap();
        let second = spec(10, 7, "another comment").unwrap();
        let reseeded = spec(10, 8, "first comment").unwrap();
        assert_eq!(first.content_key, second.content_key);
        assert_ne!(first.content_key, reseeded.content_key);
    }

    #[test]
    fn every_artifact_has_a_revision_and_content_key() {
        let spec = spec(10, 7, "records").unwrap();
        let raw = RawObservation::new(
            spec.content_key.clone(),
            0,
            "latency".to_owned(),
            MetricUnit::Nanoseconds,
            12.0,
        )
        .unwrap();
        let summary = BenchSummary::new(
            spec.content_key,
            "latency".to_owned(),
            MetricUnit::Nanoseconds,
            1,
            12.0,
            12.0,
            0.0,
        )
        .unwrap();
        let decision = BenchDecision::new(
            summary.content_key.clone(),
            summary.content_key.clone(),
            0.0,
            0.99,
            DecisionOutcome::Pass,
        )
        .unwrap();
        for (revision, key) in [
            (raw.schema_revision, raw.content_key),
            (summary.schema_revision, summary.content_key),
            (decision.schema_revision, decision.content_key),
        ] {
            assert_eq!(revision, BENCH_SCHEMA_REVISION);
            assert!(key.0.starts_with("sha256:"));
        }
    }
}
