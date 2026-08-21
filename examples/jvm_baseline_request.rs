use std::{collections::BTreeMap, fs};
use xtask::bench::{
    BuildIdentity, MetricDirection,
    cli::{ArmIdentity, CommandSpec, RunRequest},
    compare::RobustComparisonPolicy,
    env::{CompatibilityPolicy, DeclaredHost, EnvironmentField, LocalHostProbe, probe_environment},
    jvm_baseline::corpus,
    run::RunConfig,
};

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    assert_eq!(
        args.len(),
        11,
        "usage: jvm_baseline_request <phase> <host-id> <baseline-binary> <baseline-revision> <candidate-binary> <candidate-revision> <working-directory> <toolchain> <output>"
    );
    let build = |revision: &str| BuildIdentity {
        source_revision: revision.into(),
        target: "x86_64-unknown-linux-gnu".into(),
        profile: "release".into(),
        features: vec![],
        toolchain: args[8].clone(),
    };
    let baseline_build = build(&args[4]);
    let candidate_build = build(&args[6]);
    let spec = corpus(candidate_build.clone())
        .unwrap()
        .into_iter()
        .find(|spec| spec.workload.parameters["phase"] == args[1])
        .unwrap();
    let environment = probe_environment(
        DeclaredHost::new(args[2].clone(), args[2].clone()).unwrap(),
        &candidate_build,
        &LocalHostProbe,
    );
    let command = |program: &str| CommandSpec {
        program: program.into(),
        arguments: vec![args[1].clone()],
        working_directory: args[7].clone(),
        environment: BTreeMap::new(),
        inherit_environment: true,
        timeout_ms: 10_000,
    };
    let executable_key = |path: &str| {
        let bytes = fs::read(path).unwrap();
        format!("sha256:{}", xtask::bench::content_identity(&bytes))
    };
    let request = RunRequest {
        spec,
        baseline_environment: probe_environment(
            DeclaredHost::new(args[2].clone(), args[2].clone()).unwrap(),
            &baseline_build,
            &LocalHostProbe,
        ),
        candidate_environment: environment,
        baseline_identity: ArmIdentity {
            executable_content_key: executable_key(&args[3]),
            build: baseline_build,
            command_identity: format!("{}:{}", args[3], args[1]),
        },
        candidate_identity: ArmIdentity {
            executable_content_key: executable_key(&args[5]),
            build: candidate_build,
            command_identity: format!("{}:{}", args[5], args[1]),
        },
        baseline: command(&args[3]),
        candidate: command(&args[5]),
        run_config: RunConfig {
            calibration_target_ns: 250_000_000,
            max_iterations: 10_000_000,
            sample_timeout_ns: 10_000_000_000,
        },
        direction: MetricDirection::LowerIsBetter,
        comparison_policy: RobustComparisonPolicy {
            minimum_samples: 20,
            maximum_relative_dispersion: 0.022779728136124593,
            outlier_mad_multiplier: None,
            required_threshold: if args[1] == "cold-preparation" {
                0.022779728136124593
            } else {
                0.017843516512883528
            },
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
    fs::write(&args[10], serde_json::to_vec(&request).unwrap()).unwrap();
}
