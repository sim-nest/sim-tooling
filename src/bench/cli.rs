//! Command-line adapter for benchmark execution, comparison, inspection, and policy checks.

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use sim_table_core::TablePath;

use super::{
    BenchSpec, DecisionOutcome, MetricDirection,
    compare::{ComparisonSample, RobustComparisonPolicy},
    env::{CompatibilityPolicy, EnvironmentProbe},
    exec::{ProcessDeclaration, execute},
    report::{AttemptRecord, BenchReport, CounterSample, FsReportDir, ReportCodec, write_report},
    run::{self as sampler, Arm, MonotonicClock, RunConfig, RunPhase, Workload},
};

const MAX_REPORT_BYTES: usize = 64 * 1024 * 1024;

/// Serializable, shell-free process declaration accepted by `bench run`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommandSpec {
    /// Executable name or path.
    pub program: String,
    /// Exact argument vector.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Working directory.
    pub working_directory: String,
    /// Explicit environment additions.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Whether the ambient environment is retained.
    #[serde(default)]
    pub inherit_environment: bool,
    /// Per-invocation timeout in milliseconds.
    pub timeout_ms: u64,
}

/// Complete data contract for `bench run`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunRequest {
    /// Validated benchmark declaration.
    pub spec: BenchSpec,
    /// Baseline environment evidence.
    pub baseline_environment: EnvironmentProbe,
    /// Candidate environment evidence.
    pub candidate_environment: EnvironmentProbe,
    /// Baseline command.
    pub baseline: CommandSpec,
    /// Candidate command.
    pub candidate: CommandSpec,
    /// Calibration and execution limits.
    pub run_config: RunConfig,
    /// Direction used to judge elapsed duration.
    pub direction: MetricDirection,
    /// Statistical comparison policy.
    pub comparison_policy: RobustComparisonPolicy,
    /// Required environment equivalence.
    pub environment_policy: CompatibilityPolicy,
}

/// Stable projection used by both JSON and human output.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReportView {
    /// Artifact content identity.
    pub content_key: String,
    /// Workload name.
    pub workload: String,
    /// Final policy decision.
    pub outcome: DecisionOutcome,
    /// Direction-adjusted relative regression, when decidable.
    pub relative_regression: Option<f64>,
    /// Retained baseline sample count.
    pub baseline_samples: usize,
    /// Retained candidate sample count.
    pub candidate_samples: usize,
    /// Number of excluded samples.
    pub excluded_samples: usize,
    /// Reasons a decision is inconclusive.
    pub inconclusive_reasons: Vec<String>,
}

impl ReportView {
    /// Projects one verified report without rereading or rerunning anything.
    pub fn from_report(report: &BenchReport) -> Result<Self, String> {
        report.verify()?;
        Ok(Self {
            content_key: report.content_key.0.clone(),
            workload: report.spec.workload.name.clone(),
            outcome: report.comparison.outcome.clone(),
            relative_regression: report.comparison.relative_regression,
            baseline_samples: report.baseline_samples.len(),
            candidate_samples: report.candidate_samples.len(),
            excluded_samples: report.comparison.excluded_samples.len(),
            inconclusive_reasons: report.comparison.inconclusive_reasons.clone(),
        })
    }

    /// Renders the concise human face from the same fields as serialization.
    pub fn human(&self) -> String {
        let outcome = match self.outcome {
            DecisionOutcome::Pass => "pass",
            DecisionOutcome::Fail => "fail",
            DecisionOutcome::Inconclusive => "inconclusive",
        };
        let change = self.relative_regression.map_or_else(
            || "n/a".to_owned(),
            |value| format!("{:+.2}%", value * 100.0),
        );
        format!(
            "{}: {} (regression {}, samples {}/{}, excluded {})",
            self.workload,
            outcome,
            change,
            self.baseline_samples,
            self.candidate_samples,
            self.excluded_samples
        )
    }
}

/// Dispatches the `bench` command family.
pub fn run(args: Vec<String>) -> Result<(), String> {
    let [program, _, command, tail @ ..] = args.as_slice() else {
        return Err(usage(args.first().map_or("xtask", String::as_str)));
    };
    match command.as_str() {
        "run" => run_command(tail),
        "compare" | "show" => view_command(tail),
        "check" => check_command(tail),
        _ => Err(usage(program)),
    }
}

fn run_command(args: &[String]) -> Result<(), String> {
    let request_path = option(args, "--request")?;
    let output_path = option(args, "--out")?;
    let request: RunRequest = serde_json::from_slice(
        &fs::read(request_path).map_err(|e| format!("read run request: {e}"))?,
    )
    .map_err(|e| format!("decode run request: {e}"))?;
    let clock = SystemClock(Instant::now());
    let mut workload = ProcessWorkload::new(request.baseline, request.candidate)?;
    let record = sampler::run(&request.spec, &request.run_config, &clock, &mut workload)?;
    let mut baseline = Vec::new();
    let mut candidate = Vec::new();
    let mut counters = Vec::new();
    let attempts = record
        .attempts
        .iter()
        .map(AttemptRecord::from)
        .collect::<Vec<_>>();
    for sample in record.samples {
        if sample.phase != RunPhase::Measured {
            continue;
        }
        if !matches!(sample.status, sampler::SampleStatus::Completed) {
            continue;
        }
        let value = sample.duration_ns as f64;
        let row = ComparisonSample {
            sample_index: sample.schedule_index,
            value,
        };
        match sample.arm {
            Arm::Baseline => baseline.push(row),
            Arm::Candidate => candidate.push(row),
        }
        counters.push(CounterSample {
            sample_index: sample.schedule_index,
            arm: sample.arm,
            counters: sample.counters,
        });
    }
    let all_failed = baseline.is_empty() && candidate.is_empty();
    let report = BenchReport::new_attributed_with_attempts(
        request.spec,
        request.baseline_environment,
        request.candidate_environment,
        baseline,
        candidate,
        counters,
        attempts,
        request.direction,
        request.comparison_policy,
        request.environment_policy,
    )?;
    write_artifact(Path::new(output_path), &report)?;
    if all_failed {
        return Err(
            "benchmark produced no valid measured samples; attempt ledger was written".to_owned(),
        );
    }
    println!("{}", ReportView::from_report(&report)?.human());
    Ok(())
}

