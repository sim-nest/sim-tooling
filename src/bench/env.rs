//! Typed environment evidence and fail-closed benchmark compatibility.
//!
//! A [`DeclaredHost`] is supplied by the control plane's `test-hosts.toml`
//! inventory. Probing augments that identity with observations; it never
//! invents identity from an observed hostname or a display label.

// conformance: typed host evidence refuses unavailable or incompatible comparisons.

use std::collections::BTreeSet;
use std::fs;

use serde::{Deserialize, Serialize};

use super::BuildIdentity;

/// Stable, structured identity copied from one `test-hosts.toml` host row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredHost {
    /// Stable inventory key of the host row.
    pub inventory_id: String,
    /// Declared SSH host, kept separate from the inventory identity.
    pub ssh_host: String,
}

impl DeclaredHost {
    /// Constructs an inventory-backed host identity.
    pub fn new(inventory_id: String, ssh_host: String) -> Result<Self, String> {
        let mut errors = Vec::new();
        required("host.inventory_id", &inventory_id, &mut errors);
        required("host.ssh_host", &ssh_host, &mut errors);
        if errors.is_empty() {
            Ok(Self {
                inventory_id,
                ssh_host,
            })
        } else {
            Err(errors.join("; "))
        }
    }
}

/// Evidence for one probed field. Unavailability is data, not a guessed value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ProbeEvidence<T> {
    /// The probe returned an explicit value.
    Available {
        /// Observed field value.
        value: T,
    },
    /// The probe could not establish a value.
    Unavailable {
        /// Exact reason the field could not be established.
        reason: String,
    },
}

impl<T> ProbeEvidence<T> {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }
}

/// Material execution-host observations used to admit benchmark comparisons.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostEnvironment {
    /// Inventory-backed identity, not a free-form machine label.
    pub host: DeclaredHost,
    /// Operating-system family.
    pub operating_system: ProbeEvidence<String>,
    /// CPU architecture.
    pub architecture: ProbeEvidence<String>,
    /// CPU model reported by the operating system.
    pub cpu_model: ProbeEvidence<String>,
    /// Number of logical CPUs available to the process.
    pub logical_cpus: ProbeEvidence<u32>,
    /// Total physical memory in bytes.
    pub memory_bytes: ProbeEvidence<u64>,
    /// Active CPU frequency governor.
    pub cpu_governor: ProbeEvidence<String>,
}

/// Build observations paired with the host evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildEnvironment {
    /// Source revision.
    pub source_revision: ProbeEvidence<String>,
    /// Rust compilation target.
    pub target: ProbeEvidence<String>,
    /// Cargo profile.
    pub profile: ProbeEvidence<String>,
    /// Canonically ordered enabled features.
    pub features: ProbeEvidence<Vec<String>>,
    /// Rust toolchain identity.
    pub toolchain: ProbeEvidence<String>,
}

/// Complete material environment evidence for one benchmark run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentProbe {
    /// Execution-host evidence.
    pub host: HostEnvironment,
    /// Executable-build evidence.
    pub build: BuildEnvironment,
}

/// Injectable source for host observations, allowing unavailable reads to be tested.
pub trait HostProbeSource {
    /// Reads a UTF-8 operating-system file.
    fn read(&self, path: &str) -> Result<String, String>;
    /// Returns the process architecture.
    fn architecture(&self) -> Result<String, String>;
    /// Returns the process-visible logical CPU count.
    fn logical_cpus(&self) -> Result<u32, String>;
}

/// Host probe source backed by the local operating system.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalHostProbe;

impl HostProbeSource for LocalHostProbe {
    fn read(&self, path: &str) -> Result<String, String> {
        fs::read_to_string(path).map_err(|error| error.to_string())
    }

    fn architecture(&self) -> Result<String, String> {
        Ok(std::env::consts::ARCH.to_owned())
    }

    fn logical_cpus(&self) -> Result<u32, String> {
        std::thread::available_parallelism()
            .map_err(|error| error.to_string())
            .and_then(|count| {
                u32::try_from(count.get()).map_err(|_| "logical CPU count exceeds u32".to_owned())
            })
    }
}

