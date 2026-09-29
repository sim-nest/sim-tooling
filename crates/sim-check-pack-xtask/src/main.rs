// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Binary entry point for the isolated SIM conformance-pack command.

#![forbid(unsafe_code)]

fn main() {
    if let Err(err) = sim_check_pack_xtask::run(std::env::args().collect()) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
