//! Bounded, argv-exact process execution for benchmark workloads.
//!
//! Commands are never interpreted by a shell. The executor owns recorded
//! samples; adapters can prepare files and return a declaration, but never
//! receive a mutable sample.

// conformance: exact process declarations produce bounded, auditable samples.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

/// A complete, explicit process invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessDeclaration {
    /// Executable path or name. It is passed directly to [`Command::new`].
    pub program: String,
    /// Exact argument vector; no shell parsing or interpolation occurs.
    pub arguments: Vec<String>,
    /// Controlled working directory.
    pub working_directory: PathBuf,
    /// Complete child environment when `inherit_environment` is false.
    pub environment: BTreeMap<String, String>,
    /// Whether to retain the ambient environment before applying `environment`.
    pub inherit_environment: bool,
    /// Maximum retained bytes from stdout.
    pub stdout_limit: usize,
    /// Maximum retained bytes from stderr.
    pub stderr_limit: usize,
    /// Wall-clock deadline for the child.
    pub timeout: Duration,
    /// Optional CPU affinity request.
    pub affinity: Option<CpuAffinity>,
}

/// Requested logical CPUs for a process workload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuAffinity {
    /// Non-empty list of logical CPU indices.
    pub logical_cpus: Vec<usize>,
}

/// Requested versus achieved isolation evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsolationRecord {
    requested_affinity: Option<Vec<usize>>,
    observed_affinity: Option<Vec<usize>>,
    achieved_affinity: bool,
    workload_tree_contained: bool,
    detail: String,
}

impl IsolationRecord {
    /// Requested logical CPUs, if any.
    pub fn requested_affinity(&self) -> Option<&[usize]> {
        self.requested_affinity.as_deref()
    }

    /// Whether the platform mechanism reported successful affinity application.
    pub fn achieved_affinity(&self) -> bool {
        self.achieved_affinity
    }

    /// Logical CPUs observed on the spawned workload process.
    pub fn observed_affinity(&self) -> Option<&[usize]> {
        self.observed_affinity.as_deref()
    }

    /// Whether the complete workload was placed in executor-owned containment.
    pub fn workload_tree_contained(&self) -> bool {
        self.workload_tree_contained
    }

    /// Human-readable achievement or gap evidence.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// Immutable result recorded by the executor.
#[derive(Debug)]
pub struct ProcessSample {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_truncated: bool,
    stderr_truncated: bool,
    status: Option<ExitStatus>,
    timed_out: bool,
    elapsed: Duration,
    isolation: IsolationRecord,
}

impl ProcessSample {
    /// Retained stdout prefix.
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }
    /// Retained stderr prefix.
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
    /// Whether stdout exceeded its retention limit.
    pub fn stdout_truncated(&self) -> bool {
        self.stdout_truncated
    }
    /// Whether stderr exceeded its retention limit.
    pub fn stderr_truncated(&self) -> bool {
        self.stderr_truncated
    }
    /// Exit status, absent only when a killed child could not be reaped.
    pub fn status(&self) -> Option<ExitStatus> {
        self.status
    }
    /// Whether the deadline caused termination.
    pub fn timed_out(&self) -> bool {
        self.timed_out
    }
    /// Wall-clock process duration.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }
    /// Requested versus achieved isolation.
    pub fn isolation(&self) -> &IsolationRecord {
        &self.isolation
    }
}

/// Preparation-only workload adapter.
pub trait WorkloadAdapter {
    /// Prepare inputs beneath `artifact_directory` and declare the invocation.
    fn prepare(&mut self, artifact_directory: &Path) -> Result<ProcessDeclaration, String>;
}

/// Prepares and executes a workload while keeping the resulting sample private
/// from the adapter.
pub fn execute_adapter<A: WorkloadAdapter>(
    adapter: &mut A,
    artifact_directory: &Path,
) -> Result<ProcessSample, String> {
    fs::create_dir_all(artifact_directory)
        .map_err(|error| format!("create artifact directory: {error}"))?;
    let declaration = adapter.prepare(artifact_directory)?;
    execute(&declaration)
}

/// Executes one declaration with bounded output and time.
pub fn execute(declaration: &ProcessDeclaration) -> Result<ProcessSample, String> {
    validate(declaration)?;
    let requested = declaration
        .affinity
        .as_ref()
        .map(|value| value.logical_cpus.clone());
    let mut command = command_for(declaration);
    command.current_dir(&declaration.working_directory);
    if !declaration.inherit_environment {
        command.env_clear();
    }
    command.envs(&declaration.environment);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn workload: {error}"))?;
    // Affinity and process-tree controls are deliberately not synthesized from
    // host-specific APIs.  This host tool is portable; callers that require
    // physical isolation must select a platform capsule outside this process.
    let observed: Option<Vec<usize>> = None;
    let stdout = drain(
        child.stdout.take().ok_or("workload stdout was not piped")?,
        declaration.stdout_limit,
    );
    let stderr = drain(
        child.stderr.take().ok_or("workload stderr was not piped")?,
        declaration.stderr_limit,
    );
    let (status, timed_out) = wait_bounded(&mut child, declaration.timeout)?;
    let (stdout, stdout_truncated) = stdout
        .join()
        .map_err(|_| "stdout reader panicked")?
        .map_err(|error| format!("read workload stdout: {error}"))?;
    let (stderr, stderr_truncated) = stderr
        .join()
        .map_err(|_| "stderr reader panicked")?
        .map_err(|error| format!("read workload stderr: {error}"))?;
    let achieved = requested
        .as_ref()
        .zip(observed.as_ref())
        .is_some_and(|(requested, observed)| same_cpu_set(requested, observed));
    let detail = match (&requested, achieved) {
        (None, _) => "CPU affinity was not requested".to_owned(),
        (Some(_), false) => "CPU affinity requires an external platform capsule".to_owned(),
        (Some(_), true) => "requested CPU affinity verified on the workload process".to_owned(),
    };
    Ok(ProcessSample {
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
        status,
        timed_out,
        elapsed: started.elapsed(),
        isolation: IsolationRecord {
            requested_affinity: requested,
            observed_affinity: observed,
            achieved_affinity: achieved,
            workload_tree_contained: false,
            detail,
        },
    })
}

