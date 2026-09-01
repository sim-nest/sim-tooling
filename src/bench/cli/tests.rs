use super::*;
use crate::bench::{
    BuildIdentity, ComparisonPolicy, EnvironmentRequirements, MetricSpec, MetricUnit, SamplingPlan,
    WorkloadIdentity,
    env::{DeclaredHost, EnvironmentField, HostProbeSource, probe_environment},
};

struct Host;
impl HostProbeSource for Host {
    fn read(&self, path: &str) -> Result<String, String> {
        Ok(match path {
            "/etc/os-release" => "ID=sim-os\n",
            "/proc/cpuinfo" => "model name: fixed cpu\n",
            "/proc/meminfo" => "MemTotal: 1024 kB\n",
            _ => "performance\n",
        }
        .to_owned())
    }
    fn architecture(&self) -> Result<String, String> {
        Ok("x86_64".to_owned())
    }
    fn logical_cpus(&self) -> Result<u32, String> {
        Ok(4)
    }
}

fn fixture() -> BenchReport {
    let build = BuildIdentity {
        source_revision: "0123456789abcdef".into(),
        target: "x86_64-sim".into(),
        profile: "release".into(),
        features: vec![],
        toolchain: "rustc 1.90".into(),
    };
    let spec = BenchSpec::new(
        WorkloadIdentity {
            name: "cli-report".into(),
            revision: "1".into(),
            parameters: BTreeMap::new(),
        },
        build.clone(),
        vec![MetricSpec {
            name: "latency".into(),
            unit: MetricUnit::Nanoseconds,
            direction: MetricDirection::LowerIsBetter,
        }],
        SamplingPlan {
            warmup_samples: 0,
            measured_samples: 3,
            seed: 7,
        },
        ComparisonPolicy {
            required_threshold: Some(0.2),
            noise_floor: 0.01,
            confidence_level: 0.95,
        },
        EnvironmentRequirements {
            operating_system: None,
            architecture: None,
            minimum_logical_cpus: 1,
            minimum_memory_bytes: 1,
            capabilities: vec![],
            network_isolation: true,
        },
        None,
    )
    .unwrap();
    let env = probe_environment(
        DeclaredHost::new("bench-1".into(), "bench-1".into()).unwrap(),
        &build,
        &Host,
    );
    let samples = |values: &[f64]| {
        values
            .iter()
            .enumerate()
            .map(|(i, value)| ComparisonSample {
                sample_index: i as u32,
                value: *value,
            })
            .collect()
    };
    BenchReport::new(
        spec,
        env.clone(),
        env,
        samples(&[10.0, 11.0, 12.0]),
        samples(&[9.0, 10.0, 11.0]),
        MetricDirection::LowerIsBetter,
        RobustComparisonPolicy {
            minimum_samples: 2,
            maximum_relative_dispersion: 1.0,
            outlier_mad_multiplier: None,
            required_threshold: 0.2,
            confidence_level: 0.95,
            bootstrap_seed: 9,
            bootstrap_resamples: 100,
            bootstrap_max_work: 10_000,
        },
        CompatibilityPolicy::requiring([EnvironmentField::CpuModel]),
    )
    .unwrap()
}

#[test]
fn check_reads_stored_artifact_without_process_execution() {
    let root = std::env::temp_dir().join(format!("sim-bench-cli-check-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let artifact = root.join("report.json");
    fs::write(&artifact, ReportCodec::encode(&fixture()).unwrap()).unwrap();
    let marker = root.join("must-not-exist");
    let args = vec![artifact.to_string_lossy().into_owned()];
    check_command(&args).unwrap();
    assert!(
        !marker.exists(),
        "checking an artifact must have no execution path"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn machine_and_human_faces_project_the_same_report_view() {
    let view = ReportView::from_report(&fixture()).unwrap();
    let json = serde_json::to_value(&view).unwrap();
    let human = view.human();
    assert_eq!(json["workload"], "cli-report");
    assert_eq!(json["outcome"], "pass");
    assert!(human.starts_with("cli-report: pass"));
    assert!(human.contains(&format!(
        "samples {}/{}",
        view.baseline_samples, view.candidate_samples
    )));
}

#[test]
fn equal_arm_identities_are_refused_before_dispatch() {
    let build = BuildIdentity {
        source_revision: "same".into(),
        target: "x86_64-sim".into(),
        profile: "release".into(),
        features: vec![],
        toolchain: "rustc-test".into(),
    };
    let identity = ArmIdentity {
        executable_content_key: "sha256:same".into(),
        build,
        command_identity: "same-command".into(),
    };
    let command = CommandSpec {
        program: "/immutable/workload".into(),
        arguments: vec!["warm".into()],
        working_directory: "/immutable".into(),
        environment: BTreeMap::new(),
        inherit_environment: false,
        timeout_ms: 1,
    };
    assert_eq!(
        validate_distinct_arm_values(&identity, &identity, &command, &command).unwrap_err(),
        "baseline and candidate executable content identities are equal"
    );
}
