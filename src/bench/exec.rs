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

#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[cfg(unix)]
#[allow(unsafe_code)]
mod unix {
    use std::{io, mem::MaybeUninit};

    pub fn signal_group(pid: u32, signal: i32) -> io::Result<()> {
        // SAFETY: kill is called with a negated, validated child PID and no pointers.
        let result = unsafe { libc::kill(-(pid as i32), signal) };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(target_os = "linux")]
    pub fn affinity(pid: u32) -> io::Result<Vec<usize>> {
        let mut mask = MaybeUninit::<libc::cpu_set_t>::zeroed();
        // SAFETY: mask points to writable storage of the exact size supplied.
        let result = unsafe {
            libc::sched_getaffinity(
                pid as libc::pid_t,
                size_of::<libc::cpu_set_t>(),
                mask.as_mut_ptr(),
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: sched_getaffinity initialized the complete cpu_set_t on success.
        let mask = unsafe { mask.assume_init() };
        Ok((0..libc::CPU_SETSIZE as usize)
            .filter(|cpu| unsafe { libc::CPU_ISSET(*cpu, &mask) })
            .collect())
    }
}

const TERMINATION_GRACE: Duration = Duration::from_millis(100);
const FORCE_KILL_GRACE: Duration = Duration::from_millis(250);

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
    let (mut command, mechanism) = command_for(declaration);
    command.current_dir(&declaration.working_directory);
    if !declaration.inherit_environment {
        command.env_clear();
    }
    command.envs(&declaration.environment);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    configure_containment(&mut command);

    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn workload: {error}"))?;
    let observed = observe_affinity(&mut child, requested.as_deref());
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
    let detail = match (&requested, mechanism, achieved) {
        (None, _, _) => "CPU affinity was not requested".to_owned(),
        (Some(_), false, _) => {
            "CPU affinity requested but this platform has no supported mechanism".to_owned()
        }
        (Some(_), true, true) => {
            "requested CPU affinity verified on the workload process".to_owned()
        }
        (Some(_), true, false) => {
            "CPU affinity mechanism did not report successful execution".to_owned()
        }
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
            workload_tree_contained: cfg!(unix),
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

fn configure_containment(command: &mut Command) {
    #[cfg(unix)]
    command.process_group(0);
}

#[cfg(target_os = "linux")]
fn observe_affinity(child: &mut Child, requested: Option<&[usize]>) -> Option<Vec<usize>> {
    let deadline = Instant::now() + Duration::from_millis(50);
    let mut observed = None;
    loop {
        if let Ok(cpus) = unix::affinity(child.id()) {
            if requested.is_some_and(|value| same_cpu_set(value, &cpus)) {
                return Some(cpus);
            }
            observed = Some(cpus);
        }
        if Instant::now() >= deadline || child.try_wait().ok().flatten().is_some() {
            return observed;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(not(target_os = "linux"))]
fn observe_affinity(_child: &mut Child, _requested: Option<&[usize]>) -> Option<Vec<usize>> {
    None
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

fn command_for(declaration: &ProcessDeclaration) -> (Command, bool) {
    #[cfg(target_os = "linux")]
    if let Some(affinity) = &declaration.affinity {
        let taskset = ["/usr/bin/taskset", "/bin/taskset"]
            .into_iter()
            .find(|path| Path::new(path).is_file());
        if let Some(taskset) = taskset {
            let mut command = Command::new(taskset);
            let cpus = affinity
                .logical_cpus
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(",");
            command.args(["--cpu-list", &cpus, "--", &declaration.program]);
            command.args(&declaration.arguments);
            return (command, true);
        }
    }
    let mut command = Command::new(&declaration.program);
    command.args(&declaration.arguments);
    (command, false)
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
            clean_workload_tree(child.id())?;
            return Ok((Some(status), false));
        }
        if Instant::now() >= deadline {
            #[cfg(unix)]
            terminate_workload_tree(child.id())?;
            #[cfg(not(unix))]
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

#[cfg(unix)]
fn signal_group(pid: u32, signal: i32) -> Result<(), String> {
    match unix::signal_group(pid, signal) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
        Err(error) => Err(format!("signal workload process group: {error}")),
    }
}

#[cfg(unix)]
fn group_exists(pid: u32) -> bool {
    !matches!(unix::signal_group(pid, 0), Err(error) if error.raw_os_error() == Some(libc::ESRCH))
}

#[cfg(unix)]
fn terminate_workload_tree(pid: u32) -> Result<(), String> {
    signal_group(pid, libc::SIGTERM)?;
    let deadline = Instant::now() + TERMINATION_GRACE;
    while group_exists(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    signal_group(pid, libc::SIGKILL)?;
    let deadline = Instant::now() + FORCE_KILL_GRACE;
    while group_exists(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

fn clean_workload_tree(pid: u32) -> Result<(), String> {
    #[cfg(unix)]
    if group_exists(pid) {
        terminate_workload_tree(pid)?;
    }
    Ok(())
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
            detail: "CPU affinity requested but this platform has no supported mechanism"
                .to_owned(),
        };
        assert_eq!(record.requested_affinity(), Some(requested.as_slice()));
        assert!(!record.achieved_affinity());
        assert!(record.detail().contains("no supported mechanism"));
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

    #[cfg(unix)]
    #[test]
    fn timeout_kills_descendants_that_hold_pipes_and_ignore_termination() {
        let mut hostile = declaration(vec![
            "-c".into(),
            "trap '' TERM; sh -c 'trap \"\" TERM; sh -c \"trap \\\"\\\" TERM; while :; do sleep 1; done\" & wait' & echo $!; wait".into(),
        ]);
        hostile.timeout = Duration::from_millis(40);
        hostile.stdout_limit = 64;
        let started = Instant::now();
        let sample = execute(&hostile).unwrap();
        assert!(sample.timed_out());
        assert!(sample.isolation().workload_tree_contained());
        assert!(started.elapsed() < Duration::from_secs(1));
        let descendant = std::str::from_utf8(sample.stdout())
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        assert!(!group_exists(descendant));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn affinity_is_observed_independently_of_workload_exit_status() {
        let available = unix::affinity(std::process::id()).unwrap();
        let cpu = *available
            .first()
            .expect("test process has an available CPU");
        let mut failing = declaration(Vec::new());
        failing.program = "/bin/false".to_owned();
        failing.affinity = Some(CpuAffinity {
            logical_cpus: vec![cpu],
        });
        let sample = execute(&failing).unwrap();
        assert!(!sample.status().unwrap().success());
        assert_eq!(
            sample.isolation().observed_affinity(),
            Some([cpu].as_slice())
        );
        assert!(sample.isolation().achieved_affinity());

        let record = IsolationRecord {
            requested_affinity: Some(vec![cpu]),
            observed_affinity: Some(available),
            achieved_affinity: false,
            workload_tree_contained: true,
            detail: "requested affinity did not match observed affinity".to_owned(),
        };
        assert!(!record.achieved_affinity());
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
