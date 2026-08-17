//! Frozen BYTECODE_SPEED_4 acceptance policy.

use serde::Deserialize;

/// Canonical acceptance data consumed by the final proof phase.
pub const ACCEPTANCE_TOML: &str = include_str!("../../benchmarks/bytecode-speed-4/acceptance.toml");

/// Frozen acceptance policy.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AcceptanceSpec {
    /// Schema identity.
    pub schema: String,
    /// Roadmap scope whose optimization is judged.
    pub scope: String,
    /// Characterization phase supporting the numeric policy.
    pub characterization: String,
    /// Explicit scope decision freezing the policy.
    pub scope_decision: String,
    /// Whether an inconclusive BENCH_2 decision counts as a pass.
    pub inconclusive_is_pass: bool,
    /// Required aggregate direction-adjusted improvement.
    pub aggregate_improvement_floor: f64,
    /// Auditable derivation of the aggregate floor.
    pub aggregate_derivation: String,
    /// Matching-host requirements.
    pub host: AcceptanceHost,
    /// Sampling and dispersion policy.
    pub sampling: AcceptanceSampling,
    /// Per-workload comparison policy.
    pub cases: Vec<AcceptanceCase>,
}

/// Matching-host requirements expressed with BENCH_2 environment fields.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AcceptanceHost {
    /// Environment fields that must match.
    pub required_equal: Vec<String>,
    /// Registered host inventory identity.
    pub inventory_id: String,
    /// Required architecture.
    pub architecture: String,
    /// Required build target.
    pub build_target: String,
}

/// Sampling and robust-comparison policy.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AcceptanceSampling {
    /// Minimum retained observations on each side.
    pub minimum_samples: usize,
    /// Maximum admitted MAD divided by absolute median.
    pub maximum_relative_dispersion: f64,
    /// Explicit absence of outlier exclusion.
    pub outlier_mad_multiplier: String,
    /// Bootstrap interval mass.
    pub confidence_level: f64,
    /// Deterministic bootstrap seed.
    pub bootstrap_seed: u64,
    /// Bootstrap resample count.
    pub bootstrap_resamples: usize,
    /// Bootstrap work bound.
    pub bootstrap_max_work: u64,
}

/// One characterized workload's regression policy.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AcceptanceCase {
    /// BENCH_2 workload identity.
    pub workload: String,
    /// Characterization report path relative to the acceptance data.
    pub baseline_report: String,
    /// Content identity of that report.
    pub baseline_content_key: String,
    /// Retained baseline-arm dispersion.
    pub baseline_relative_dispersion: f64,
    /// Retained candidate-arm dispersion.
    pub candidate_relative_dispersion: f64,
    /// Maximum permitted direction-adjusted regression.
    pub required_threshold: f64,
    /// Auditable derivation of the threshold.
    pub derivation: String,
}

/// Decodes the committed acceptance policy.
pub fn acceptance_spec() -> Result<AcceptanceSpec, String> {
    toml::from_str(ACCEPTANCE_TOML).map_err(|error| error.to_string())
}

/// Rejects any policy weakening unless both provenance decisions change.
pub fn reject_uncharacterized_weakening(
    frozen: &AcceptanceSpec,
    candidate: &AcceptanceSpec,
) -> Result<(), String> {
    let weakened = candidate.aggregate_improvement_floor < frozen.aggregate_improvement_floor
        || candidate.sampling.minimum_samples < frozen.sampling.minimum_samples
        || candidate.sampling.maximum_relative_dispersion
            > frozen.sampling.maximum_relative_dispersion
        || candidate.inconclusive_is_pass
        || frozen.cases.iter().any(|case| {
            candidate
                .cases
                .iter()
                .find(|other| other.workload == case.workload)
                .is_none_or(|other| other.required_threshold > case.required_threshold)
        });
    if weakened
        && (candidate.characterization == frozen.characterization
            || candidate.scope_decision == frozen.scope_decision)
    {
        return Err(
            "acceptance weakening requires a new characterization and explicit scope decision"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_is_derived_from_retained_baseline_dispersion() {
        let spec = acceptance_spec().unwrap();
        assert!(!spec.inconclusive_is_pass);
        assert_eq!(spec.cases.len(), 2);
        for case in &spec.cases {
            assert_eq!(
                case.required_threshold,
                case.baseline_relative_dispersion
                    .max(case.candidate_relative_dispersion)
            );
        }
        assert_eq!(
            spec.aggregate_improvement_floor,
            spec.cases
                .iter()
                .map(|case| case.required_threshold)
                .sum::<f64>()
        );
    }

    #[test]
    fn raising_a_ceiling_needs_new_characterization_and_scope_decision() {
        let frozen = acceptance_spec().unwrap();
        let mut weakened = frozen.clone();
        weakened.cases[0].required_threshold += f64::EPSILON;
        assert!(reject_uncharacterized_weakening(&frozen, &weakened).is_err());
        weakened.characterization = "BYTECODESPEED4.NEW-CHARACTERIZATION".into();
        weakened.scope_decision = "BYTECODESPEED4.NEW-SCOPE-DECISION".into();
        assert!(reject_uncharacterized_weakening(&frozen, &weakened).is_ok());
    }
}
