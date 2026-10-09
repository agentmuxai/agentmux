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
                assert_eq!(store_title(&store, "b1", "Fix the login race", replace, || true), Ok(true));
                assert_eq!(title_of(&store), "Fix the login race");
            }
        }
    }

    #[test]
    fn recovery_never_replaces_a_title() {
        let store = store_with_block(&[(META_TITLE, "Set up CI for the docs site")]);
        assert_eq!(store_title(&store, "b1", "Something else entirely", Replace::IfEmpty, || true), Ok(false));
        assert_eq!(title_of(&store), "Set up CI for the docs site");
    }

    #[test]
    fn the_pane_replaces_a_title_only_with_news() {
        let store = store_with_block(&[(META_TITLE, "Fix the login race")]);
        assert_eq!(store_title(&store, "b1", "Fix login race condition", Replace::IfNews, || true), Ok(false));
        assert_eq!(title_of(&store), "Fix the login race");
        assert_eq!(store_title(&store, "b1", "Set up CI for the docs site", Replace::IfNews, || true), Ok(true));
        assert_eq!(title_of(&store), "Set up CI for the docs site");
    }

    #[test]
    fn a_superseded_request_stores_nothing() {
        let store = store_with_block(&[(META_TITLE, "Set up CI for the docs site")]);
        assert_eq!(store_title(&store, "b1", "Fix the login race", Replace::IfNews, || false), Ok(false));
        assert_eq!(title_of(&store), "Set up CI for the docs site");
    }

    #[test]
    fn a_placeholder_is_never_stored() {
        let store = store_with_block(&[]);
        assert_eq!(store_title(&store, "b1", "(none yet)", Replace::IfNews, || true), Ok(false));
        assert_eq!(title_of(&store), "");
    }

    #[test]
    fn other_meta_survives_the_write() {
        let store = store_with_block(&[("agentName", "AgentX")]);
        store_title(&store, "b1", "Fix the login race", Replace::IfEmpty, || true).unwrap();
        let meta = store.must_get::<Block>("b1").unwrap().meta;
        assert_eq!(obj::meta_get_string(&meta, "agentName", ""), "AgentX");
    }
}
