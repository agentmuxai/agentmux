// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What an own Save As dialog came to, for a pane download (#196).
//!
//! Plain data and functions, so the answers are tested without a dialog. `downloads.rs` runs the dialog and
//! acts on the answer.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::commands::file_dialog::Outcome;

/// What the Save As dialog came to.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum SaveAnswer {
    /// The person picked this file.
    Chosen(PathBuf),
    /// The person closed the dialog.
    Cancelled,
    /// The dialog could not open or died (a panic in its thread, a missing portal): nobody answered.
    Failed,
}

/// Run the blocking dialog `ask` so that a panic inside it is an answer too. Without this a dead dialog thread
/// never posted anything back: the download waited for an answer for good, and so did every download behind it.
///
/// `can_ask` is [`crate::commands::file_dialog::can_ask`], checked before the dialog: when nothing can show one the
/// answer is `Failed` at once and `ask` never runs. A "nothing chosen" that returns sooner than a person could
/// have closed a window ([`crate::commands::file_dialog::QUICK_CANCEL`]) is `Failed` too: the dialog never opened.
pub(super) fn ask_guarded(can_ask: bool, ask: impl FnOnce() -> Outcome) -> SaveAnswer {
    if !can_ask {
        return SaveAnswer::Failed;
    }
    let started = Instant::now();
    answer_from(std::panic::catch_unwind(std::panic::AssertUnwindSafe(ask)), started.elapsed())
}

/// What a finished dialog call came to, from its result and how long it took.
fn answer_from(done: std::thread::Result<Outcome>, took: Duration) -> SaveAnswer {
    match done {
        Ok(Outcome::Chosen(path)) => SaveAnswer::Chosen(path),
        Ok(Outcome::Cancelled) if took < crate::commands::file_dialog::QUICK_CANCEL => SaveAnswer::Failed,
        Ok(Outcome::Cancelled) => SaveAnswer::Cancelled,
        Ok(Outcome::Failed(_)) | Err(_) => SaveAnswer::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chosen(p: &str) -> Outcome {
        Outcome::Chosen(PathBuf::from(p))
    }

    #[test]
    fn a_dialog_that_panics_is_an_answer_not_a_wait_for_ever() {
        assert_eq!(ask_guarded(true, || chosen("/d/a.txt")), SaveAnswer::Chosen(PathBuf::from("/d/a.txt")));
        // The Linux portal panic of 2026-11-20: "there is no reactor running".
        assert_eq!(ask_guarded(true, || panic!("there is no reactor running")), SaveAnswer::Failed);
        // And a dialog thread that has failed once does not stop the next one.
        assert_eq!(ask_guarded(true, || chosen("/d/b.txt")), SaveAnswer::Chosen(PathBuf::from("/d/b.txt")));
    }

    #[test]
    fn a_dialog_that_could_not_open_is_not_a_cancel() {
        assert_eq!(ask_guarded(true, || Outcome::Failed("no portal".into())), SaveAnswer::Failed);
    }

    #[test]
    fn with_no_dialog_backend_nothing_is_asked_and_the_download_is_saved_without_asking() {
        // Linux with no session bus and no zenity: rfd would say "nothing chosen" at once.
        let mut asked = false;
        assert_eq!(
            ask_guarded(false, || {
                asked = true;
                Outcome::Cancelled
            }),
            SaveAnswer::Failed
        );
        assert!(!asked, "the dialog is not tried");
    }

    #[test]
    fn a_cancel_too_quick_to_be_a_person_is_a_dialog_that_never_opened() {
        assert_eq!(answer_from(Ok(Outcome::Cancelled), Duration::from_millis(20)), SaveAnswer::Failed);
        assert_eq!(answer_from(Ok(Outcome::Cancelled), Duration::from_secs(3)), SaveAnswer::Cancelled, "a person closed it");
        assert_eq!(answer_from(Ok(chosen("/d/a.txt")), Duration::from_millis(20)), SaveAnswer::Chosen(PathBuf::from("/d/a.txt")));
    }
}
