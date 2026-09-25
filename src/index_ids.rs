// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Stable identifier slugs for SIM Index subjects written by the xtask index
//! tools (seed, source, doctor).

use std::path::Path;

pub(crate) fn slug_path(input: &str) -> String {
    input
        .split('/')
        .map(slug_ident)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn slug_ident(input: &str) -> String {
    let mut out = String::new();
    let mut previous_dash = false;
    let mut previous_lower_or_digit = false;
    for byte in input.bytes() {
        let ch = match byte {
            b'a'..=b'z' | b'0'..=b'9' => {
                previous_dash = false;
                previous_lower_or_digit = true;
                byte as char
            }
            b'_' | b'.' => {
                previous_dash = false;
                previous_lower_or_digit = false;
                byte as char
            }
            b'A'..=b'Z' => {
                if previous_lower_or_digit && !previous_dash {
                    out.push('-');
                }
                previous_dash = false;
                previous_lower_or_digit = false;
                byte.to_ascii_lowercase() as char
            }
            b'-' => {
                if previous_dash {
                    continue;
                }
                previous_dash = true;
                previous_lower_or_digit = false;
                '-'
            }
            _ => {
                if previous_dash {
                    continue;
                }
                previous_dash = true;
                previous_lower_or_digit = false;
                '-'
            }
        };
        out.push(ch);
    }
    out.trim_matches('-').to_owned()
}

pub(crate) fn repo_name(repo: &Path) -> String {
    repo.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace")
        .to_owned()
}
