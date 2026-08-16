//! Deterministic benchmark calibration and sampling state machine.
//!
//! Time is supplied by [`MonotonicClock`]. The runner never sleeps, which
//! keeps both execution policy and tests independent of wall-clock behavior.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{BenchSpec, MetricUnit, RawObservation};

/// One side of a benchmark comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Arm {
    /// Previously accepted implementation.
    Baseline,
    /// Implementation under evaluation.
    Candidate,
}

/// Distinct lifecycle states entered by the runner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunPhase {
    /// One-time workload preparation.
    Setup,
    /// Iteration-count selection; never contributes observations.
    Calibration,
    /// Unrecorded steady-state execution.
    Warmup,
    /// Recorded comparison samples.
    Measured,
}

/// Monotonic time source used at sample boundaries.
pub trait MonotonicClock {
    /// Returns nanoseconds from an arbitrary, stable epoch.
    fn now_ns(&self) -> u64;
}

/// Work performed by the state machine.
pub trait Workload<C: MonotonicClock> {
    /// Performs one-time setup.
    fn setup(&mut self, clock: &C) -> Result<(), String>;

    /// Runs `iterations` and returns additional named counters.
    fn sample(
        &mut self,
        arm: Arm,
        phase: RunPhase,
        iterations: u64,
        clock: &C,
    ) -> Result<BTreeMap<String, u64>, String>;
}

/// Validated execution limits that are intentionally separate from statistics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunConfig {
    /// Desired duration used to choose the measured iteration count.
    pub calibration_target_ns: u64,
    /// Maximum permitted iterations in any invocation.
    pub max_iterations: u64,
    /// Duration after which a completed sample is classified as timed out.
    pub sample_timeout_ns: u64,
}

impl RunConfig {
    /// Refuses zero limits, which would make calibration ambiguous or unsafe.
    pub fn validate(&self) -> Result<(), String> {
        let mut errors = Vec::new();
        if self.calibration_target_ns == 0 {
            errors.push("calibration_target_ns must be greater than zero");
        }
        if self.max_iterations == 0 {
            errors.push("max_iterations must be greater than zero");
        }
        if self.sample_timeout_ns == 0 {
            errors.push("sample_timeout_ns must be greater than zero");
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

/// Recorded calibration input and selected iteration count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalibrationDecision {
    /// Iterations used by the calibration probe.
    pub probe_iterations: u64,
    /// Elapsed probe duration.
    pub probe_duration_ns: u64,
    /// Desired sample duration.
    pub target_duration_ns: u64,
    /// Iterations selected for warmup and measurement.
    pub selected_iterations: u64,
}

/// Status retained for every attempted measured sample.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SampleStatus {
    /// Workload completed within its timeout.
    Completed,
    /// Workload completed, but elapsed time exceeded the declared timeout.
    TimedOut,
    /// Workload returned an error.
    Failed(String),
}

/// Complete raw record for a measured invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleRecord {
    /// Position in the realized interleaving.
    pub schedule_index: u32,
    /// Comparison arm invoked at this position.
    pub arm: Arm,
    /// Iterations requested.
    pub iterations: u64,
    /// Raw monotonic duration, including failed invocations.
    pub duration_ns: u64,
    /// Workload counters, retained without aggregation.
    pub counters: BTreeMap<String, u64>,
    /// Completion classification.
    pub status: SampleStatus,
}

/// Auditable output of a benchmark run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// States in the order entered.
    pub phases: Vec<RunPhase>,
    /// Calibration evidence and choice.
    pub calibration: CalibrationDecision,
    /// Exact measured arm order produced from the specification seed.
    pub realized_schedule: Vec<Arm>,
    /// Every measured attempt, including failures and timeouts.
    pub samples: Vec<SampleRecord>,
    /// Successful duration observations suitable for summary input.
    pub observations: Vec<RawObservation>,
}

