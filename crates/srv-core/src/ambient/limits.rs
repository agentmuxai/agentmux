// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How many ambient CLI calls may run at once. `AmbientGateway` coalesces and
//! cancels only within one `(entity, purpose)` key; without a cap, N panes
//! finishing a turn at the same moment would each spawn their own CLI.
//!
//! Two classes, each with one cap, so the total is bounded and a burst of
//! background work can never hold up a call the user is waiting on. These
//! replace six separate semaphores (pull calls, definition previews, backlog
//! naming, narration, the recovery sweep's own, and none at all for the
//! continuity summary), each justified alone, none bounding the total.
//! docs/reports/REPORT_AMBIENT_FRAMEWORK_REASSESSMENT_2026_10_08.md section 6.4.

use std::time::Duration;

use tokio::sync::Semaphore;

/// Which queue an ambient call waits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Shown to the user as a result of what they just did: the pane's title, the
    /// next-message suggestion, a name for a row they opened or a subagent that
    /// just started.
    Interactive,
    /// Fill-in nobody is waiting on: title recovery, previews, backlog naming,
    /// narration, the continuity summary.
    Background,
}

/// Simultaneous interactive calls, across all panes.
pub const INTERACTIVE_CAP: usize = 2;
/// Simultaneous background calls, across all panes. One more than interactive:
/// a continuity summary can run for up to 90 s, and narration should not wait
/// behind two of them.
pub const BACKGROUND_CAP: usize = 3;

/// The longest an interactive call waits for a permit before giving up. With
/// the 30 s interactive time limit (`purpose`), a call ends within 40 s, inside
/// the pane's 45 s RPC timeout (`ambient-rpc.ts`), so a reply is never paid for
/// after the pane has stopped waiting for it.
pub const INTERACTIVE_MAX_QUEUE_WAIT: Duration = Duration::from_secs(10);

impl Class {
    /// How long a call of this class may wait for a permit; `None` for no limit
    /// (nobody is waiting on background work).
    pub fn max_queue_wait(self) -> Option<Duration> {
        match self {
            Class::Interactive => Some(INTERACTIVE_MAX_QUEUE_WAIT),
            Class::Background => None,
        }
    }

    /// The class's process-wide semaphore.
    pub fn semaphore(self) -> &'static Semaphore {
        static INTERACTIVE: std::sync::OnceLock<Semaphore> = std::sync::OnceLock::new();
        static BACKGROUND: std::sync::OnceLock<Semaphore> = std::sync::OnceLock::new();
        match self {
            Class::Interactive => INTERACTIVE.get_or_init(|| Semaphore::new(INTERACTIVE_CAP)),
            Class::Background => BACKGROUND.get_or_init(|| Semaphore::new(BACKGROUND_CAP)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_class_is_capped_and_the_two_are_separate() {
        let interactive = Class::Interactive.semaphore();
        let background = Class::Background.semaphore();
        assert!(!std::ptr::eq(interactive, background), "background work must not queue behind interactive work");
        // Capacity, read without taking permits other tests might be holding.
        assert!(interactive.available_permits() <= INTERACTIVE_CAP);
        assert!(background.available_permits() <= BACKGROUND_CAP);
        assert!(std::ptr::eq(interactive, Class::Interactive.semaphore()), "one semaphore per class, process-wide");
    }
}