/// Probes every material host and build field without substituting defaults.
pub fn probe_environment(
    host: DeclaredHost,
    build: &BuildIdentity,
    source: &impl HostProbeSource,
) -> EnvironmentProbe {
    let operating_system = source
        .read("/etc/os-release")
        .and_then(|text| os_release_value(&text, "ID"))
        .map_or_else(ProbeEvidence::unavailable, available);
    let architecture = source
        .architecture()
        .and_then(non_empty)
        .map_or_else(ProbeEvidence::unavailable, available);
    let cpu_model = source
        .read("/proc/cpuinfo")
        .and_then(|text| cpuinfo_value(&text, "model name"))
        .map_or_else(ProbeEvidence::unavailable, available);
    let logical_cpus = source
        .logical_cpus()
        .and_then(|count| {
            (count > 0)
                .then_some(count)
                .ok_or("reported zero logical CPUs".to_owned())
        })
        .map_or_else(ProbeEvidence::unavailable, available);
    let memory_bytes = source
        .read("/proc/meminfo")
        .and_then(|text| memory_bytes(&text))
        .map_or_else(ProbeEvidence::unavailable, available);
    let cpu_governor = source
        .read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
        .and_then(non_empty)
        .map_or_else(ProbeEvidence::unavailable, available);

    EnvironmentProbe {
        host: HostEnvironment {
            host,
            operating_system,
            architecture,
            cpu_model,
            logical_cpus,
            memory_bytes,
            cpu_governor,
        },
        build: BuildEnvironment::from(build),
    }
}

impl From<&BuildIdentity> for BuildEnvironment {
    fn from(build: &BuildIdentity) -> Self {
        let mut features = build.features.clone();
        features.sort();
        features.dedup();
        Self {
            source_revision: evidence_string(
                &build.source_revision,
                "build source revision is empty",
            ),
            target: evidence_string(&build.target, "build target is empty"),
            profile: evidence_string(&build.profile, "build profile is empty"),
            features: ProbeEvidence::Available { value: features },
            toolchain: evidence_string(&build.toolchain, "build toolchain is empty"),
        }
    }
}

/// Typed material field understood by compatibility policy.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvironmentField {
    /// Inventory host key.
    HostInventoryId,
    /// Declared SSH host.
    HostSshHost,
    /// Operating-system family.
    OperatingSystem,
    /// CPU architecture.
    Architecture,
    /// CPU model.
    CpuModel,
    /// Logical CPU count.
    LogicalCpus,
    /// Physical memory.
    MemoryBytes,
    /// CPU frequency governor.
    CpuGovernor,
    /// Source revision.
    BuildSourceRevision,
    /// Rust target.
    BuildTarget,
    /// Cargo profile.
    BuildProfile,
    /// Enabled features.
    BuildFeatures,
    /// Toolchain identity.
    BuildToolchain,
}

impl EnvironmentField {
    /// Stable diagnostic name for this field.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::HostInventoryId => "host.inventory-id",
            Self::HostSshHost => "host.ssh-host",
            Self::OperatingSystem => "host.operating-system",
            Self::Architecture => "host.architecture",
            Self::CpuModel => "host.cpu-model",
            Self::LogicalCpus => "host.logical-cpus",
            Self::MemoryBytes => "host.memory-bytes",
            Self::CpuGovernor => "host.cpu-governor",
            Self::BuildSourceRevision => "build.source-revision",
            Self::BuildTarget => "build.target",
            Self::BuildProfile => "build.profile",
            Self::BuildFeatures => "build.features",
            Self::BuildToolchain => "build.toolchain",
        }
    }
}

/// Policy controlling which material fields must match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityPolicy {
    required_equal: BTreeSet<EnvironmentField>,
}

impl CompatibilityPolicy {
    /// Constructs a policy from external field names, rejecting unknown names.
    pub fn from_required_names<'a>(
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, String> {
        let mut required_equal = BTreeSet::new();
        let mut errors = Vec::new();
        for name in names {
            match field_named(name) {
                Some(field) => {
                    required_equal.insert(field);
                }
                None => errors.push(format!("unknown required environment field `{name}`")),
            }
        }
        if errors.is_empty() {
            Ok(Self { required_equal })
        } else {
            Err(errors.join("; "))
        }
    }

    /// Constructs a policy from typed fields.
    pub fn requiring(fields: impl IntoIterator<Item = EnvironmentField>) -> Self {
        Self {
            required_equal: fields.into_iter().collect(),
        }
    }
}

/// One reason why two runs cannot be compared.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityMismatch {
    /// Material field which failed admission.
    pub field: EnvironmentField,
    /// Explicit refusal reason.
    pub reason: String,
}

/// Applies policy and returns every incompatibility instead of stopping at the first.
pub fn compatibility_mismatches(
    policy: &CompatibilityPolicy,
    baseline: &EnvironmentProbe,
    candidate: &EnvironmentProbe,
) -> Vec<CompatibilityMismatch> {
    policy
        .required_equal
        .iter()
        .filter_map(|field| compare_field(field, baseline, candidate))
        .collect()
}