/// Executes the complete benchmark lifecycle.
pub fn run<C: MonotonicClock, W: Workload<C>>(
    spec: &BenchSpec,
    config: &RunConfig,
    clock: &C,
    workload: &mut W,
) -> Result<RunRecord, String> {
    config.validate()?;
    let mut phases = vec![RunPhase::Setup];
    workload
        .setup(clock)
        .map_err(|error| format!("setup failed: {error}"))?;

    phases.push(RunPhase::Calibration);
    let start = clock.now_ns();
    workload
        .sample(Arm::Candidate, RunPhase::Calibration, 1, clock)
        .map_err(|error| format!("calibration failed: {error}"))?;
    let probe_duration_ns = elapsed(start, clock.now_ns())?;
    let selected_iterations = calibrated_iterations(probe_duration_ns, config)?;
    let calibration = CalibrationDecision {
        probe_iterations: 1,
        probe_duration_ns,
        target_duration_ns: config.calibration_target_ns,
        selected_iterations,
    };

    phases.push(RunPhase::Warmup);
    for index in 0..spec.sampling_plan.warmup_samples {
        let arm = if index % 2 == 0 {
            Arm::Baseline
        } else {
            Arm::Candidate
        };
        workload
            .sample(arm, RunPhase::Warmup, selected_iterations, clock)
            .map_err(|error| format!("warmup sample {index} failed: {error}"))?;
    }

    phases.push(RunPhase::Measured);
    let realized_schedule = schedule(spec.sampling_plan.measured_samples, spec.sampling_plan.seed)?;
    let mut samples = Vec::with_capacity(realized_schedule.len());
    let mut observations = Vec::with_capacity(realized_schedule.len());
    for (index, &arm) in realized_schedule.iter().enumerate() {
        let start = clock.now_ns();
        let result = workload.sample(arm, RunPhase::Measured, selected_iterations, clock);
        let duration_ns = elapsed(start, clock.now_ns())?;
        let (counters, status) = match result {
            Ok(counters) if duration_ns > config.sample_timeout_ns => {
                (counters, SampleStatus::TimedOut)
            }
            Ok(counters) => (counters, SampleStatus::Completed),
            Err(error) => (BTreeMap::new(), SampleStatus::Failed(error)),
        };
        if status == SampleStatus::Completed {
            observations.push(RawObservation::new(
                spec.content_key.clone(),
                u32::try_from(index).map_err(|_| "measured schedule exceeds u32")?,
                format!("{}-duration", arm_name(arm)),
                MetricUnit::Nanoseconds,
                duration_ns as f64,
            )?);
        }
        samples.push(SampleRecord {
            schedule_index: u32::try_from(index).map_err(|_| "measured schedule exceeds u32")?,
            arm,
            iterations: selected_iterations,
            duration_ns,
            counters,
            status,
        });
    }
    Ok(RunRecord {
        phases,
        calibration,
        realized_schedule,
        samples,
        observations,
    })
}

fn elapsed(start: u64, end: u64) -> Result<u64, String> {
    end.checked_sub(start)
        .ok_or_else(|| "monotonic clock moved backwards".to_owned())
}

fn calibrated_iterations(probe_ns: u64, config: &RunConfig) -> Result<u64, String> {
    if probe_ns == 0 {
        return Err("calibration probe duration was zero".to_owned());
    }
    let numerator = config
        .calibration_target_ns
        .checked_add(probe_ns - 1)
        .ok_or_else(|| "calibration iteration arithmetic overflowed".to_owned())?;
    Ok((numerator / probe_ns).clamp(1, config.max_iterations))
}

fn schedule(samples_per_arm: u32, seed: u64) -> Result<Vec<Arm>, String> {
    let capacity = usize::try_from(samples_per_arm)
        .ok()
        .and_then(|count| count.checked_mul(2))
        .ok_or_else(|| "measured schedule size overflowed".to_owned())?;
    let mut arms = Vec::with_capacity(capacity);
    for _ in 0..samples_per_arm {
        arms.extend([Arm::Baseline, Arm::Candidate]);
    }
    let mut state = seed;
    for upper in (1..arms.len()).rev() {
        let random = splitmix64(&mut state);
        let index =
            (random % u64::try_from(upper + 1).map_err(|_| "schedule index overflowed")?) as usize;
        arms.swap(upper, index);
    }
    Ok(arms)
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn arm_name(arm: Arm) -> &'static str {
    match arm {
        Arm::Baseline => "baseline",
        Arm::Candidate => "candidate",
    }
}

#[cfg(test)]
mod tests {
    // conformance: deterministic sampling retains lifecycle, calibration, scheduling,
    // timeout, failure, and counter evidence under an injected monotonic clock.

    use std::{cell::Cell, collections::BTreeMap};

    use super::*;
    use crate::bench::{
        BuildIdentity, ComparisonPolicy, EnvironmentRequirements, MetricDirection, MetricSpec,
        SamplingPlan, WorkloadIdentity,
    };

    #[derive(Default)]
    struct FakeClock(Cell<u64>);
    impl FakeClock {
        fn advance(&self, ns: u64) {
            self.0.set(self.0.get() + ns);
        }
    }
    impl MonotonicClock for FakeClock {
        fn now_ns(&self) -> u64 {
            self.0.get()
        }
    }

