// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/// The directory a shell starts in: the pane's `cmd:cwd`, or for a local
/// shell without one, the user's home. Without that default a new terminal
/// inherited srv's own working directory: wherever the app was started from,
/// and inside the checkout's `dist/` in a dev build
/// (docs/specs/PLAN_SHORTCUT_KINKS_2026_10_10.md, A6). An SSH or WSL pane's
/// directory is a path on the other side, so it gets no local default.
pub(crate) fn spawn_cwd(pane_cwd: &str, remote: bool) -> String {
    if !pane_cwd.is_empty() || remote {
        return pane_cwd.to_string();
    }
    crate::backend::base::get_home_dir().to_string_lossy().to_string()
}

#[cfg(test)]
mod spawn_cwd_tests {
    use super::spawn_cwd;

    #[test]
    fn keeps_the_panes_own_directory() {
        assert_eq!(spawn_cwd("/work/proj", false), "/work/proj");
        assert_eq!(spawn_cwd("/work/proj", true), "/work/proj");
    }

    #[test]
    fn a_local_shell_without_one_starts_at_home() {
        let home = crate::backend::base::get_home_dir().to_string_lossy().to_string();
        assert_eq!(spawn_cwd("", false), home);
    }

    #[test]
    fn a_remote_shell_gets_no_local_default() {
        assert_eq!(spawn_cwd("", true), "");
    }
}
