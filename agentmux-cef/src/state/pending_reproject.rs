// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/// See `AppState::pending_reproject_closures`. A thin, named wrapper around
/// `HashMap<new_label, old_window_id>` rather than a bare map — reagent P2
/// (PR #2032, 2026-07-08) noted the deferred-close logic had no automated
/// test coverage; giving it named methods makes the intent explicit and
/// gives something to unit-test directly, without needing a live instance.
#[derive(Debug, Default)]
pub struct PendingReprojectClosures(std::collections::HashMap<String, String>);

impl PendingReprojectClosures {
    /// Record that `new_label`'s eventual `register_backend_window` call
    /// should trigger closing `old_window_id`. Called right after
    /// `reproject_from_snapshot` returns — before any confirmation the new
    /// window actually exists.
    pub fn stage(&mut self, new_label: String, old_window_id: String) {
        self.0.insert(new_label, old_window_id);
    }

    /// Called from `register_backend_window` for every label that
    /// registers. Returns the old window_id to close if `label` was staged
    /// (removing the entry so it fires at most once); `None` for any
    /// ordinary (non-reprojected) window registration.
    pub fn confirm(&mut self, label: &str) -> Option<String> {
        self.0.remove(label)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.len()
    }
}

#[cfg(test)]
mod pending_reproject_closures_tests {
    use super::*;

    #[test]
    fn confirm_returns_none_for_unstaged_label() {
        let mut p = PendingReprojectClosures::default();
        assert_eq!(p.confirm("window-abc"), None);
    }

    #[test]
    fn confirm_returns_the_staged_old_id() {
        let mut p = PendingReprojectClosures::default();
        p.stage("window-new".to_string(), "old-srv-id".to_string());
        assert_eq!(p.confirm("window-new"), Some("old-srv-id".to_string()));
    }

    #[test]
    fn confirm_only_fires_once() {
        let mut p = PendingReprojectClosures::default();
        p.stage("window-new".to_string(), "old-srv-id".to_string());
        assert_eq!(p.confirm("window-new"), Some("old-srv-id".to_string()));
        // A second registration for the same label (shouldn't normally
        // happen, but must not re-trigger a close) gets nothing.
        assert_eq!(p.confirm("window-new"), None);
    }

    #[test]
    fn confirm_is_independent_per_label() {
        let mut p = PendingReprojectClosures::default();
        p.stage("window-a".to_string(), "old-a".to_string());
        p.stage("window-b".to_string(), "old-b".to_string());
        assert_eq!(p.len(), 2);
        assert_eq!(p.confirm("window-a"), Some("old-a".to_string()));
        assert_eq!(p.len(), 1);
        assert_eq!(p.confirm("window-b"), Some("old-b".to_string()));
        assert_eq!(p.len(), 0);
    }

    #[test]
    fn unrelated_label_registration_is_a_noop() {
        // e.g. a pool window, or main itself — never staged, must not panic
        // or spuriously return something.
        let mut p = PendingReprojectClosures::default();
        p.stage("window-real-reproject".to_string(), "old-id".to_string());
        assert_eq!(p.confirm("window-pool-abc123"), None);
        assert_eq!(p.confirm("main"), None);
        // The real one is still there, untouched.
        assert_eq!(p.confirm("window-real-reproject"), Some("old-id".to_string()));
    }
}
