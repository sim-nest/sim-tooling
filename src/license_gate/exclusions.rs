// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The declared `license-exclusions.toml` manifest.
//!
//! ```toml
//! [[exclude]]
//! glob = "src/*_generated.rs"
//! kind = "generated"
//! reason = "emitted by build.rs from the opcode table"
//! ```

use std::{fs, path::Path};

use serde::Deserialize;

pub(crate) const MANIFEST: &str = "license-exclusions.toml";

const KINDS: &[&str] = &["generated", "third-party", "golden", "frozen-evidence"];

#[derive(Debug, Default)]
pub(crate) struct Exclusions {
    globs: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    #[serde(default)]
    exclude: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    glob: String,
    kind: String,
    reason: String,
}

impl Exclusions {
    pub(crate) fn load(root: &Path) -> Result<Self, String> {
        match fs::read_to_string(root.join(MANIFEST)) {
            Err(_) => Ok(Self::default()),
            Ok(text) => Self::parse(&text),
        }
    }

    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let manifest: Manifest =
            toml::from_str(text).map_err(|err| format!("{MANIFEST}: {err}"))?;
        let mut globs = Vec::new();
        for entry in manifest.exclude {
            if !KINDS.contains(&entry.kind.as_str()) {
                return Err(format!(
                    "{MANIFEST}: {}: kind must be one of {}",
                    entry.glob,
                    KINDS.join(", ")
                ));
            }
            if entry.reason.trim().is_empty() {
                return Err(format!("{MANIFEST}: {}: reason is empty", entry.glob));
            }
            globs.push(entry.glob);
        }
        Ok(Self { globs })
    }

    pub(crate) fn matches(&self, rel: &str) -> bool {
        self.globs.iter().any(|glob| glob_matches(glob, rel))
    }
}

/// Matches a `/`-separated path against a glob where `*` and `?` stay within
/// one segment and a whole `**` segment spans any number of segments.
pub(crate) fn glob_matches(glob: &str, path: &str) -> bool {
    let glob: Vec<&str> = glob.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    segments_match(&glob, &path)
}

fn segments_match(glob: &[&str], path: &[&str]) -> bool {
    match glob.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|skip| segments_match(rest, &path[skip..])),
        Some((first, rest)) => path
            .split_first()
            .is_some_and(|(head, tail)| segment_matches(first, head) && segments_match(rest, tail)),
    }
}

fn segment_matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    chars_match(&pattern, &text)
}

fn chars_match(pattern: &[char], text: &[char]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some(('*', rest)) => (0..=text.len()).any(|skip| chars_match(rest, &text[skip..])),
        Some(('?', rest)) => !text.is_empty() && chars_match(rest, &text[1..]),
        Some((c, rest)) => text.first() == Some(c) && chars_match(rest, &text[1..]),
    }
}
