// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An entry's history: one row for every change to it and every use of it, in `db_vault_activity`
//! (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md §5).
//!
//! A row says who, what, where, under which version of the value, and why (the agent's own reason), and never
//! holds a value, a tail, a length or a hash of one. Rows are chained: each carries the hash of the one before,
//! and the newest row's place is also kept in the OS keychain, so a row edited, removed or slipped in by
//! someone with only the database is found. That is evidence for the person, not proof against someone who
//! holds the database and the keychain both.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::{expiry, vaults, VaultActivity, PERSON};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::{activity_hash, VaultActivityRow};
use crate::identity::secret_store;

/// A reason has at least this many characters, and at most [`MAX_REASON`], on one line.
pub const MIN_REASON: usize = 8;
pub const MAX_REASON: usize = 300;
/// An event has at most this many references, each at most [`MAX_REF`] characters.
pub const MAX_REFS: usize = 3;
pub const MAX_REF: usize = 120;
/// The most rows one history answers with.
pub const MAX_ROWS: i64 = 1000;

/// The keychain item holding the newest row's place in the chain.
const HEAD_ID: &str = "vault-history-head";
/// The head is written to the keychain at most this often (a trailing write follows a burst).
const HEAD_EVERY: Duration = Duration::from_secs(30);

/// What an event adds to its row besides what the row already says.
#[derive(Debug, Clone, Default)]
pub struct Why {
    pub reason: String,
    pub refs: Vec<String>,
}

/// The reason an agent gives for using or changing an entry, checked before anything is asked or written:
/// one line of [`MIN_REASON`] to [`MAX_REASON`] characters. The error is what to tell the agent.
pub fn require_reason(raw: Option<&str>) -> Result<String, String> {
    let line = one_line(raw.unwrap_or(""), usize::MAX);
    let n = line.chars().count();
    if n < MIN_REASON {
        return Err(format!(
            "a reason is required: say in one line what you need this for and why, for which task ({MIN_REASON} to {MAX_REASON} characters). The person sees it in the approval, and it is kept in the entry's history"
        ));
    }
    if n > MAX_REASON {
        return Err(format!("a reason is at most {MAX_REASON} characters, one line: yours is {n}"));
    }
    Ok(line)
}

/// `agent`, `person` (the pane) or `srv` (a sweep, an expiry reminder).
pub fn actor_kind(actor: &str) -> &'static str {
    if actor == PERSON {
        "person"
    } else if actor == expiry::ACTOR {
        "srv"
    } else {
        "agent"
    }
}

/// `text` on one line, trimmed and clipped to `max` characters.
pub fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(max).collect()
}

/// The references as the row keeps them: up to [`MAX_REFS`], each on one line, empty ones dropped.
pub fn refs_text(refs: &[String]) -> String {
    let kept: Vec<String> = refs.iter().map(|r| one_line(r, MAX_REF)).filter(|r| !r.is_empty()).take(MAX_REFS).collect();
    if kept.is_empty() {
        String::new()
    } else {
        serde_json::to_string(&kept).unwrap_or_default()
    }
}

/// The references of a row, as a list.
pub fn refs_of(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap_or_default()
}

/// This machine's name, as the rows say where an event happened. Empty when it can't be found.
fn host() -> &'static str {
    static HOST: OnceLock<String> = OnceLock::new();
    HOST.get_or_init(|| {
        let found = ["HOSTNAME", "COMPUTERNAME", "HOST"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .chain(std::fs::read_to_string("/etc/hostname").ok())
            .map(|h| h.trim().split('.').next().unwrap_or("").to_string())
            .find(|h| !h.is_empty());
        found.unwrap_or_default()
    })
}

/// Fills in what the history keeps on every row (who kind of actor, the machine, a clean reason and references),
/// appends it to the chain, and notes the chain's new head. A failure is logged, not raised: a use or a change
/// never fails for want of a history line.
pub fn append(store: &Store, mut row: VaultActivityRow) {
    row.actor_kind = actor_kind(&row.actor).into();
    row.host = host().into();
    row.reason = one_line(&row.reason, MAX_REASON);
    match store.vault_activity_add(&row) {
        Ok(stored) => note_head(&stored),
        Err(e) => tracing::warn!(error = %e, action = row.action, "vault: couldn't record activity"),
    }
}

/// An entry's history, newest first: every change and use since it was made. `agent` is the proven agent asking
/// (`None` for one that isn't, which sees what every agent may), or the person's pane (`person`, which may see all).
pub fn history_for(store: &Store, entry_id: &str, agent: Asker<'_>, limit: i64) -> Result<Vec<VaultActivity>, String> {
    let row = store.vault_get(entry_id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no entry {entry_id}"))?;
    if let Asker::Agent(agent) = agent {
        let allowed = store.vaults().map_err(|e| e.to_string())?.iter().find(|v| v.id == row.vault_id).is_none_or(|v| vaults::may_use(v, agent));
        if !allowed {
            // The same answer as for an id that isn't there.
            return Err(format!("no entry {entry_id}"));
        }
    }
    let rows = store.vault_activity_history(entry_id, row.created_at, limit.clamp(1, MAX_ROWS)).map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(VaultActivity::from).collect())
}

/// Who is asking for a history.
#[derive(Debug, Clone, Copy)]
pub enum Asker<'a> {
    /// The person, in the pane.
    Person,
    /// An agent: `Some(id)` when it proved who it is.
    Agent(Option<&'a str>),
}

