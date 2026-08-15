use std::{
    cell::Cell,
    collections::BTreeMap,
    fs,
    hint::black_box,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use sim_table_core::TablePath;

use super::{
    BenchSpec, BuildIdentity, ComparisonPolicy, DecisionOutcome, EnvironmentRequirements,
    MetricDirection, MetricSpec, MetricUnit, SamplingPlan, WorkloadIdentity,
    compare::{ComparisonSample, RobustComparisonPolicy},
    env::{
        CompatibilityPolicy, DeclaredHost, EnvironmentField, EnvironmentProbe, LocalHostProbe,
        ProbeEvidence, probe_environment,
    },
    report::{AtomicReportDir, BenchReport, FsReportDir, read_report, write_report},
    run::{self, Arm, MonotonicClock, RunConfig, RunPhase, SampleStatus, Workload},
};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
pub(super) struct FakeClock(Cell<u64>);

impl FakeClock {
    fn advance(&self, nanoseconds: u64) {
        self.0.set(self.0.get().checked_add(nanoseconds).unwrap());
    }
}

impl MonotonicClock for FakeClock {
    fn now_ns(&self) -> u64 {
        self.0.get()
    }
}

pub(super) trait NeutralOperation {
    fn perform(&mut self, iterations: u64) -> u64;
}

pub(super) struct CpuOperation;

impl NeutralOperation for CpuOperation {
    fn perform(&mut self, iterations: u64) -> u64 {
        let mut state = 0x6a09_e667_f3bc_c909_u64;
        for index in 0..iterations {
            state = black_box(state.rotate_left(7) ^ index).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        }
        black_box(state)
    }
}

pub(super) struct AllocationOperation;

impl NeutralOperation for AllocationOperation {
    fn perform(&mut self, iterations: u64) -> u64 {
        let length = usize::try_from(iterations).unwrap();
        let values = (0..length).map(|value| value as u64).collect::<Vec<_>>();
        black_box(
            values
                .iter()
                .fold(0_u64, |sum, value| sum.wrapping_add(*value)),
        )
    }
}

pub(super) struct NeutralWorkload<O> {
    operation: O,
    measured_calls: u32,
    fail_on_measured_call: Option<u32>,
}

impl<O> NeutralWorkload<O> {
    pub(super) fn new(operation: O, fail_on_measured_call: Option<u32>) -> Self {
        Self {
            operation,
            measured_calls: 0,
            fail_on_measured_call,
        }
    }
}

impl<O: NeutralOperation> Workload<FakeClock> for NeutralWorkload<O> {
    fn setup(&mut self, _clock: &FakeClock) -> Result<(), String> {
        Ok(())
    }

    fn sample(
        &mut self,
        arm: Arm,
        phase: RunPhase,
        iterations: u64,
        clock: &FakeClock,
    ) -> Result<BTreeMap<String, u64>, String> {
        let checksum = self.operation.perform(iterations);
        let per_iteration = match arm {
            Arm::Baseline => 10,
            Arm::Candidate => 9,
        };
        clock.advance(iterations.checked_mul(per_iteration).unwrap());
        if phase == RunPhase::Measured {
            self.measured_calls += 1;
            if self.fail_on_measured_call == Some(self.measured_calls) {
                return Err("retained neutral specimen failure".to_owned());
            }
        }
        Ok(BTreeMap::from([("checksum".to_owned(), checksum)]))
    }
}

pub(super) fn spec(name: &str) -> BenchSpec {
    BenchSpec::new(
        WorkloadIdentity {
            name: name.to_owned(),
            revision: "1".to_owned(),
            parameters: BTreeMap::new(),
        },
        build(),
        vec![MetricSpec {
            name: "elapsed".to_owned(),
            unit: MetricUnit::Nanoseconds,
            direction: MetricDirection::LowerIsBetter,
        }],
        SamplingPlan {
            warmup_samples: 2,
            measured_samples: 4,
            seed: 19,
        },
        ComparisonPolicy {
            required_threshold: Some(0.05),
            noise_floor: 0.01,
            confidence_level: 0.95,
        },
        EnvironmentRequirements {
            operating_system: None,
            architecture: None,
            minimum_logical_cpus: 1,
            minimum_memory_bytes: 0,
            capabilities: vec![],
            network_isolation: true,
        },
        Some("neutral workload specimen".to_owned()),
    )
    .unwrap()
}

fn build() -> BuildIdentity {
    BuildIdentity {
        source_revision: "neutral-specimen".to_owned(),
        target: std::env::consts::ARCH.to_owned(),
        profile: "test".to_owned(),
        features: vec![],
        toolchain: "rust-test".to_owned(),
    }
}

pub(super) fn local_environment() -> EnvironmentProbe {
    probe_environment(
        DeclaredHost::new("local-ci-machine".to_owned(), "localhost".to_owned()).unwrap(),
        &build(),
        &LocalHostProbe,
    )
}

pub(super) fn assert_local_fingerprint(environment: &EnvironmentProbe) {
    assert_eq!(environment.host.host.inventory_id, "local-ci-machine");
    assert_eq!(environment.host.host.ssh_host, "localhost");
    assert!(matches!(
        environment.host.architecture,
        ProbeEvidence::Available { ref value } if value == std::env::consts::ARCH
    ));
    assert!(matches!(
        environment.host.logical_cpus,
        ProbeEvidence::Available { value } if value > 0
    ));
}

pub(super) fn run_config() -> RunConfig {
    RunConfig {
        calibration_target_ns: 100,
        max_iterations: 100,
        sample_timeout_ns: 1_000,
    }
}

pub(super) fn matched_report(name: &str) -> BenchReport {
    report(
        name,
        vec![100.0, 101.0, 99.0, 100.0],
        vec![99.0, 100.0, 98.0, 99.0],
    )
}

pub(super) fn dispersed_report(name: &str) -> BenchReport {
    report(
        name,
        vec![10.0, 100.0, 20.0, 200.0],
        vec![10.0, 100.0, 20.0, 200.0],
    )
}

fn report(name: &str, baseline: Vec<f64>, candidate: Vec<f64>) -> BenchReport {
    let environment = local_environment();
    BenchReport::new(
        spec(name),
        environment.clone(),
        environment,
        samples(baseline),
        samples(candidate),
        MetricDirection::LowerIsBetter,
        RobustComparisonPolicy {
            minimum_samples: 4,
            maximum_relative_dispersion: 0.25,
            outlier_mad_multiplier: None,
            required_threshold: 0.05,
            confidence_level: 0.95,
            bootstrap_seed: 19,
            bootstrap_resamples: 128,
            bootstrap_max_work: 8_192,
        },
        CompatibilityPolicy::requiring([
            EnvironmentField::HostInventoryId,
            EnvironmentField::Architecture,
            EnvironmentField::BuildTarget,
        ]),
    )
    .unwrap()
}

fn samples(values: Vec<f64>) -> Vec<ComparisonSample> {
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| ComparisonSample {
            sample_index: index as u32,
            value,
        })
        .collect()
}

