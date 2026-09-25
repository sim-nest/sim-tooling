// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The only programs simdoc ever launches, and how it launches them.
//!
//! Nothing is looked up on `PATH`. The toolchain is the directory holding the
//! `cargo` that started this process (`CARGO` is absolute when cargo runs the
//! encoder), and it is used only after its `cargo`, `rustc`, `rustdoc`, and
//! libraries hash to the digest fixed into this executable when it was built
//! and their `-vV` commits equal the ones it was built with. `git` is taken
//! from a fixed list of root-owned system locations. Every child runs with a
//! cleared environment holding only an allowlist and a `PATH` of the
//! toolchain directory and the system binary directories, so a substituted
//! `cargo`, `rustc`, `rustdoc`, or `git` earlier on the caller's `PATH` is
//! never reached, and no wrapper or flag variable steers a build.
//!
//! Every process spawn in this crate goes through this module (a source test
//! enforces it), and nothing is ever run through a shell.

use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use crate::build_identity::{tool_identity, toolchain_digest};

/// Environment variables children may inherit.
const PASSED_ENVIRONMENT: [&str; 7] = [
    "HOME",
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "CARGO_NET_OFFLINE",
    "LANG",
    "LC_ALL",
    "TMPDIR",
];
/// System binary directories a child's `PATH` may include, after the
/// toolchain directory.
const SYSTEM_PATH: &str = "/usr/bin:/bin";
/// Root-owned locations `git` may be taken from.
const GIT_LOCATIONS: [&str; 3] = ["/usr/bin/git", "/bin/git", "/usr/local/bin/git"];

/// What this executable was built with: the identity a toolchain must match.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ExpectedToolchain {
    pub(crate) sha256: &'static str,
    pub(crate) rustc_release: &'static str,
    pub(crate) rustc_commit: &'static str,
    pub(crate) cargo_release: &'static str,
    pub(crate) cargo_commit: &'static str,
}

/// The identity fixed into this executable at build time.
pub(crate) const BUILT_WITH: ExpectedToolchain = ExpectedToolchain {
    sha256: env!("SIMDOC_TOOLCHAIN_SHA256"),
    rustc_release: env!("SIMDOC_RUSTC_RELEASE"),
    rustc_commit: env!("SIMDOC_RUSTC_COMMIT"),
    cargo_release: env!("SIMDOC_CARGO_RELEASE"),
    cargo_commit: env!("SIMDOC_CARGO_COMMIT"),
};

/// The verified programs.
#[derive(Clone, Debug)]
pub(crate) struct Tools {
    cargo: PathBuf,
    rustc: PathBuf,
    rustdoc: PathBuf,
    git: PathBuf,
    bin: PathBuf,
}

/// The verified programs of this process, resolved once.
pub(crate) fn tools() -> Result<&'static Tools, String> {
    static TOOLS: OnceLock<Result<Tools, String>> = OnceLock::new();
    TOOLS
        .get_or_init(|| {
            let cargo = env::var_os("CARGO").map(PathBuf::from).ok_or(
                "CARGO is not set: run simdoc through `cargo run` (or an xtask launcher), which \
                 names the toolchain; simdoc never looks programs up on PATH",
            )?;
            Tools::verify(&cargo, &BUILT_WITH, &GIT_LOCATIONS)
        })
        .as_ref()
        .map_err(Clone::clone)
}

impl Tools {
    /// Verifies the toolchain whose `cargo` is `cargo` against `expected`,
    /// and takes `git` from the first of `git_locations` that is a root-owned
    /// ordinary executable nobody else can write.
    pub(crate) fn verify(
        cargo: &Path,
        expected: &ExpectedToolchain,
        git_locations: &[&str],
    ) -> Result<Self, String> {
        if !cargo.is_absolute() {
            return Err(format!(
                "CARGO={} is not an absolute path; refusing to look programs up on PATH",
                cargo.display()
            ));
        }
        let bin = cargo
            .parent()
            .ok_or("cargo has no directory")?
            .to_path_buf();
        let root = bin
            .parent()
            .ok_or("cargo does not sit in a toolchain bin directory")?;
        let digest = toolchain_digest(root)
            .map_err(|err| format!("digest the toolchain of {}: {err}", cargo.display()))?;
        if digest != expected.sha256 {
            return Err(format!(
                "the toolchain beside {} is not the one this simdoc was built with (digest \
                 {digest}, built with {})",
                cargo.display(),
                expected.sha256
            ));
        }
        let rustc = bin.join("rustc");
        let rustdoc = bin.join("rustdoc");
        for (tool, path, release, commit) in [
            (
                "cargo",
                cargo,
                expected.cargo_release,
                expected.cargo_commit,
            ),
            (
                "rustc",
                &rustc,
                expected.rustc_release,
                expected.rustc_commit,
            ),
        ] {
            let (found_release, found_commit) = reported_identity(path)?;
            if found_release != release || found_commit != commit {
                return Err(format!(
                    "{tool} at {} reports {found_release} ({found_commit}), not the {release} \
                     ({commit}) this simdoc was built with",
                    path.display()
                ));
            }
        }
        Ok(Self {
            cargo: cargo.to_path_buf(),
            rustc,
            rustdoc,
            git: trusted_git(git_locations)?,
            bin,
        })
    }

    /// A `cargo` invocation with the scrubbed environment.
    pub(crate) fn cargo(&self) -> Command {
        self.command(&self.cargo)
    }

    /// A `rustc` invocation with the scrubbed environment.
    pub(crate) fn rustc(&self) -> Command {
        self.command(&self.rustc)
    }

    /// A `git` invocation with the scrubbed environment and no user or
    /// system Git configuration.
    pub(crate) fn git(&self) -> Command {
        let mut command = self.command(&self.git);
        command
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0");
        command
    }

    fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command.env_clear();
        for name in PASSED_ENVIRONMENT {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut path = OsString::from(&self.bin);
        path.push(":");
        path.push(SYSTEM_PATH);
        command
            .env("PATH", path)
            .env("CARGO", &self.cargo)
            .env("RUSTC", &self.rustc)
            .env("RUSTDOC", &self.rustdoc)
            .env("RUSTC_WRAPPER", "")
            .env("RUSTC_WORKSPACE_WRAPPER", "");
        command
    }
}

fn reported_identity(tool: &Path) -> Result<(String, String), String> {
    // The verified programs are run by absolute path with an empty
    // environment: nothing the caller exported can change what they report.
    let output = Command::new(tool)
        .env_clear()
        .args(["--version", "--verbose"])
        .output()
        .map_err(|err| format!("run {} -vV: {err}", tool.display()))?;
    tool_identity(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| format!("{} -vV reports no release or commit", tool.display()))
}

/// The first of `locations` that is an ordinary file owned by root that no
/// group or other user can write.
fn trusted_git(locations: &[&str]) -> Result<PathBuf, String> {
    for location in locations {
        let path = Path::new(location);
        let Ok(metadata) = fs::metadata(path) else {
            continue;
        };
        if metadata.is_file() && metadata.uid() == 0 && metadata.mode() & 0o022 == 0 {
            return Ok(path.to_path_buf());
        }
    }
    Err(format!(
        "no root-owned git at {}; simdoc never looks git up on PATH",
        locations.join(", ")
    ))
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