/// Where the chain stands, for the pane.
#[derive(Debug, Clone, Serialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct VaultChain {
    /// No line was changed, removed or slipped in that the chain can tell.
    pub intact: bool,
    /// How many lines are chained and checked.
    pub checked: usize,
    /// How many older lines were made before the chain began, and so can't be checked.
    pub before_chain: usize,
    /// How many lines after the chain began have no place in it and look like what an older AgentMux sharing this
    /// computer's store writes (it doesn't know the chain). They are not checked, and are shown as a count.
    pub unchained_after: usize,
    /// The first line found wrong, when not intact.
    #[ts(type = "number | null")]
    pub broken_at: Option<i64>,
    /// Whether the newest line was compared with the place the keychain keeps for it. False when the keychain
    /// couldn't be read or holds none yet: lines removed from the end of the record would not be seen.
    pub head_checked: bool,
    /// What is wrong with it, in a sentence; empty when intact.
    pub problem: String,
}

/// The place in the chain the keychain holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    pub seq: i64,
    pub hash: String,
}

/// Whether a row with no hash is what a build that doesn't know the chain writes: none of the columns the chain
/// added. (Someone blanking them all by hand looks the same, which is why such rows are counted and shown.)
fn looks_unchained(r: &VaultActivityRow) -> bool {
    r.version == 0 && r.actor_kind.is_empty() && r.host.is_empty() && r.reason.is_empty() && r.refs.is_empty() && r.prev_hash.is_empty()
}

/// Walks `rows` (oldest first) and says whether the chain holds, and whether `head` is a row in it.
pub fn verify_chain(rows: &[VaultActivityRow], head: Option<&Head>) -> VaultChain {
    let mut out = VaultChain { intact: true, checked: 0, before_chain: 0, unchained_after: 0, broken_at: None, head_checked: head.is_some(), problem: String::new() };
    let mut prev = String::new();
    let mut broke = |out: &mut VaultChain, seq: i64, problem: &str| {
        out.intact = false;
        out.broken_at = Some(seq);
        out.problem = problem.into();
    };
    for row in rows {
        if row.hash.is_empty() {
            // Before the chain began, or after it by an older build still running on the same store. Such a row
            // has none of the chain's columns; one that has some but no place in the chain was taken out of it.
            if !looks_unchained(row) {
                broke(&mut out, row.seq, "this line was taken out of the chain");
                return out;
            }
            if out.checked > 0 {
                out.unchained_after += 1;
            } else {
                out.before_chain += 1;
            }
            continue;
        }
        if row.prev_hash != prev {
            broke(&mut out, row.seq, "the line before this one was removed or changed");
            return out;
        }
        if activity_hash(&prev, row) != row.hash {
            broke(&mut out, row.seq, "this line was changed");
            return out;
        }
        prev = row.hash.clone();
        out.checked += 1;
    }
    if let Some(head) = head {
        match rows.iter().find(|r| r.seq == head.seq) {
            None => broke(&mut out, head.seq, "the newest lines were removed"),
            Some(r) if r.hash != head.hash => broke(&mut out, head.seq, "this line is not the one the keychain recorded"),
            Some(_) => {}
        }
    }
    out
}

/// Where the chain stands now: the whole table walked, and the keychain's head compared.
pub fn check(store: &Store) -> Result<VaultChain, String> {
    let rows = store.vault_activity_all().map_err(|e| e.to_string())?;
    Ok(verify_chain(&rows, read_head().as_ref()))
}

fn read_head() -> Option<Head> {
    if cfg!(any(test, feature = "test-support")) {
        return None;
    }
    let stored = secret_store::get_optional(HEAD_ID).ok()??;
    let (seq, hash) = stored.split_once(':')?;
    Some(Head { seq: seq.parse().ok()?, hash: hash.to_string() })
}

struct HeadWrites {
    last: Option<Instant>,
    pending: Option<String>,
    trailing: bool,
}

static HEAD_WRITES: Mutex<HeadWrites> = Mutex::new(HeadWrites { last: None, pending: None, trailing: false });
static HEAD_FAILED_BEFORE: AtomicBool = AtomicBool::new(false);

fn write_head(value: &str) {
    if let Err(e) = secret_store::put(HEAD_ID, value) {
        // Said once: a keychain that can't be written stays so for every line after.
        if !HEAD_FAILED_BEFORE.swap(true, Ordering::Relaxed) {
            tracing::warn!(error = %e, "vault: couldn't keep the history's head in the keychain");
        }
    }
}

/// Keeps the new row's place in the keychain: at once, or at most every [`HEAD_EVERY`], with a write after a
/// burst so the head catches up with the last row.
fn note_head(row: &VaultActivityRow) {
    if cfg!(any(test, feature = "test-support")) {
        return;
    }
    let value = format!("{}:{}", row.seq, row.hash);
    let mut st = HEAD_WRITES.lock().unwrap();
    if st.last.is_none_or(|t| t.elapsed() >= HEAD_EVERY) {
        st.last = Some(Instant::now());
        st.pending = None;
        drop(st);
        write_head(&value);
        return;
    }
    st.pending = Some(value);
    if st.trailing {
        return;
    }
    st.trailing = true;
    let wait = HEAD_EVERY.saturating_sub(st.last.map_or(Duration::ZERO, |t| t.elapsed()));
    drop(st);
    std::thread::spawn(move || {
        std::thread::sleep(wait);
        let value = {
            let mut st = HEAD_WRITES.lock().unwrap();
            st.trailing = false;
            st.last = Some(Instant::now());
            st.pending.take()
        };
        if let Some(value) = value {
            write_head(&value);
        }
    });
}
