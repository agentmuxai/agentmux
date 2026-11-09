// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A page's own file chooser (a script clicking an `<input type=file>`, a label, a "Browse" button) in
//! a browser pane an agent drives (docs/reports/REPORT_AGENTS_AND_OS_PROMPTS_2026_11_07.md §3, tier 1).
//! Without a handler CEF opens the OS dialog on the person's screen, which CDP can't see or answer, so
//! the pane waits behind it until someone notices. Here the host cancels the chooser at once and tells
//! srv, which tells the agent to put the files in with `Browser` `set_files`. When the agent holds the
//! uploads power on the page's site, srv answers "hold" and the host keeps the chooser for 30 seconds
//! for `set_files` with `chooser: true` (`chooser_hold.rs`). Panes no agent drives, and panes handed to
//! the person, keep CEF's own dialog: the person is there to answer it.

use std::sync::Arc;

use cef::*;

/// What the page asked for, in the words the agent is told. `None` for a Save As: that is the dialog
/// a download opens (`downloads.rs` answers `on_before_download` with `show_dialog = true`, and CEF
/// routes it through this same chooser), which the person answers, so it is never cancelled here.
fn ask_of(mode: FileDialogMode) -> Option<&'static str> {
    if mode == FileDialogMode::OPEN {
        Some("choose a file")
    } else if mode == FileDialogMode::OPEN_MULTIPLE {
        Some("choose several files")
    } else if mode == FileDialogMode::OPEN_FOLDER {
        Some("choose a folder")
    } else {
        None
    }
}

wrap_dialog_handler! {
    pub(crate) struct AgentMuxDialogHandler {
        state: Arc<crate::state::AppState>,
    }

    impl DialogHandler {
        fn on_file_dialog(
            &self,
            browser: Option<&mut Browser>,
            mode: FileDialogMode,
            title: Option<&CefString>,
            _default_file_path: Option<&CefString>,
            accept_filters: Option<&mut CefStringList>,
            _accept_extensions: Option<&mut CefStringList>,
            _accept_descriptions: Option<&mut CefStringList>,
            callback: Option<&mut FileDialogCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(browser), Some(callback)) = (browser, callback) else { return 0 };
            let Some(ask) = ask_of(mode) else { return 0 };
            let Some((_pane, target)) = super::jsdialog::driven(&self.state, browser) else { return 0 };
            let title = title.map(|t| t.to_string()).unwrap_or_default();
            let origin = crate::browser_pane::callbacks::main_frame_url(browser);
            let body = serde_json::json!({ "target": target, "kind": "file_chooser", "chooser": ask, "message": title, "origin": origin });
            if mode == FileDialogMode::OPEN_FOLDER {
                // Cancelled in this call, so the page and the pane never wait on it.
                callback.cancel();
                tracing::info!(target = %target, ask, "browser pane: a page's file chooser in an agent-driven pane was cancelled");
                super::jsdialog::notify(&self.state, body);
            } else {
                // Kept while srv decides: cancelled at once unless the agent holds the uploads power here.
                // `into_iter` takes the list by value: swap in an empty one (CEF still owns the original).
                let accept = accept_filters.map(|l| std::mem::take(l).into_iter().collect()).unwrap_or_default();
                tracing::info!(target = %target, ask, "browser pane: a page's file chooser in an agent-driven pane, asking srv");
                super::chooser_hold::park(&self.state, &target, callback, mode == FileDialogMode::OPEN_MULTIPLE, accept, &origin, body);
            }
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_of_chooser_is_named_for_the_agent() {
        assert_eq!(ask_of(FileDialogMode::OPEN), Some("choose a file"));
        assert_eq!(ask_of(FileDialogMode::OPEN_MULTIPLE), Some("choose several files"));
        assert_eq!(ask_of(FileDialogMode::OPEN_FOLDER), Some("choose a folder"));
    }

    #[test]
    fn a_save_as_is_left_to_the_person_because_a_download_opens_one() {
        assert_eq!(ask_of(FileDialogMode::SAVE), None);
    }
}