fn compare_field(
    field: &EnvironmentField,
    baseline: &EnvironmentProbe,
    candidate: &EnvironmentProbe,
) -> Option<CompatibilityMismatch> {
    let result = match field {
        EnvironmentField::HostInventoryId => compare_values(
            &baseline.host.host.inventory_id,
            &candidate.host.host.inventory_id,
        ),
        EnvironmentField::HostSshHost => {
            compare_values(&baseline.host.host.ssh_host, &candidate.host.host.ssh_host)
        }
        EnvironmentField::OperatingSystem => compare_evidence(
            &baseline.host.operating_system,
            &candidate.host.operating_system,
        ),
        EnvironmentField::Architecture => {
            compare_evidence(&baseline.host.architecture, &candidate.host.architecture)
        }
        EnvironmentField::CpuModel => {
            compare_evidence(&baseline.host.cpu_model, &candidate.host.cpu_model)
        }
        EnvironmentField::LogicalCpus => {
            compare_evidence(&baseline.host.logical_cpus, &candidate.host.logical_cpus)
        }
        EnvironmentField::MemoryBytes => {
            compare_evidence(&baseline.host.memory_bytes, &candidate.host.memory_bytes)
        }
        EnvironmentField::CpuGovernor => {
            compare_evidence(&baseline.host.cpu_governor, &candidate.host.cpu_governor)
        }
        EnvironmentField::BuildSourceRevision => compare_evidence(
            &baseline.build.source_revision,
            &candidate.build.source_revision,
        ),
        EnvironmentField::BuildTarget => {
            compare_evidence(&baseline.build.target, &candidate.build.target)
        }
        EnvironmentField::BuildProfile => {
            compare_evidence(&baseline.build.profile, &candidate.build.profile)
        }
        EnvironmentField::BuildFeatures => {
            compare_evidence(&baseline.build.features, &candidate.build.features)
        }
        EnvironmentField::BuildToolchain => {
            compare_evidence(&baseline.build.toolchain, &candidate.build.toolchain)
        }
    };
    result.map(|detail| CompatibilityMismatch {
        field: field.clone(),
        reason: format!("{}: {detail}", field.name()),
    })
}

fn compare_values<T: PartialEq>(baseline: &T, candidate: &T) -> Option<String> {
    (baseline != candidate).then(|| "baseline and candidate differ".to_owned())
}

fn compare_evidence<T: PartialEq>(
    baseline: &ProbeEvidence<T>,
    candidate: &ProbeEvidence<T>,
) -> Option<String> {
    match (baseline, candidate) {
        (ProbeEvidence::Available { value: left }, ProbeEvidence::Available { value: right }) => {
            compare_values(left, right)
        }
        (ProbeEvidence::Unavailable { reason }, _) => {
            Some(format!("baseline unavailable: {reason}"))
        }
        (_, ProbeEvidence::Unavailable { reason }) => {
            Some(format!("candidate unavailable: {reason}"))
        }
    }
}

fn available<T>(value: T) -> ProbeEvidence<T> {
    ProbeEvidence::Available { value }
}

fn evidence_string(value: &str, reason: &str) -> ProbeEvidence<String> {
    if value.trim().is_empty() {
        ProbeEvidence::unavailable(reason)
    } else {
        available(value.trim().to_owned())
    }
}

fn non_empty(value: String) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err("field was empty".to_owned())
    } else {
        Ok(trimmed.to_owned())
    }
}

fn os_release_value(text: &str, key: &str) -> Result<String, String> {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .map(|value| value.trim_matches('"').to_owned())
        .ok_or_else(|| format!("{key} is absent from os-release"))
        .and_then(non_empty)
}

fn cpuinfo_value(text: &str, key: &str) -> Result<String, String> {
    text.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim() == key).then(|| value.trim().to_owned())
        })
        .ok_or_else(|| format!("{key} is absent from cpuinfo"))
        .and_then(non_empty)
}

fn memory_bytes(text: &str) -> Result<u64, String> {
    let line = text
        .lines()
        .find(|line| line.starts_with("MemTotal:"))
        .ok_or_else(|| "MemTotal is absent from meminfo".to_owned())?;
    let mut words = line.split_whitespace();
    let _label = words.next();
    let kib = words
        .next()
        .ok_or_else(|| "MemTotal has no value".to_owned())?
        .parse::<u64>()
        .map_err(|error| format!("MemTotal is invalid: {error}"))?;
    match words.next() {
        Some("kB") => kib
            .checked_mul(1024)
            .ok_or_else(|| "MemTotal overflows bytes".to_owned()),
        unit => Err(format!("MemTotal has unsupported unit {unit:?}")),
    }
}

