//! Auditable policy application for robust benchmark comparisons.
//!
//! Statistical formulas remain in `sim-lib-numbers-stats`. This module owns
//! only sample alignment, declared exclusion policy, environment admission,
//! and the final benchmark threshold decision.

use serde::{Deserialize, Serialize};
use sim_lib_numbers_stats::{
    BootstrapControl, bootstrap_mean_difference_interval, exact_quantile, mean,
    median_absolute_deviation, sample_variance,
};

use super::env::{
    CompatibilityMismatch, CompatibilityPolicy, EnvironmentProbe, compatibility_mismatches,
};
use super::{DecisionOutcome, MetricDirection};

/// A measured value with the stable position needed to audit pairing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComparisonSample {
    /// Measured sample index retained in the raw run artifact.
    pub sample_index: u32,
    /// Finite metric value.
    pub value: f64,
}

/// Declared robust-comparison rules.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RobustComparisonPolicy {
    /// Minimum retained observations required on each side.
    pub minimum_samples: usize,
    /// Maximum admitted MAD divided by the absolute median.
    pub maximum_relative_dispersion: f64,
    /// Exclude observations farther than this many MADs from their median.
    pub outlier_mad_multiplier: Option<f64>,
    /// Maximum permitted direction-adjusted relative regression.
    pub required_threshold: f64,
    /// Deterministic bootstrap interval mass.
    pub confidence_level: f64,
    /// Deterministic bootstrap seed.
    pub bootstrap_seed: u64,
    /// Number of bootstrap resamples.
    pub bootstrap_resamples: usize,
    /// Hard bound on bootstrap sampled observations.
    pub bootstrap_max_work: u64,
}

/// The side from which an observation was excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SampleSide {
    /// Baseline run.
    Baseline,
    /// Candidate run.
    Candidate,
}

/// One excluded observation and the exact declared rule that excluded it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExcludedSample {
    /// Source run.
    pub side: SampleSide,
    /// Stable raw sample index.
    pub sample_index: u32,
    /// Original value retained for inspection.
    pub value: f64,
    /// Stable rule name.
    pub rule: String,
    /// Concrete rule evaluation.
    pub reason: String,
}

/// Statistics computed by `sim-lib-numbers-stats` for retained observations.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RobustSummary {
    /// Retained observation count.
    pub sample_count: usize,
    /// Arithmetic mean.
    pub mean: f64,
    /// Exact median.
    pub median: f64,
    /// Sample variance.
    pub sample_variance: f64,
    /// Raw median absolute deviation.
    pub median_absolute_deviation: f64,
    /// MAD divided by absolute median; infinity when a non-zero MAD has zero median.
    pub relative_dispersion: f64,
}

/// Absolute effect and deterministic uncertainty in source units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectEstimate {
    /// Candidate-minus-baseline point effect.
    pub point: f64,
    /// Lower endpoint of the deterministic percentile interval.
    pub lower: f64,
    /// Upper endpoint of the deterministic percentile interval.
    pub upper: f64,
    /// Requested central interval mass.
    pub confidence_level: f64,
}

/// Evidence sufficient to defend a comparison decision without raw recomputation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComparisonReport {
    /// Baseline retained-sample summary, when enough valid input remained.
    pub baseline: Option<RobustSummary>,
    /// Candidate retained-sample summary, when enough valid input remained.
    pub candidate: Option<RobustSummary>,
    /// Effect over independently resampled interleaved runs.
    pub interleaved_effect: Option<EffectEstimate>,
    /// Effect over sample-index-aligned pairs.
    pub paired_effect: Option<EffectEstimate>,
    /// Direction-adjusted relative regression used by threshold policy.
    pub relative_regression: Option<f64>,
    /// Every raw observation excluded by declared policy.
    pub excluded_samples: Vec<ExcludedSample>,
    /// Every environment field that refused admission.
    pub environment_mismatches: Vec<CompatibilityMismatch>,
    /// Stable reasons why a threshold decision was impossible.
    pub inconclusive_reasons: Vec<String>,
    /// Pass, fail, or explicitly inconclusive.
    pub outcome: DecisionOutcome,
}