fn view_command(args: &[String]) -> Result<(), String> {
    let artifact = positional(args)?;
    let report = read_artifact(Path::new(artifact))?;
    let view = ReportView::from_report(&report)?;
    if args.iter().any(|arg| arg == "--json") {
        println!(
            "{}",
            serde_json::to_string(&view).map_err(|e| e.to_string())?
        );
    } else {
        println!("{}", view.human());
    }
    Ok(())
}

fn check_command(args: &[String]) -> Result<(), String> {
    let artifact = positional(args)?;
    let report = read_artifact(Path::new(artifact))?;
    let view = ReportView::from_report(&report)?;
    match view.outcome {
        DecisionOutcome::Pass => {
            println!("{}", view.human());
            Ok(())
        }
        DecisionOutcome::Fail => Err(format!("benchmark policy failed: {}", view.human())),
        DecisionOutcome::Inconclusive => {
            Err(format!("benchmark policy inconclusive: {}", view.human()))
        }
    }
}

fn read_artifact(path: &Path) -> Result<BenchReport, String> {
    let metadata = fs::metadata(path).map_err(|e| format!("inspect report: {e}"))?;
    if metadata.len() > MAX_REPORT_BYTES as u64 {
        return Err(format!("report exceeds {MAX_REPORT_BYTES} byte limit"));
    }
    ReportCodec::decode(&fs::read(path).map_err(|e| format!("read report: {e}"))?)
}

fn write_artifact(path: &Path, report: &BenchReport) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or("invalid output path")?;
    let dir = FsReportDir::open(parent)?;
    let table_path =
        TablePath::from_segments([name]).map_err(|e| format!("invalid output path: {e:?}"))?;
    write_report(&dir, &table_path, report)
}

fn option<'a>(args: &'a [String], name: &str) -> Result<&'a str, String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
        .ok_or_else(|| format!("missing {name}"))
}

fn positional(args: &[String]) -> Result<&str, String> {
    args.iter()
        .find(|arg| !arg.starts_with('-'))
        .map(String::as_str)
        .ok_or_else(|| "missing report artifact path".to_owned())
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} bench <run --request <request.json> --out <report.json>|compare <report.json> [--json]|show <report.json> [--json]|check <report.json>>"
    )
}

struct SystemClock(Instant);
impl MonotonicClock for SystemClock {
    fn now_ns(&self) -> u64 {
        self.0.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
    }
}

struct ProcessWorkload {
    baseline: ProcessDeclaration,
    candidate: ProcessDeclaration,
}
impl ProcessWorkload {
    fn new(baseline: CommandSpec, candidate: CommandSpec) -> Result<Self, String> {
        Ok(Self {
            baseline: declaration(baseline)?,
            candidate: declaration(candidate)?,
        })
    }
}
impl<C: MonotonicClock> Workload<C> for ProcessWorkload {
    fn setup(&mut self, _: &C) -> Result<(), String> {
        Ok(())
    }
    fn sample(
        &mut self,
        arm: Arm,
        _: RunPhase,
        iterations: u64,
        _: &C,
    ) -> Result<BTreeMap<String, u64>, String> {
        let declaration = match arm {
            Arm::Baseline => &self.baseline,
            Arm::Candidate => &self.candidate,
        };
        let mut declaration = declaration.clone();
        declaration
            .environment
            .insert("SIM_BENCH_ITERATIONS".into(), iterations.to_string());
        let sample = execute(&declaration)?;
        if sample.timed_out() {
            return Err("process timed out".to_owned());
        }
        if !sample.status().is_some_and(|status| status.success()) {
            return Err(format!(
                "process exited unsuccessfully: {:?}",
                sample.status()
            ));
        }
        if sample.stdout_truncated() {
            return Err("workload counter output exceeded its retention limit".to_owned());
        }
        let mut receipt: BTreeMap<String, u64> = serde_json::from_slice(sample.stdout())
            .map_err(|error| format!("decode workload counters as JSON object: {error}"))?;
        let executed = receipt
            .remove("executed_iterations")
            .ok_or("workload receipt omitted executed_iterations")?;
        if executed != iterations {
            return Err(format!(
                "workload executed {executed} iterations; requested {iterations}"
            ));
        }
        Ok(receipt)
    }
}

fn declaration(value: CommandSpec) -> Result<ProcessDeclaration, String> {
    if value.timeout_ms == 0 {
        return Err("timeout_ms must be greater than zero".to_owned());
    }
    Ok(ProcessDeclaration {
        program: value.program,
        arguments: value.arguments,
        working_directory: value.working_directory.into(),
        environment: value.environment,
        inherit_environment: value.inherit_environment,
        stdout_limit: 64 * 1024,
        stderr_limit: 64 * 1024,
        timeout: Duration::from_millis(value.timeout_ms),
        affinity: None,
    })
}

#[cfg(test)]
#[path = "cli/tests.rs"]
mod tests;
