// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a session title is stored, and when a new one replaces the old.
//!
//! Both writers go through [`store_title`]: the pane's own request
//! (`session:activity_summary`, on a human message) and the empty-title recovery
//! sweep (`backend::reactive::activity_watcher`). The check and the write happen
//! in one transaction, so neither can overwrite a title the other stored while
//! its call ran. The pane used to write the title itself, after its own copy of
//! these rules. docs/reports/REPORT_AMBIENT_FRAMEWORK_REASSESSMENT_2026_10_08.md
//! section 6.7.

use std::collections::HashSet;

use crate::ambient::validate::is_usable_title;
use crate::backend::obj::{self, Block};
use crate::backend::storage::store::Store;

/// The block meta key the title lives in. The pane header and the Swarm row read it.
pub const META_TITLE: &str = "term:ambient_summary";

/// When the title was stored, or last confirmed as still right (epoch ms), and the
/// session's human turn it was judged at. The pane reads them to age a title
/// (`frontend/app/store/title-schedule.ts`); `0` there means "re-evaluate at the next
/// message" (set after a compaction). Written here and nowhere else on the backend.
pub const META_TITLE_AT: &str = "term:ambient_summary_at";
pub const META_TITLE_TURN: &str = "term:ambient_summary_turn";

/// The pane's count of this session's human messages (`title-schedule.ts`), the
/// turn a title is stamped with when the caller does not say.
const META_HUMAN_TURNS: &str = "term:human_turns";

/// When a candidate title is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replace {
    /// Only where there is no usable title: the recovery sweep, whose source (the
    /// activity alone) is weaker than the pane's.
    IfEmpty,
    /// Where there is none, or where the candidate names a different goal
    /// ([`is_title_news`]). A rewording keeps the old title, so it stays stable.
    IfNews,
}

/// Store `title` as the block's session title if `replace` allows it. A value
/// that is not a usable title is never stored. `still_current` is asked inside
/// the transaction: a request superseded by a newer one must not store its
/// title after the newer one did. `Ok(true)` when written.
pub fn store_title(
    store: &Store,
    block_id: &str,
    title: &str,
    replace: Replace,
    turn: Option<u32>,
    still_current: impl Fn() -> bool,
) -> Result<bool, String> {
    store_title_at(store, block_id, title, replace, turn, now_ms(), still_current)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Stamp the title as judged `now` at `turn` (the caller's, else the session's
/// human-turn count in the block's meta, else none).
fn stamp(block: &mut Block, turn: Option<u32>, now: i64) {
    let turn = turn.map(i64::from).or_else(|| block.meta.get(META_HUMAN_TURNS).and_then(serde_json::Value::as_i64));
    block.meta.insert(META_TITLE_AT.to_string(), serde_json::Value::from(now));
    match turn {
        Some(t) => block.meta.insert(META_TITLE_TURN.to_string(), serde_json::Value::from(t)),
        None => block.meta.remove(META_TITLE_TURN),
    };
}

fn store_title_at(
    store: &Store,
    block_id: &str,
    title: &str,
    replace: Replace,
    turn: Option<u32>,
    now: i64,
    still_current: impl Fn() -> bool,
) -> Result<bool, String> {
    if !is_usable_title(title) {
        return Ok(false);
    }
    store
        .with_tx(|tx| {
            if !still_current() {
                return Ok(false);
            }
            let mut block = tx.must_get::<Block>(block_id)?;
            let current = obj::meta_get_string(&block.meta, META_TITLE, "");
            if is_usable_title(&current) {
                let keep = match replace {
                    Replace::IfEmpty => true,
                    Replace::IfNews => !is_title_news(&current, title),
                };
                if keep {
                    return Ok(false);
                }
            }
            block.meta.insert(META_TITLE.to_string(), serde_json::Value::String(title.to_string()));
            stamp(&mut block, turn, now);
            tx.update(&mut block)?;
            Ok(true)
        })
        .map_err(|e| e.to_string())
}

/// The title was re-evaluated from scratch (`refresh`, no current title shown to the
/// model) and `candidate` is a rewording of it: keep the title, and say it was judged
/// still right now, so it is not asked about again at every message. `Ok(true)` when
/// the stamp was written. Nothing happens for a candidate that is not a usable title
/// (the model abstained: the title stays aged and is tried again), for a title that
/// is not there or is not the same goal, or for a superseded request.
pub fn confirm_title(
    store: &Store,
    block_id: &str,
    candidate: &str,
    turn: Option<u32>,
    still_current: impl Fn() -> bool,
) -> Result<bool, String> {
    confirm_title_at(store, block_id, candidate, turn, now_ms(), still_current)
}

fn confirm_title_at(
    store: &Store,
    block_id: &str,
    candidate: &str,
    turn: Option<u32>,
    now: i64,
    still_current: impl Fn() -> bool,
) -> Result<bool, String> {
    if !is_usable_title(candidate) {
        return Ok(false);
    }
    store
        .with_tx(|tx| {
            if !still_current() {
                return Ok(false);
            }
            let mut block = tx.must_get::<Block>(block_id)?;
            let current = obj::meta_get_string(&block.meta, META_TITLE, "");
            if !is_usable_title(&current) || is_title_news(&current, candidate) {
                return Ok(false);
            }
            stamp(&mut block, turn, now);
            tx.update(&mut block)?;
            Ok(true)
        })
        .map_err(|e| e.to_string())
}

/// Words that carry no topic, ignored when comparing two titles.
const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "of", "to", "for", "in", "on", "with", "by", "at", "from", "into", "its", "it",
];

/// Below this overlap (Jaccard over topic words) a new title counts as news.
const NEWS_MAX_OVERLAP: f64 = 0.5;

fn topic_words(title: &str) -> HashSet<String> {
    title
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty() && !STOP_WORDS.contains(w))
        .map(str::to_string)
        .collect()
}

