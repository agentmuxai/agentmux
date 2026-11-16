// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether this srv was started headless, for the core code that decides what the process may do (the build cache
//! cleaner). `main` sets it once, right after it has read the command line (`headless::requested()`); the
//! headless module (`headless.rs`, startup glue) stays where it is. Unset means not headless, as in the tests.

use std::sync::atomic::{AtomicBool, Ordering};

static HEADLESS: AtomicBool = AtomicBool::new(false);

/// Record that this process was started headless (`--headless` or `AGENTMUX_HEADLESS=1`).
pub fn set_headless(headless: bool) {
    HEADLESS.store(headless, Ordering::SeqCst);
}

/// Was this process started headless? False until `set_headless` says so.
pub fn headless() -> bool {
    HEADLESS.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_is_not_headless_and_the_flag_follows_the_last_call() {
        assert!(!headless());
        set_headless(true);
        assert!(headless());
        set_headless(false);
        assert!(!headless());
    }
}
