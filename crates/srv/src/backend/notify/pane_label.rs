// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a notification calls a pane that holds no agent: its kind and title, so the
//! countdown to close a browser or an editor says what is closing, not "An agent".

use crate::backend::obj::{meta_get_string, MetaMapType};
use crate::backend::tower_agentmux::page_host;

use super::router::is_bidi_control;

/// The longest title shown after the kind.
const TITLE_MAX: usize = 48;

/// `browser pane: Azure: subscriptions`, `editor: notes.md`, or just the kind when
/// the pane has no title. The title is the pane's own (`frame:title`), else what
/// its view shows: a page's host, a file's name, a folder's name. Never a full URL or
/// path, which can carry a user, a token or a directory the user keeps private.
pub fn pane_label(meta: &MetaMapType) -> String {
    let get = |key: &str| meta_get_string(meta, key, "");
    let view = get("view");
    let kind = match view.as_str() {
        "browser" => "browser pane".to_string(),
        "editor" => "editor".to_string(),
        "term" => "terminal".to_string(),
        "files" => "file browser".to_string(),
        "sysinfo" => "system info pane".to_string(),
        "help" => "help pane".to_string(),
        "tower" => "Tower pane".to_string(),
        "" => "pane".to_string(),
        other => format!("{} pane", one_line(other, 24)),
    };
    let title = [get("frame:title")].into_iter().find(|t| !t.trim().is_empty()).or_else(|| match view.as_str() {
        "browser" => page_host(&get("url")),
        "editor" => last_segment(&get("file")),
        "files" => last_segment(&get("files:path")),
        _ => None,
    });
    match title.map(|t| one_line(&t, TITLE_MAX)).filter(|t| !t.is_empty()) {
        Some(t) => format!("{kind}: {t}"),
        None => kind,
    }
}

/// The last part of a path, whichever way it separates.
fn last_segment(path: &str) -> Option<String> {
    path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).map(str::to_string)
}

/// One line: bidi-override characters removed, control characters turned into
/// spaces, whitespace collapsed, cut at `max` characters.
fn one_line(raw: &str, max: usize) -> String {
    let cleaned: String = raw.chars().filter(|c| !is_bidi_control(*c)).map(|c| if c.is_control() { ' ' } else { c }).collect();
    let line = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(pairs: &[(&str, &str)]) -> MetaMapType {
        pairs.iter().map(|(k, v)| (k.to_string(), serde_json::json!(v))).collect()
    }

    #[test]
    fn a_browser_pane_is_named_by_its_title() {
        assert_eq!(
            pane_label(&meta(&[("view", "browser"), ("url", "https://portal.azure.com/x"), ("frame:title", "Azure: subscriptions")])),
            "browser pane: Azure: subscriptions"
        );
    }

    #[test]
    fn an_untitled_browser_pane_gives_its_host_and_nothing_more_of_the_url() {
        assert_eq!(
            pane_label(&meta(&[("view", "browser"), ("url", "https://me:secret@example.com:8443/a/b?token=x")])),
            "browser pane: example.com"
        );
        assert_eq!(pane_label(&meta(&[("view", "browser"), ("url", "not a url")])), "browser pane");
    }

    #[test]
    fn an_editor_gives_the_file_name_not_the_path() {
        assert_eq!(pane_label(&meta(&[("view", "editor"), ("file", "C:\\work\\private\\notes.md")])), "editor: notes.md");
        assert_eq!(pane_label(&meta(&[("view", "editor"), ("file", "/home/me/a/b.rs"), ("frame:title", "plan")])), "editor: plan");
    }

    #[test]
    fn other_panes_are_named_by_their_kind_and_never_as_an_agent() {
        assert_eq!(pane_label(&meta(&[("view", "files"), ("files:path", "/home/me/proj/")])), "file browser: proj");
        assert_eq!(pane_label(&meta(&[("view", "term")])), "terminal");
        assert_eq!(pane_label(&meta(&[("view", "tower")])), "Tower pane");
        assert_eq!(pane_label(&meta(&[("view", "widgetx")])), "widgetx pane");
        assert_eq!(pane_label(&meta(&[])), "pane");
    }

    #[test]
    fn a_title_is_one_short_clean_line() {
        let long = format!("{}\u{202E}tail", "x".repeat(80));
        let label = pane_label(&meta(&[("view", "browser"), ("frame:title", &format!("  a\nb\t{long}"))]));
        assert!(label.starts_with("browser pane: a b xxx"), "{label}");
        assert!(!label.contains('\u{202E}') && !label.contains('\n'), "{label}");
        assert_eq!(label.chars().count(), "browser pane: ".chars().count() + TITLE_MAX + 1, "{label}");
    }
}