    #[derive(Default)]
    struct FakeWorkload {
        calls: Vec<RunPhase>,
        measured: u32,
    }
    impl Workload<FakeClock> for FakeWorkload {
        fn setup(&mut self, _clock: &FakeClock) -> Result<(), String> {
            self.calls.push(RunPhase::Setup);
            Ok(())
        }
        fn sample(
            &mut self,
            _arm: Arm,
            phase: RunPhase,
            iterations: u64,
            clock: &FakeClock,
        ) -> Result<BTreeMap<String, u64>, String> {
            self.calls.push(phase);
            let duration = if phase == RunPhase::Calibration {
                10
            } else {
                iterations * 10
            };
            clock.advance(duration);
            if phase == RunPhase::Measured {
                self.measured += 1;
            }
            if self.measured == 3 {
                return Err("fixture failure".to_owned());
            }
            Ok(BTreeMap::from([("operations".to_owned(), iterations)]))
        }
    }

    fn spec(seed: u64) -> BenchSpec {
        BenchSpec::new(
            WorkloadIdentity {
                name: "fixture".into(),
                revision: "1".into(),
                parameters: BTreeMap::new(),
            },
            BuildIdentity {
                source_revision: "abc".into(),
                target: "test".into(),
                profile: "release".into(),
                features: vec![],
                toolchain: "rustc".into(),
            },
            vec![MetricSpec {
                name: "latency".into(),
                unit: MetricUnit::Nanoseconds,
                direction: MetricDirection::LowerIsBetter,
            }],
            SamplingPlan {
                warmup_samples: 2,
                measured_samples: 4,
                seed,
            },
            ComparisonPolicy {
                required_threshold: None,
                noise_floor: 0.0,
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
            None,
        )
        .unwrap()
    }

    #[test]
    fn lifecycle_is_clock_driven_and_calibration_is_not_observed() {
        let clock = FakeClock::default();
        let mut workload = FakeWorkload::default();
        let record = run(
            &spec(17),
            &RunConfig {
                calibration_target_ns: 50,
                max_iterations: 100,
                sample_timeout_ns: 60,
            },
            &clock,
            &mut workload,
        )
        .unwrap();
        assert_eq!(
            record.phases,
            [
                RunPhase::Setup,
                RunPhase::Calibration,
                RunPhase::Warmup,
                RunPhase::Measured
            ]
        );
        assert_eq!(record.calibration.selected_iterations, 5);
        assert_eq!(record.samples.len(), 8);
        assert_eq!(record.observations.len(), 7);
        assert!(
            record
                .observations
                .iter()
                .all(|sample| sample.value == 50.0)
        );
        assert_eq!(
            workload
                .calls
                .iter()
                .filter(|phase| **phase == RunPhase::Calibration)
                .count(),
            1
        );
    }

    #[test]
    fn interleaving_is_seeded_balanced_and_reproducible() {
        let first = schedule(12, 99).unwrap();
        assert_eq!(first, schedule(12, 99).unwrap());
        assert_ne!(first, schedule(12, 100).unwrap());
        assert_eq!(
            first.iter().filter(|arm| **arm == Arm::Baseline).count(),
            12
        );
        assert_eq!(
            first.iter().filter(|arm| **arm == Arm::Candidate).count(),
            12
        );
    }

    #[test]
    fn records_timeouts_failures_and_counters() {
        let clock = FakeClock::default();
        let mut workload = FakeWorkload::default();
        let record = run(
            &spec(1),
            &RunConfig {
                calibration_target_ns: 100,
                max_iterations: 100,
                sample_timeout_ns: 50,
            },
            &clock,
            &mut workload,
        )
        .unwrap();
        assert!(
            record
                .samples
                .iter()
                .any(|sample| sample.status == SampleStatus::TimedOut)
        );
        assert!(
            record
                .samples
                .iter()
                .any(|sample| matches!(sample.status, SampleStatus::Failed(_)))
        );
        assert!(
            record
                .samples
                .iter()
                .any(|sample| sample.counters.get("operations") == Some(&10))
        );
    }

    #[test]
    fn checked_calibration_rejects_zero_time_and_overflow() {
        let config = RunConfig {
            calibration_target_ns: u64::MAX,
            max_iterations: u64::MAX,
            sample_timeout_ns: 1,
        };
        assert!(
            calibrated_iterations(0, &config)
                .unwrap_err()
                .contains("zero")
        );
        assert!(
            calibrated_iterations(2, &config)
                .unwrap_err()
                .contains("overflow")
        );
    }
}
