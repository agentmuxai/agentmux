// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Which files the person attached while a pane was handed to them, for the agent's answer.
//!
//! The person picks the file in the browser's own dialog; the agent never sees the file or where it is. What it
//! is told is the names of the files that are in the pane's file inputs now and were not when the hand-off began
//! (`file_inputs`, read by the host from the page's own inputs: names only). When the hand-off ends with Done, the
//! agent reads "the person attached: a.pdf" or "no file was attached", or that nothing could be checked. A
//! page that empties its input once it has taken the file, a drop zone and a closed shadow root are not seen.

use serde_json::json;

use super::ui_handlers::{get_host_ipc, proxy_to_host_timeout};
use super::AppState;

/// The most names put in a sentence; the rest are counted.
const SHOWN: usize = 10;
/// How long the host gets to read a pane's inputs when Done was clicked: the agent is waiting on the answer.
const READ_WITHIN: std::time::Duration = std::time::Duration::from_secs(8);
/// And for the baseline, taken before the person's card goes up: short, so the card is not held back (a read that
/// does not come in time leaves the agent told that nothing could be checked).
const BASELINE_WITHIN: std::time::Duration = std::time::Duration::from_secs(2);

/// The file names in a pane's file inputs, and how many of its frames could not be read.
async fn names_in(state: &AppState, pane: &str, within: std::time::Duration) -> Result<(Vec<String>, usize), String> {
    let host = get_host_ipc(state).await?;
    let v = proxy_to_host_timeout(state, &host, "file_inputs", json!({ "block_id": pane }), Some(within)).await?;
    if v.get("ok").and_then(|o| o.as_bool()) == Some(false) {
        return Err(v.get("error").and_then(|e| e.as_str()).unwrap_or("couldn't read the page's file inputs").to_string());
    }
    let names = v
        .pointer("/data/names")
        .and_then(|n| n.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let unreadable = v.pointer("/data/unreadable").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
    Ok((names, unreadable))
}

/// What the pane's file inputs held when a hand-off began, to tell what the person added.
pub(crate) struct Watch {
    before: Option<Vec<String>>,
}

impl Watch {
    pub(crate) async fn begin(state: &AppState, pane: &str) -> Self {
        Self { before: names_in(state, pane, BASELINE_WITHIN).await.ok().map(|(names, _)| names) }
    }

    /// The sentence for the agent once the person clicked Done: what they attached.
    pub(crate) async fn sentence(&self, state: &AppState, pane: &str) -> String {
        sentence(self.before.as_deref(), names_in(state, pane, READ_WITHIN).await)
    }
}

/// The names in `now` that `before` did not have, counting repeats (the same name twice is two files).
fn added(before: &[String], now: &[String]) -> Vec<String> {
    let mut left: Vec<&String> = before.iter().collect();
    let mut out = Vec::new();
    for n in now {
        match left.iter().position(|b| *b == n) {
            Some(i) => {
                left.swap_remove(i);
            }
            None => out.push(n.clone()),
        }
    }
    out
}

/// One name as the agent reads it: no control characters, masked as short text is, in quotes.
fn shown(name: &str) -> String {
    let clean: String = name.chars().filter(|c| !c.is_control()).collect();
    format!("\"{}\"", agentmux_common::pii_mask::mask_short_text(&clean))
}

/// The agent's sentence from what the inputs held before and now.
fn sentence(before: Option<&[String]>, now: Result<(Vec<String>, usize), String>) -> String {
    let (Some(before), Ok((now, unreadable))) = (before, now) else {
        return "AgentMux could not check whether a file was attached: look at the page in a snapshot.".to_string();
    };
    let new = added(before, &now);
    let unseen = if unreadable > 0 { format!(" ({unreadable} frame(s) could not be read.)") } else { String::new() };
    if new.is_empty() {
        return format!("No file was attached to a file input.{unseen}");
    }
    let names: Vec<String> = new.iter().take(SHOWN).map(|n| shown(n)).collect();
    let more = if new.len() > SHOWN { format!(" and {} more", new.len() - SHOWN) } else { String::new() };
    format!("The person attached: {}{more}. You did not receive the file itself.{unseen}", names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_files_that_were_not_there_before_count_and_repeats_count_twice() {
        assert_eq!(added(&v(&["old.pdf"]), &v(&["old.pdf", "a.pdf"])), v(&["a.pdf"]));
        assert_eq!(added(&v(&["a.pdf"]), &v(&["a.pdf", "a.pdf"])), v(&["a.pdf"]), "a second file with the same name");
        assert!(added(&v(&["a.pdf"]), &v(&["a.pdf"])).is_empty());
        assert!(added(&v(&["a.pdf"]), &[]).is_empty(), "a removed file is not an attachment");
    }

    #[test]
    fn the_agent_is_told_the_names_and_that_it_has_not_the_file() {
        let s = sentence(Some(&v(&[])), Ok((v(&["a.pdf", "b.png"]), 0)));
        assert_eq!(s, "The person attached: \"a.pdf\", \"b.png\". You did not receive the file itself.");
    }

    #[test]
    fn no_new_file_says_so_and_counts_the_frames_it_could_not_read() {
        assert_eq!(sentence(Some(&v(&["a.pdf"])), Ok((v(&["a.pdf"]), 0))), "No file was attached to a file input.");
        assert!(sentence(Some(&v(&[])), Ok((v(&[]), 2))).contains("2 frame(s) could not be read"));
    }

    #[test]
    fn a_read_that_failed_is_not_taken_for_no_file() {
        assert!(sentence(None, Ok((v(&["a.pdf"]), 0))).contains("could not check"), "no baseline");
        assert!(sentence(Some(&v(&[])), Err("no host".into())).contains("could not check"), "no answer now");
    }

    #[test]
    fn many_names_are_counted_and_odd_characters_are_dropped() {
        let many: Vec<String> = (0..13).map(|i| format!("f{i}.txt")).collect();
        let s = sentence(Some(&[]), Ok((many, 0)));
        assert!(s.contains("and 3 more") && s.contains("\"f9.txt\"") && !s.contains("f10.txt"), "{s}");
        assert_eq!(shown("a\u{0}\nb.pdf"), "\"ab.pdf\"");
    }
}
