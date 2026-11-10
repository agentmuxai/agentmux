// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The restart splash's "Window paint" row, reported once per host process.
//!
//! Every arm of a top-level window's paint gate used to report a new stage
//! begin, so a main page that kept failing to load (a dev frontend server that
//! was down) added one row per load retry, and the splash showed a growing
//! list of counters (operator screenshot, 2026-11-10). One begin per process
//! is all the splash needs; later arms and retries are the same wait.

use std::sync::OnceLock;
use std::time::Instant;

const STAGE: &str = "paint";

static BEGAN_AT: OnceLock<Instant> = OnceLock::new();

/// Report the "paint" stage begin the first time a window arms its paint
/// gate in this process, and never again.
pub(super) fn report_begin_once() {
    if BEGAN_AT.set(Instant::now()).is_ok() {
        crate::launcher_ipc::report_startup_stage_begin(STAGE, "Window paint");
    }
}

/// The main page failed to load and will be retried: say why on the paint row
/// instead of leaving it counting up. A later successful reveal ends the row
/// again, as ok. Does nothing before the first paint gate was armed.
pub(super) fn report_load_retry(error_text: &str) {
    let Some(began) = BEGAN_AT.get() else { return };
    crate::launcher_ipc::report_startup_stage_end(STAGE, began.elapsed().as_millis() as u64, "warn", Some(retry_detail(error_text)));
}

fn retry_detail(error_text: &str) -> String {
    let why = error_text.trim();
    if why.is_empty() {
        "Can't reach the app page, retrying".to_string()
    } else {
        format!("Can't reach the app page: {why}, retrying")
    }
}

#[cfg(test)]
mod tests {
    use super::retry_detail;

    #[test]
    fn the_retry_detail_names_the_error() {
        assert_eq!(retry_detail("ERR_CONNECTION_REFUSED"), "Can't reach the app page: ERR_CONNECTION_REFUSED, retrying");
        assert_eq!(retry_detail("  "), "Can't reach the app page, retrying");
    }
}
