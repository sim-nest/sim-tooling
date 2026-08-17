//! Representative JVM corpus declarations for BYTECODE_SPEED_4.
//!
//! Execution remains a normal process workload. These declarations contain no
//! clock or statistics and can therefore be replayed by any BENCH_2 runner on a
//! compatible registered host.

use super::{
    BenchSpec, BuildIdentity, ComparisonPolicy, EnvironmentRequirements, MetricDirection,
    MetricSpec, MetricUnit, SamplingPlan, WorkloadIdentity,
};
use std::collections::BTreeMap;

/// Complete counter vocabulary required for an attributable JVM baseline.
pub const JVM_BASELINE_COUNTERS: [&str; 8] = [
    "preparation",
    "dispatch",
    "resolution",
    "allocation",
    "root-scanning",
    "safepoint-polling",
    "work-accounting",
    "verifier-checks",
];

/// Produces the cold-preparation and warm-execution benchmark declarations.
pub fn corpus(build: BuildIdentity) -> Result<[BenchSpec; 2], String> {
    Ok([
        spec(
            "jvm-bytecode-cold-preparation",
            "cold-preparation",
            build.clone(),
        )?,
        spec("jvm-bytecode-warm-execution", "warm-execution", build)?,
    ])
}

fn spec(name: &str, phase: &str, build: BuildIdentity) -> Result<BenchSpec, String> {
    BenchSpec::new(
        WorkloadIdentity {
            name: name.into(),
            revision: "bytecode-speed-4/v1".into(),
            parameters: BTreeMap::from([
                ("class".into(), "Example".into()),
                ("method".into(), "zero()I".into()),
                ("phase".into(), phase.into()),
            ]),
        },
        build,
        JVM_BASELINE_COUNTERS.map(|name| MetricSpec {
            name: name.into(),
            unit: MetricUnit::Count,
            direction: MetricDirection::LowerIsBetter,
        }).into(),
        SamplingPlan { warmup_samples: 4, measured_samples: 20, seed: 0x4259_5445_434f_4445 },
        ComparisonPolicy { required_threshold: None, noise_floor: 0.01, confidence_level: 0.95 },
        EnvironmentRequirements {
            operating_system: Some("linux".into()),
            architecture: Some("x86_64".into()),
            minimum_logical_cpus: 12,
            minimum_memory_bytes: 32 * 1024 * 1024 * 1024,
            capabilities: vec!["cpu:x86_64".into()],
            network_isolation: true,
        },
        Some("Representative two-instruction JVM corpus; all attribution comes from retained adapter counters.".into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_separates_cold_and_warm_and_declares_every_counter() {
        let specs = corpus(BuildIdentity {
            source_revision: "0123456789abcdef".into(),
            target: "x86_64-unknown-linux-gnu".into(),
            profile: "release".into(),
            features: vec![],
            toolchain: "rustc-test".into(),
        })
        .unwrap();
        assert_ne!(specs[0].content_key, specs[1].content_key);
        for spec in specs {
            assert_eq!(spec.metrics.len(), JVM_BASELINE_COUNTERS.len());
            assert!(
                JVM_BASELINE_COUNTERS
                    .iter()
                    .all(|name| spec.metrics.iter().any(|metric| metric.name == *name))
            );
        }
    }
}
