use super::*;
use crate::bench::{
    BuildIdentity, ComparisonPolicy, EnvironmentRequirements, MetricSpec, MetricUnit, SamplingPlan,
    WorkloadIdentity,
    env::{DeclaredHost, EnvironmentField, HostProbeSource, probe_environment},
};
use std::{collections::BTreeMap, fs};

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
            name: "report".into(),
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
            warmup_samples: 1,
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
    let samples = |v: &[f64]| {
        v.iter()
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
fn codec_identity_and_aggregate_tampering_are_checked() {
    let report = fixture();
    let bytes = ReportCodec::encode(&report).unwrap();
    let decoded = ReportCodec::decode(&bytes).unwrap();
    assert_eq!(decoded, report);
    assert_eq!(ReportCodec::encode(&decoded).unwrap(), bytes);
    let mut tampered: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    tampered["comparison"]["candidate"]["mean"] = serde_json::json!(999.0);
    assert!(ReportCodec::decode(&serde_json::to_vec(&tampered).unwrap()).is_err());
}

#[test]
fn interrupted_atomic_write_keeps_old_or_complete_new_artifact() {
    let root = std::env::temp_dir().join(format!("sim-bench-report-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dir = FsReportDir::open(&root).unwrap();
    let path = TablePath::parse_absolute("/runs/current.json").unwrap();
    let old = fixture();
    write_report(&dir, &path, &old).unwrap();
    fs::write(root.join("runs/.current.json.interrupted.tmp"), b"{partial").unwrap();
    assert_eq!(read_report(&dir, &path, 1_000_000).unwrap(), old);
    let mut new = fixture();
    new.spec.comment = Some("new annotation".into());
    new.content_key = new.expected_key().unwrap();
    write_report(&dir, &path, &new).unwrap();
    assert_eq!(read_report(&dir, &path, 1_000_000).unwrap(), new);
    assert_eq!(dir.browse(1).unwrap(), vec![path]);
    assert!(
        dir.read_bounded(&TablePath::parse_absolute("/runs/current.json").unwrap(), 1)
            .is_err()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn every_summary_recomputes_from_retained_raw_samples() {
    let report = fixture();
    report.verify().unwrap();
    let summary = report.comparison.candidate.unwrap();
    assert_eq!(summary.sample_count, report.candidate_samples.len());
    assert_eq!(summary.mean, 10.0);
    assert_eq!(summary.median, 10.0);
}