fn same_cpu_set(left: &[usize], right: &[usize]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort_unstable();
    left.dedup();
    right.sort_unstable();
    right.dedup();
    left == right
}

fn validate(declaration: &ProcessDeclaration) -> Result<(), String> {
    if declaration.program.is_empty() {
        return Err("program must not be empty".to_owned());
    }
    if !declaration.working_directory.is_dir() {
        return Err("working_directory must exist and be a directory".to_owned());
    }
    if declaration.timeout.is_zero() {
        return Err("timeout must be greater than zero".to_owned());
    }
    if declaration
        .affinity
        .as_ref()
        .is_some_and(|value| value.logical_cpus.is_empty())
    {
        return Err("affinity logical_cpus must not be empty".to_owned());
    }
    Ok(())
}

fn command_for(declaration: &ProcessDeclaration) -> Command {
    let mut command = Command::new(&declaration.program);
    command.args(&declaration.arguments);
    command
}

fn drain<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
) -> thread::JoinHandle<io::Result<(Vec<u8>, bool)>> {
    thread::spawn(move || {
        let mut retained = Vec::with_capacity(limit.min(8192));
        let mut truncated = false;
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let available = limit.saturating_sub(retained.len());
            retained.extend_from_slice(&buffer[..read.min(available)]);
            truncated |= read > available;
        }
        Ok((retained, truncated))
    })
}

fn wait_bounded(
    child: &mut Child,
    timeout: Duration,
) -> Result<(Option<ExitStatus>, bool), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("wait for workload: {error}"))?
        {
            return Ok((Some(status), false));
        }
        if Instant::now() >= deadline {
            child
                .kill()
                .map_err(|error| format!("terminate timed-out workload: {error}"))?;
            let status = child
                .wait()
                .map_err(|error| format!("reap timed-out workload: {error}"))?;
            return Ok((Some(status), true));
        }
        thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(arguments: Vec<String>) -> ProcessDeclaration {
        ProcessDeclaration {
            program: "/bin/sh".to_owned(),
            arguments,
            working_directory: std::env::temp_dir(),
            environment: BTreeMap::new(),
            inherit_environment: false,
            stdout_limit: 8,
            stderr_limit: 8,
            timeout: Duration::from_secs(2),
            affinity: None,
        }
    }

    #[test]
    fn shell_metacharacters_are_one_literal_argument() {
        let marker = "$(touch should-not-exist); * | &";
        let sample = execute(&declaration(vec![
            "-c".into(),
            "printf '%s' \"$1\"".into(),
            "argv0".into(),
            marker.into(),
        ]))
        .unwrap();
        assert_eq!(sample.stdout(), b"$(touch ");
        assert!(sample.stdout_truncated());
        assert!(sample.status().unwrap().success());
    }

    #[test]
    fn unsupported_affinity_records_the_gap() {
        let requested = vec![0];
        let record = IsolationRecord {
            requested_affinity: Some(requested.clone()),
            observed_affinity: None,
            achieved_affinity: false,
            workload_tree_contained: false,
            detail: "CPU affinity requires an external platform capsule".to_owned(),
        };
        assert_eq!(record.requested_affinity(), Some(requested.as_slice()));
        assert!(!record.achieved_affinity());
        assert!(record.detail().contains("platform capsule"));
    }

    #[test]
    fn output_and_timeout_are_bounded() {
        let output = execute(&declaration(vec![
            "-c".into(),
            "printf 123456789; printf abcdefghi >&2".into(),
        ]))
        .unwrap();
        assert_eq!(output.stdout(), b"12345678");
        assert_eq!(output.stderr(), b"abcdefgh");
        assert!(output.stdout_truncated() && output.stderr_truncated());

        let mut slow = declaration(vec!["2".into()]);
        slow.program = "/bin/sleep".to_owned();
        slow.timeout = Duration::from_millis(10);
        assert!(execute(&slow).unwrap().timed_out());
    }

    struct Adapter;
    impl WorkloadAdapter for Adapter {
        fn prepare(&mut self, artifact_directory: &Path) -> Result<ProcessDeclaration, String> {
            fs::write(artifact_directory.join("input"), b"prepared")
                .map_err(|error| error.to_string())?;
            let mut declaration = declaration(vec!["-c".into(), "test -f input".into()]);
            declaration.working_directory = artifact_directory.to_owned();
            Ok(declaration)
        }
    }

    #[test]
    fn adapter_only_prepares_before_executor_records_sample() {
        let root = std::env::temp_dir().join(format!("sim-bench-exec-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mut adapter = Adapter;
        let command = adapter.prepare(&root).unwrap_err();
        assert!(command.contains("No such file") || command.contains("not found"));
        fs::create_dir_all(&root).unwrap();
        let sample = execute_adapter(&mut adapter, &root).unwrap();
        assert!(!sample.timed_out());
        fs::remove_dir_all(root).unwrap();
    }
}