pub(super) fn assert_report_round_trip(report: &BenchReport) {
    let temp = TempDir::new();
    let dir = FsReportDir::open(temp.path()).unwrap();
    let path = TablePath::from_segments(["neutral", "report.json"]).unwrap();
    write_report(&dir, &path, report).unwrap();
    assert_eq!(read_report(&dir, &path, 1_000_000).unwrap(), *report);
    assert_eq!(dir.browse(4).unwrap(), vec![path]);
}

pub(super) fn assert_synthetic_host_refused(report: &BenchReport) {
    let mut synthetic = report.candidate_environment.clone();
    synthetic.host.host.inventory_id = "synthetic-other-host".to_owned();
    let refused = BenchReport::new(
        report.spec.clone(),
        report.baseline_environment.clone(),
        synthetic,
        report.baseline_samples.clone(),
        report.candidate_samples.clone(),
        report.direction.clone(),
        report.comparison_policy,
        report.environment_policy.clone(),
    )
    .unwrap();
    assert_eq!(refused.comparison.outcome, DecisionOutcome::Inconclusive);
    assert_eq!(refused.comparison.environment_mismatches.len(), 1);
    assert!(
        refused
            .comparison
            .inconclusive_reasons
            .iter()
            .any(|reason| { reason.starts_with("environment-compatibility:") })
    );
}

pub(super) fn assert_retained_failure<O: NeutralOperation>(operation: O, name: &str) {
    let clock = FakeClock::default();
    let mut workload = NeutralWorkload::new(operation, Some(2));
    let record = run::run(&spec(name), &run_config(), &clock, &mut workload).unwrap();
    assert_eq!(
        record.phases,
        vec![
            RunPhase::Setup,
            RunPhase::Calibration,
            RunPhase::Warmup,
            RunPhase::Measured
        ]
    );
    assert_eq!(record.calibration.probe_duration_ns, 9);
    assert_eq!(record.calibration.selected_iterations, 12);
    assert!(record.samples.iter().any(|sample| {
        sample.status == SampleStatus::Failed("retained neutral specimen failure".to_owned())
            && sample.duration_ns > 0
    }));
    assert_eq!(record.observations.len(), record.samples.len() - 1);
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sim-tooling-neutral-specimen-{}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
