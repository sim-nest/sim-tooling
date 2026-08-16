use std::{collections::BTreeMap, fs};
use xtask::bench::{
    BuildIdentity, MetricDirection,
    cli::{CommandSpec, RunRequest},
    compare::RobustComparisonPolicy,
    env::{CompatibilityPolicy, DeclaredHost, EnvironmentField, LocalHostProbe, probe_environment},
    jvm_baseline::corpus,
    run::RunConfig,
};

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    assert_eq!(
        args.len(),
        8,
        "usage: jvm_baseline_request <phase> <host-id> <binary> <working-directory> <source-revision> <toolchain> <output>"
    );
    let build = BuildIdentity {
        source_revision: args[5].clone(),
        target: "x86_64-unknown-linux-gnu".into(),
        profile: "release".into(),
        features: vec![],
        toolchain: args[6].clone(),
    };
    let spec = corpus(build.clone())
        .unwrap()
        .into_iter()
        .find(|spec| spec.workload.parameters["phase"] == args[1])
        .unwrap();
    let environment = probe_environment(
        DeclaredHost::new(args[2].clone(), args[2].clone()).unwrap(),
        &build,
        &LocalHostProbe,
    );
    let command = CommandSpec {
        program: args[3].clone(),
        arguments: vec![args[1].clone()],
        working_directory: args[4].clone(),
        environment: BTreeMap::new(),
        inherit_environment: true,
        timeout_ms: 10_000,
    };
    let request = RunRequest {
        spec,
        baseline_environment: environment.clone(),
        candidate_environment: environment,
        baseline: command.clone(),
        candidate: command,
        run_config: RunConfig {
            calibration_target_ns: 1_000_000,
            max_iterations: 1,
            sample_timeout_ns: 10_000_000_000,
        },
        direction: MetricDirection::LowerIsBetter,
        comparison_policy: RobustComparisonPolicy {
            minimum_samples: 10,
            maximum_relative_dispersion: 2.0,
            outlier_mad_multiplier: None,
            required_threshold: 1.0,
            confidence_level: 0.95,
            bootstrap_seed: 19,
            bootstrap_resamples: 128,
            bootstrap_max_work: 20_000,
        },
        environment_policy: CompatibilityPolicy::requiring([
            EnvironmentField::HostInventoryId,
            EnvironmentField::Architecture,
            EnvironmentField::BuildTarget,
        ]),
    };
    fs::write(&args[7], serde_json::to_vec(&request).unwrap()).unwrap();
}