/// Is `candidate` a different goal from `current`, or a rewording of it? A
/// rewording ("Fix the login race" → "Fix login race condition") keeps the old
/// title; a real change of topic replaces it.
pub fn is_title_news(current: &str, candidate: &str) -> bool {
    let a = topic_words(current);
    let b = topic_words(candidate);
    if a.is_empty() || b.is_empty() {
        return a.len() != b.len();
    }
    let shared = a.intersection(&b).count();
    let overlap = shared as f64 / (a.len() + b.len() - shared) as f64;
    overlap < NEWS_MAX_OVERLAP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rewording_is_not_news() {
        for (current, candidate) in [
            ("Fix the login race", "Fix login race condition"),
            ("Fix the login race", "fix the LOGIN race"),
            ("Harden the swarm ambient summary", "Harden swarm summary"),
        ] {
            assert!(!is_title_news(current, candidate), "{current:?} -> {candidate:?}");
        }
    }

    #[test]
    fn a_new_topic_is_news() {
        for (current, candidate) in [
            ("Fix the login race", "Set up CI for the docs site"),
            ("Harden the swarm ambient summary", "Add UDP discovery for LAN peers"),
            ("Review PR 4230", "Write the multi-host swarm spec"),
        ] {
            assert!(is_title_news(current, candidate), "{current:?} -> {candidate:?}");
        }
    }

    fn store_with_block(meta: &[(&str, &str)]) -> Store {
        let store = Store::open_in_memory().unwrap();
        let mut block = Block {
            oid: "b1".to_string(),
            parentoref: String::new(),
            version: 0,
            runtimeopts: None,
            stickers: None,
            meta: obj::MetaMapType::new(),
            subblockids: None,
        };
        for (k, v) in meta {
            block.meta.insert(k.to_string(), serde_json::Value::String(v.to_string()));
        }
        store.insert(&mut block).unwrap();
        store
    }

    fn title_of(store: &Store) -> String {
        obj::meta_get_string(&store.must_get::<Block>("b1").unwrap().meta, META_TITLE, "")
    }

    #[test]
    fn either_writer_fills_an_empty_or_placeholder_title() {
        for replace in [Replace::IfEmpty, Replace::IfNews] {
            for before in [&[][..], &[(META_TITLE, "(none yet)")][..]] {
                let store = store_with_block(before);
                assert_eq!(store_title(&store, "b1", "Fix the login race", replace, None, || true), Ok(true));
                assert_eq!(title_of(&store), "Fix the login race");
            }
        }
    }

    #[test]
    fn recovery_never_replaces_a_title() {
        let store = store_with_block(&[(META_TITLE, "Set up CI for the docs site")]);
        assert_eq!(store_title(&store, "b1", "Something else entirely", Replace::IfEmpty, None, || true), Ok(false));
        assert_eq!(title_of(&store), "Set up CI for the docs site");
    }

    #[test]
    fn the_pane_replaces_a_title_only_with_news() {
        let store = store_with_block(&[(META_TITLE, "Fix the login race")]);
        assert_eq!(store_title(&store, "b1", "Fix login race condition", Replace::IfNews, None, || true), Ok(false));
        assert_eq!(title_of(&store), "Fix the login race");
        assert_eq!(store_title(&store, "b1", "Set up CI for the docs site", Replace::IfNews, None, || true), Ok(true));
        assert_eq!(title_of(&store), "Set up CI for the docs site");
    }

    #[test]
    fn a_superseded_request_stores_nothing() {
        let store = store_with_block(&[(META_TITLE, "Set up CI for the docs site")]);
        assert_eq!(store_title(&store, "b1", "Fix the login race", Replace::IfNews, None, || false), Ok(false));
        assert_eq!(title_of(&store), "Set up CI for the docs site");
    }

    #[test]
    fn a_placeholder_is_never_stored() {
        let store = store_with_block(&[]);
        assert_eq!(store_title(&store, "b1", "(none yet)", Replace::IfNews, None, || true), Ok(false));
        assert_eq!(title_of(&store), "");
    }

    #[test]
    fn other_meta_survives_the_write() {
        let store = store_with_block(&[("agentName", "AgentX")]);
        store_title(&store, "b1", "Fix the login race", Replace::IfEmpty, None, || true).unwrap();
        let meta = store.must_get::<Block>("b1").unwrap().meta;
        assert_eq!(obj::meta_get_string(&meta, "agentName", ""), "AgentX");
    }
    fn number_of(store: &Store, key: &str) -> Option<i64> {
        store.must_get::<Block>("b1").unwrap().meta.get(key).and_then(serde_json::Value::as_i64)
    }

    fn set_number(store: &Store, key: &str, value: i64) {
        let mut block = store.must_get::<Block>("b1").unwrap();
        block.meta.insert(key.to_string(), serde_json::Value::from(value));
        store.update(&mut block).unwrap();
    }

    #[test]
    fn a_stored_title_is_stamped_with_its_time_and_the_callers_turn() {
        let store = store_with_block(&[]);
        assert_eq!(store_title_at(&store, "b1", "Fix the login race", Replace::IfNews, Some(7), 1_000, || true), Ok(true));
        assert_eq!(number_of(&store, META_TITLE_AT), Some(1_000));
        assert_eq!(number_of(&store, META_TITLE_TURN), Some(7));
    }

    #[test]
    fn without_a_turn_the_stamp_takes_the_sessions_human_turn_count() {
        let store = store_with_block(&[]);
        set_number(&store, META_HUMAN_TURNS, 4);
        store_title_at(&store, "b1", "Fix the login race", Replace::IfEmpty, None, 1_000, || true).unwrap();
        assert_eq!(number_of(&store, META_TITLE_TURN), Some(4));
    }

    #[test]
    fn a_title_that_is_kept_is_not_restamped_by_a_store() {
        let store = store_with_block(&[(META_TITLE, "Fix the login race")]);
        set_number(&store, META_TITLE_AT, 5);
        assert_eq!(store_title_at(&store, "b1", "Fix login race condition", Replace::IfNews, Some(9), 1_000, || true), Ok(false));
        assert_eq!(number_of(&store, META_TITLE_AT), Some(5));
    }

    #[test]
    fn a_rewording_found_on_a_refresh_confirms_the_title_and_restamps_it() {
        let store = store_with_block(&[(META_TITLE, "Fix the login race")]);
        set_number(&store, META_TITLE_AT, 0);
        assert_eq!(confirm_title_at(&store, "b1", "Fix login race condition", Some(12), 9_000, || true), Ok(true));
        assert_eq!(title_of(&store), "Fix the login race");
        assert_eq!(number_of(&store, META_TITLE_AT), Some(9_000));
        assert_eq!(number_of(&store, META_TITLE_TURN), Some(12));
    }

    #[test]
    fn a_new_goal_or_an_abstention_does_not_confirm_the_old_title() {
        let store = store_with_block(&[(META_TITLE, "Fix the login race")]);
        set_number(&store, META_TITLE_AT, 0);
        assert_eq!(confirm_title_at(&store, "b1", "Set up CI for the docs site", Some(12), 9_000, || true), Ok(false));
        assert_eq!(confirm_title_at(&store, "b1", "(none yet)", Some(12), 9_000, || true), Ok(false));
        assert_eq!(confirm_title_at(&store, "b1", "Fix login race condition", Some(12), 9_000, || false), Ok(false));
        assert_eq!(number_of(&store, META_TITLE_AT), Some(0));
    }
}
