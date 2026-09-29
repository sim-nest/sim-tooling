// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Child processes whose captured output is bounded.
//!
//! Both pipes are read concurrently in fixed-size chunks. When either stream
//! exceeds its ceiling the child is killed at once and the run is refused with
//! a classified error; the reader keeps draining (and discarding) so that a
//! grandchild still holding the pipe can never block the parent.

use std::{
    io::Read,
    process::{Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
};

/// Captured output of a bounded run.
#[derive(Debug)]
pub(crate) struct Captured {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

enum Event {
    Overflow,
    Done(usize, Result<Vec<u8>, String>),
}

/// Runs `command` with no stdin, capturing at most `stdout_limit` and
/// `stderr_limit` bytes. Exceeding either ceiling kills the child and returns
/// an error naming `role` and the ceiling.
pub(crate) fn run_bounded(
    mut command: Command,
    role: &str,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<Captured, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("start {role}: {err}"))?;
    let (sender, receiver) = mpsc::channel();
    let stdout = child.stdout.take().ok_or("child stdout was not piped")?;
    let stderr = child.stderr.take().ok_or("child stderr was not piped")?;
    let readers = [
        spawn_reader(0, stdout, stdout_limit, sender.clone()),
        spawn_reader(1, stderr, stderr_limit, sender),
    ];

    let mut streams: [Option<Result<Vec<u8>, String>>; 2] = [None, None];
    let mut overflowed = false;
    while streams.iter().any(Option::is_none) {
        match receiver.recv() {
            Ok(Event::Overflow) if !overflowed => {
                overflowed = true;
                let _ = child.kill();
            }
            Ok(Event::Overflow) => {}
            Ok(Event::Done(index, result)) => streams[index] = Some(result),
            Err(_) => break,
        }
    }
    for reader in readers {
        let _ = reader.join();
    }
    let status = child
        .wait()
        .map_err(|err| format!("wait for {role}: {err}"))?;
    if overflowed {
        return Err(format!(
            "{role} output exceeded its bound (stdout {stdout_limit} bytes, stderr {stderr_limit} bytes); the run was terminated"
        ));
    }
    let [stdout, stderr] = streams;
    Ok(Captured {
        status,
        stdout: stdout.ok_or_else(|| format!("{role} stdout was not collected"))??,
        stderr: stderr.ok_or_else(|| format!("{role} stderr was not collected"))??,
    })
}

fn spawn_reader(
    index: usize,
    mut pipe: impl Read + Send + 'static,
    limit: usize,
    sender: mpsc::Sender<Event>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut kept = Vec::new();
        let mut chunk = [0_u8; 8192];
        let mut over = false;
        let result = loop {
            match pipe.read(&mut chunk) {
                Ok(0) => break Ok(kept),
                Ok(read) if over => {
                    let _ = read;
                }
                Ok(read) => {
                    if kept.len() + read > limit {
                        over = true;
                        kept.clear();
                        let _ = sender.send(Event::Overflow);
                    } else {
                        kept.extend_from_slice(&chunk[..read]);
                    }
                }
                Err(err) => break Err(format!("read child output: {err}")),
            }
        };
        let _ = sender.send(Event::Done(index, result));
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_within_bounds_is_captured() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf out; printf err >&2"]);
        let captured = run_bounded(command, "fixture", 16, 16).unwrap();
        assert!(captured.status.success());
        assert_eq!(captured.stdout, b"out");
        assert_eq!(captured.stderr, b"err");
    }

    #[test]
    fn an_unbounded_producer_is_terminated_and_refused() {
        let mut command = Command::new("sh");
        command.args(["-c", "while :; do printf 0123456789; done"]);
        let err = run_bounded(command, "fixture", 1024, 1024).unwrap_err();
        assert!(err.contains("fixture output exceeded its bound"), "{err}");

        let mut command = Command::new("sh");
        command.args(["-c", "while :; do printf 0123456789 >&2; done"]);
        let err = run_bounded(command, "fixture", 1024, 64).unwrap_err();
        assert!(err.contains("the run was terminated"), "{err}");
    }
}