fn field_named(name: &str) -> Option<EnvironmentField> {
    [
        EnvironmentField::HostInventoryId,
        EnvironmentField::HostSshHost,
        EnvironmentField::OperatingSystem,
        EnvironmentField::Architecture,
        EnvironmentField::CpuModel,
        EnvironmentField::LogicalCpus,
        EnvironmentField::MemoryBytes,
        EnvironmentField::CpuGovernor,
        EnvironmentField::BuildSourceRevision,
        EnvironmentField::BuildTarget,
        EnvironmentField::BuildProfile,
        EnvironmentField::BuildFeatures,
        EnvironmentField::BuildToolchain,
    ]
    .into_iter()
    .find(|field| field.name() == name)
}

fn required(field: &str, value: &str, errors: &mut Vec<String>) {
    if value.trim().is_empty() {
        errors.push(format!("{field} must not be empty"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixtureProbe {
        governor: Result<String, String>,
    }

    impl HostProbeSource for FixtureProbe {
        fn read(&self, path: &str) -> Result<String, String> {
            match path {
                "/etc/os-release" => Ok("ID=linux\n".to_owned()),
                "/proc/cpuinfo" => Ok("model name: Fixture CPU\n".to_owned()),
                "/proc/meminfo" => Ok("MemTotal: 1024 kB\n".to_owned()),
                _ => self.governor.clone(),
            }
        }

        fn architecture(&self) -> Result<String, String> {
            Ok("x86_64".to_owned())
        }

        fn logical_cpus(&self) -> Result<u32, String> {
            Ok(8)
        }
    }

    fn build() -> BuildIdentity {
        BuildIdentity {
            source_revision: "abc".to_owned(),
            target: "x86_64-unknown-linux-gnu".to_owned(),
            profile: "release".to_owned(),
            features: Vec::new(),
            toolchain: "rustc 1.90".to_owned(),
        }
    }

    fn probe() -> EnvironmentProbe {
        probe_environment(
            DeclaredHost::new("tiger".to_owned(), "tiger".to_owned()).unwrap(),
            &build(),
            &FixtureProbe {
                governor: Ok("performance\n".to_owned()),
            },
        )
    }

    #[test]
    fn different_cpu_models_are_refused_and_name_the_field() {
        let baseline = probe();
        let mut candidate = probe();
        candidate.host.cpu_model = available("Different CPU".to_owned());
        let mismatches = compatibility_mismatches(
            &CompatibilityPolicy::requiring([EnvironmentField::CpuModel]),
            &baseline,
            &candidate,
        );
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].field, EnvironmentField::CpuModel);
        assert!(mismatches[0].reason.contains("host.cpu-model"));
    }

    #[test]
    fn unreadable_governor_is_recorded_as_unavailable() {
        let result = probe_environment(
            DeclaredHost::new(
                "unreadable-governor-fixture".to_owned(),
                "governor-fixture.example.invalid".to_owned(),
            )
            .unwrap(),
            &build(),
            &FixtureProbe {
                governor: Err("permission denied".to_owned()),
            },
        );
        assert_eq!(
            result.host.cpu_governor,
            ProbeEvidence::Unavailable {
                reason: "permission denied".to_owned()
            }
        );
    }

    #[test]
    fn unknown_required_policy_field_is_a_construction_error() {
        let error =
            CompatibilityPolicy::from_required_names(["host.cpu-model", "host.label"]).unwrap_err();
        assert!(error.contains("unknown required environment field `host.label`"));
    }

    #[test]
    fn reports_every_required_mismatch() {
        let baseline = probe();
        let mut candidate = probe();
        candidate.host.cpu_model = available("other".to_owned());
        candidate.build.profile = available("debug".to_owned());
        let mismatches = compatibility_mismatches(
            &CompatibilityPolicy::requiring([
                EnvironmentField::CpuModel,
                EnvironmentField::BuildProfile,
            ]),
            &baseline,
            &candidate,
        );
        assert_eq!(mismatches.len(), 2);
    }

    #[test]
    fn build_probe_covers_all_identity_fields() {
        let evidence = BuildEnvironment::from(&BuildIdentity { ..build() });
        assert!(matches!(
            evidence.source_revision,
            ProbeEvidence::Available { .. }
        ));
        assert!(matches!(evidence.target, ProbeEvidence::Available { .. }));
        assert!(matches!(evidence.profile, ProbeEvidence::Available { .. }));
        assert!(matches!(evidence.features, ProbeEvidence::Available { .. }));
        assert!(matches!(
            evidence.toolchain,
            ProbeEvidence::Available { .. }
        ));
    }
}
