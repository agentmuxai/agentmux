// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent's summary line as other instances see it: the line under its Swarm
//! row, sent with its status on `GET /agentmux/reactive/agent-names` so this
//! machine's other channels and LAN hosts can show it on their remote rows
//! (SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md §3.4 Phase 4).
//!
//! The line is the frontend's `readSwarmSummary`: the generated title when it
//! is usable, else the CLI's OSC title. The local row's fallbacks (a restored
//! title, a one-liner from the last prompt, a status phrase) are not sent.
//!
//! Sent unless `swarm:sharesummaries` is off, by the owner's decision of
//! 2026-10-13 (on by default, every tier). [`clean_line`] bounds a line on both
//! sides: a peer is untrusted.

use std::collections::BTreeMap;

use crate::backend::obj::{self, MetaMapType};
use crate::backend::storage::store::Store;
use crate::backend::wconfig::SettingsType;

/// The CLI's own window title (`frontend/app/store/meta-keys.ts`).
pub const META_OSC_TITLE: &str = "term:osc_title";

/// Longest line sent or kept, in characters. A stored title is at most 200.
pub const MAX_LINE_CHARS: usize = 200;

/// "Share what my agents are working on". Absent means on. The key is written
/// out at the read so the settings-defaults check finds this default.
pub fn sharing_on(settings: &SettingsType) -> bool {
    settings.extra.get("swarm:sharesummaries").and_then(|v| v.as_bool()).unwrap_or(true)
}

/// One line: whitespace and control characters folded to single spaces,
/// trimmed and capped at [`MAX_LINE_CHARS`]. `None` when nothing is left.
pub fn clean_line(text: &str) -> Option<String> {
    let folded = text
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let line: String = folded.chars().take(MAX_LINE_CHARS).collect();
    (!line.is_empty()).then_some(line)
}

/// The line for a block's meta, or `None` when it has none to show.
pub fn line_from_meta(meta: &MetaMapType) -> Option<String> {
    let title = obj::meta_get_string(meta, crate::ambient::title::META_TITLE, "");
    if crate::ambient::validate::is_usable_title(&title) {
        if let Some(line) = clean_line(&title) {
            return Some(line);
        }
    }
    clean_line(&obj::meta_get_string(meta, META_OSC_TITLE, ""))
}

/// The line of the agent on `block_id`. Reads the store.
pub fn agent_line_of_block(mstore: &Store, block_id: &str) -> Option<String> {
    let block = mstore.get::<obj::Block>(block_id).ok()??;
    line_from_meta(&block.meta)
}

/// Each agent's line by name, an agent with none left out, for the names
/// route; `None` when `swarm:sharesummaries` is off. `agents` is
/// `(name, block_id)`. Reads the store.
pub fn lines_by_name<'a>(
    mstore: &Store,
    settings: &SettingsType,
    agents: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<BTreeMap<&'a str, String>> {
    sharing_on(settings).then(|| {
        agents
            .into_iter()
            .filter_map(|(name, block_id)| Some((name, agent_line_of_block(mstore, block_id)?)))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(pairs: &[(&str, &str)]) -> MetaMapType {
        pairs.iter().map(|(k, v)| (k.to_string(), serde_json::json!(v))).collect()
    }

    #[test]
    fn the_generated_title_wins_over_the_osc_title() {
        let m = meta(&[
            (crate::ambient::title::META_TITLE, "Fixing the login race"),
            (META_OSC_TITLE, "claude"),
        ]);
        assert_eq!(line_from_meta(&m).as_deref(), Some("Fixing the login race"));
    }

    #[test]
    fn an_unusable_title_falls_back_to_the_osc_title_and_then_to_none() {
        let m = meta(&[(crate::ambient::title::META_TITLE, "(none yet)"), (META_OSC_TITLE, " Build ")]);
        assert_eq!(line_from_meta(&m).as_deref(), Some("Build"));
        assert_eq!(line_from_meta(&meta(&[(crate::ambient::title::META_TITLE, "none")])), None);
        assert_eq!(line_from_meta(&MetaMapType::default()), None);
    }

    #[test]
    fn a_line_is_one_bounded_line() {
        assert_eq!(clean_line("  two\nlines\t\u{7}here ").as_deref(), Some("two lines here"));
        assert_eq!(clean_line(" \n\t "), None);
        let long = "é".repeat(MAX_LINE_CHARS + 50);
        assert_eq!(clean_line(&long).unwrap().chars().count(), MAX_LINE_CHARS);
    }

    #[test]
    fn sharing_is_on_unless_turned_off() {
        let read = |v: serde_json::Value| sharing_on(&serde_json::from_value(v).unwrap());
        assert!(read(serde_json::json!({})));
        assert!(!read(serde_json::json!({ "swarm:sharesummaries": false })));
        assert!(read(serde_json::json!({ "swarm:sharesummaries": "no" })), "not a boolean: the default");
    }
}