/// Applies robust comparison policy to two raw sample sets.
#[allow(clippy::too_many_arguments)]
pub fn compare(
    baseline: &[ComparisonSample],
    candidate: &[ComparisonSample],
    direction: MetricDirection,
    policy: RobustComparisonPolicy,
    environment_policy: &CompatibilityPolicy,
    baseline_environment: &EnvironmentProbe,
    candidate_environment: &EnvironmentProbe,
) -> Result<ComparisonReport, String> {
    validate_policy(policy)?;
    validate_samples("baseline", baseline)?;
    validate_samples("candidate", candidate)?;

    let environment_mismatches = compatibility_mismatches(
        environment_policy,
        baseline_environment,
        candidate_environment,
    );
    let (baseline, mut excluded_samples) = exclude_outliers(
        SampleSide::Baseline,
        baseline,
        policy.outlier_mad_multiplier,
    )?;
    let (candidate, candidate_exclusions) = exclude_outliers(
        SampleSide::Candidate,
        candidate,
        policy.outlier_mad_multiplier,
    )?;
    excluded_samples.extend(candidate_exclusions);

    let baseline_summary = summarize(&baseline)?;
    let candidate_summary = summarize(&candidate)?;
    let mut inconclusive_reasons = Vec::new();
    if baseline.len() < policy.minimum_samples {
        inconclusive_reasons.push(format!(
            "minimum-samples: baseline retained {} but requires {}",
            baseline.len(),
            policy.minimum_samples
        ));
    }
    if candidate.len() < policy.minimum_samples {
        inconclusive_reasons.push(format!(
            "minimum-samples: candidate retained {} but requires {}",
            candidate.len(),
            policy.minimum_samples
        ));
    }
    for (side, summary) in [
        ("baseline", baseline_summary),
        ("candidate", candidate_summary),
    ] {
        if let Some(summary) = summary
            && summary.relative_dispersion > policy.maximum_relative_dispersion
        {
            inconclusive_reasons.push(format!(
                "maximum-relative-dispersion: {side} {} exceeds {}",
                summary.relative_dispersion, policy.maximum_relative_dispersion
            ));
        }
    }
    if !environment_mismatches.is_empty() {
        inconclusive_reasons.push(
            "environment-compatibility: required fields differ or are unavailable".to_owned(),
        );
    }

    let control = BootstrapControl::new(
        policy.bootstrap_seed,
        policy.bootstrap_resamples,
        policy.confidence_level,
        policy.bootstrap_max_work,
    )
    .map_err(|error| error.to_string())?;
    let interleaved_effect = effect(&baseline, &candidate, control)?;
    let (paired_zero, paired_differences) = aligned_pair_differences(&baseline, &candidate);
    let paired_effect = effect(&paired_zero, &paired_differences, control)?;
    if paired_zero.len() < policy.minimum_samples {
        inconclusive_reasons.push(format!(
            "minimum-pairs: retained {} aligned pairs but requires {}",
            paired_zero.len(),
            policy.minimum_samples
        ));
    }

    let relative_regression = match (baseline_summary, candidate_summary) {
        (Some(baseline), Some(candidate))
            if match direction {
                MetricDirection::LowerIsBetter => baseline.mean != 0.0,
                MetricDirection::HigherIsBetter => candidate.mean != 0.0,
            } =>
        {
            Some(match direction {
                MetricDirection::LowerIsBetter => candidate.mean / baseline.mean - 1.0,
                MetricDirection::HigherIsBetter => baseline.mean / candidate.mean - 1.0,
            })
        }
        (Some(_), Some(_)) => {
            inconclusive_reasons.push("relative-effect: reference mean is zero".to_owned());
            None
        }
        _ => None,
    };

    let outcome = if !inconclusive_reasons.is_empty() {
        DecisionOutcome::Inconclusive
    } else if relative_regression.is_some_and(|effect| effect > policy.required_threshold) {
        DecisionOutcome::Fail
    } else {
        DecisionOutcome::Pass
    };

    Ok(ComparisonReport {
        baseline: baseline_summary,
        candidate: candidate_summary,
        interleaved_effect,
        paired_effect,
        relative_regression,
        excluded_samples,
        environment_mismatches,
        inconclusive_reasons,
        outcome,
    })
}

fn validate_policy(policy: RobustComparisonPolicy) -> Result<(), String> {
    if policy.minimum_samples < 2 {
        return Err("minimum_samples must be at least two".to_owned());
    }
    for (name, value) in [
        (
            "maximum_relative_dispersion",
            policy.maximum_relative_dispersion,
        ),
        ("required_threshold", policy.required_threshold),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(format!("{name} must be finite and non-negative"));
        }
    }
    if policy
        .outlier_mad_multiplier
        .is_some_and(|value| !value.is_finite() || value <= 0.0)
    {
        return Err("outlier_mad_multiplier must be finite and positive".to_owned());
    }
    Ok(())
}

