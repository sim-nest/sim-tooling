// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Binary entry point for the standalone documentation encoder.

#![forbid(unsafe_code)]

fn main() {
    if let Err(err) = simdoc::run(std::env::args().collect()) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