fn validate_samples(name: &str, samples: &[ComparisonSample]) -> Result<(), String> {
    for (position, sample) in samples.iter().enumerate() {
        if !sample.value.is_finite() {
            return Err(format!("{name}[{position}] value must be finite"));
        }
        if position > 0 && samples[position - 1].sample_index >= sample.sample_index {
            return Err(format!("{name} sample indices must be strictly increasing"));
        }
    }
    Ok(())
}

fn exclude_outliers(
    side: SampleSide,
    samples: &[ComparisonSample],
    multiplier: Option<f64>,
) -> Result<(Vec<ComparisonSample>, Vec<ExcludedSample>), String> {
    let Some(multiplier) = multiplier else {
        return Ok((samples.to_vec(), Vec::new()));
    };
    if samples.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let values = values(samples);
    let median = exact_quantile(&values, 0.5).map_err(|error| error.to_string())?;
    let mad = median_absolute_deviation(&values).map_err(|error| error.to_string())?;
    let limit = multiplier * mad;
    let mut retained = Vec::new();
    let mut excluded = Vec::new();
    for sample in samples {
        let deviation = (sample.value - median).abs();
        if deviation > limit {
            excluded.push(ExcludedSample {
                side,
                sample_index: sample.sample_index,
                value: sample.value,
                rule: "median-absolute-deviation".to_owned(),
                reason: format!(
                    "absolute deviation {deviation} exceeds {multiplier} * MAD {mad} (limit {limit})"
                ),
            });
        } else {
            retained.push(*sample);
        }
    }
    Ok((retained, excluded))
}

fn summarize(samples: &[ComparisonSample]) -> Result<Option<RobustSummary>, String> {
    if samples.len() < 2 {
        return Ok(None);
    }
    let values = values(samples);
    let mean = mean(&values).map_err(|error| error.to_string())?;
    let median = exact_quantile(&values, 0.5).map_err(|error| error.to_string())?;
    let sample_variance = sample_variance(&values).map_err(|error| error.to_string())?;
    let mad = median_absolute_deviation(&values).map_err(|error| error.to_string())?;
    let relative_dispersion = if median == 0.0 {
        if mad == 0.0 { 0.0 } else { f64::INFINITY }
    } else {
        mad / median.abs()
    };
    Ok(Some(RobustSummary {
        sample_count: values.len(),
        mean,
        median,
        sample_variance,
        median_absolute_deviation: mad,
        relative_dispersion,
    }))
}

fn effect(
    baseline: &[ComparisonSample],
    candidate: &[ComparisonSample],
    control: BootstrapControl,
) -> Result<Option<EffectEstimate>, String> {
    if baseline.is_empty() || candidate.is_empty() {
        return Ok(None);
    }
    let interval =
        bootstrap_mean_difference_interval(&values(baseline), &values(candidate), control)
            .map_err(|error| error.to_string())?;
    Ok(Some(EffectEstimate {
        point: interval.point_effect,
        lower: interval.lower,
        upper: interval.upper,
        confidence_level: interval.confidence_level,
    }))
}

fn aligned_pair_differences(
    baseline: &[ComparisonSample],
    candidate: &[ComparisonSample],
) -> (Vec<ComparisonSample>, Vec<ComparisonSample>) {
    let mut zero = Vec::new();
    let mut differences = Vec::new();
    let mut baseline = baseline.iter().peekable();
    let mut candidate = candidate.iter().peekable();
    while let (Some(a), Some(b)) = (baseline.peek(), candidate.peek()) {
        match a.sample_index.cmp(&b.sample_index) {
            std::cmp::Ordering::Less => {
                baseline.next();
            }
            std::cmp::Ordering::Greater => {
                candidate.next();
            }
            std::cmp::Ordering::Equal => {
                zero.push(ComparisonSample {
                    sample_index: a.sample_index,
                    value: 0.0,
                });
                differences.push(ComparisonSample {
                    sample_index: a.sample_index,
                    value: b.value - a.value,
                });
                baseline.next();
                candidate.next();
            }
        }
    }
    (zero, differences)
}

fn values(samples: &[ComparisonSample]) -> Vec<f64> {
    samples.iter().map(|sample| sample.value).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::BuildIdentity;
    use crate::bench::env::{DeclaredHost, EnvironmentField, HostProbeSource, probe_environment};

    struct Host;
    impl HostProbeSource for Host {
        fn read(&self, path: &str) -> Result<String, String> {
            Ok(match path {
                "/etc/os-release" => "ID=sim-os\n".to_owned(),
                "/proc/cpuinfo" => "model name: stable cpu\n".to_owned(),
                "/proc/meminfo" => "MemTotal: 1048576 kB\n".to_owned(),
                _ => "performance\n".to_owned(),
            })
        }
        fn architecture(&self) -> Result<String, String> {
            Ok("x86_64".to_owned())
        }
        fn logical_cpus(&self) -> Result<u32, String> {
            Ok(8)
        }
    }

    fn environment() -> EnvironmentProbe {
        probe_environment(
            DeclaredHost::new("bench-1".to_owned(), "bench-1".to_owned()).unwrap(),
            &BuildIdentity {
                source_revision: "revision".to_owned(),
                target: "target".to_owned(),
                profile: "release".to_owned(),
                features: vec![],
                toolchain: "stable".to_owned(),
            },
            &Host,
        )
    }

    fn policy(maximum_relative_dispersion: f64) -> RobustComparisonPolicy {
        RobustComparisonPolicy {
            minimum_samples: 3,
            maximum_relative_dispersion,
            outlier_mad_multiplier: Some(3.0),
            required_threshold: 0.05,
            confidence_level: 0.95,
            bootstrap_seed: 7,
            bootstrap_resamples: 128,
            bootstrap_max_work: 10_000,
        }
    }

    fn samples(values: &[f64]) -> Vec<ComparisonSample> {
        values
            .iter()
            .enumerate()
            .map(|(index, value)| ComparisonSample {
                sample_index: index as u32,
                value: *value,
            })
            .collect()
    }

    #[test]
    fn excluded_outlier_is_reported_with_its_rule() {
        let environment = environment();
        let report = compare(
            &samples(&[100.0, 101.0, 99.0, 100.0]),
            &samples(&[100.0, 101.0, 99.0, 10_000.0]),
            MetricDirection::LowerIsBetter,
            policy(0.2),
            &CompatibilityPolicy::requiring([EnvironmentField::CpuModel]),
            &environment,
            &environment,
        )
        .unwrap();
        assert_eq!(report.excluded_samples.len(), 1);
        assert_eq!(report.excluded_samples[0].sample_index, 3);
        assert_eq!(report.excluded_samples[0].rule, "median-absolute-deviation");
        assert!(report.excluded_samples[0].reason.contains("MAD"));
    }

    #[test]
    fn high_dispersion_is_inconclusive_instead_of_marginal_pass() {
        let environment = environment();
        let report = compare(
            &samples(&[50.0, 100.0, 150.0, 100.0]),
            &samples(&[49.0, 99.0, 149.0, 99.0]),
            MetricDirection::LowerIsBetter,
            RobustComparisonPolicy {
                outlier_mad_multiplier: None,
                ..policy(0.1)
            },
            &CompatibilityPolicy::requiring([EnvironmentField::CpuModel]),
            &environment,
            &environment,
        )
        .unwrap();
        assert_eq!(report.outcome, DecisionOutcome::Inconclusive);
        assert!(
            report
                .inconclusive_reasons
                .iter()
                .any(|reason| reason.starts_with("maximum-relative-dispersion:"))
        );
        assert!(report.relative_regression.unwrap() < 0.05);
    }

    #[test]
    fn incompatible_environment_never_reaches_threshold_decision() {
        let baseline = environment();
        let mut candidate = environment();
        candidate.host.host.inventory_id = "bench-2".to_owned();
        let report = compare(
            &samples(&[100.0, 100.0, 100.0]),
            &samples(&[90.0, 90.0, 90.0]),
            MetricDirection::LowerIsBetter,
            policy(0.2),
            &CompatibilityPolicy::requiring([EnvironmentField::HostInventoryId]),
            &baseline,
            &candidate,
        )
        .unwrap();
        assert_eq!(report.outcome, DecisionOutcome::Inconclusive);
        assert_eq!(report.environment_mismatches.len(), 1);
    }

    #[test]
    fn source_fact_guard_keeps_statistical_formulas_in_the_owner() {
        let source = include_str!("compare.rs");
        let production = source
            .split_once("#[cfg(test)]")
            .expect("comparison module retains a distinct test section")
            .0;
        for required_owner_call in [
            "mean(&values)",
            "sample_variance(&values)",
            "exact_quantile(&values",
            "median_absolute_deviation(&values)",
            "bootstrap_mean_difference_interval(",
        ] {
            assert!(
                production.contains(required_owner_call),
                "missing statistics-owner call {required_owner_call}"
            );
        }
        for forbidden_local_formula in [".sum::<f64>()", ".sqrt()", "SplitMix", "resampled_mean"] {
            assert!(
                !production.contains(forbidden_local_formula),
                "tooling contains forbidden statistical formula marker {forbidden_local_formula}"
            );
        }
    }
}
