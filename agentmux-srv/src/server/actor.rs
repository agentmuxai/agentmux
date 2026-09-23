// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether the actor a request names is the agent that sent it — identity
//! M4a-2 (`docs/specs/SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md`
//! §6.5.3, §6.5.6).
//!
//! An actor site is a handler whose request says who is acting: a memory
//! owner, a global-memory writer, a work claimer or creator, an inject or bus
//! sender, a cron creator, a UI-automation signer (§6.5.2 item 4). When the
//! request is [`Caller::Agent`], the named actor is checked against that
//! UID's own row, and one of these is counted per site:
//!
//! - `m4.actor_mismatch.<site>` — the name is none of the row's {slug,
//!   display name, `instance_name`}, matched as the M3 resolver matches
//!   (`Store::agents_matching_name`): slug exactly, the others ASCII
//!   case-insensitively. A name that would not select the caller's row.
//! - `m4.actor_ambiguous.<site>` — the name selects the caller's row, but not
//!   by its slug, and also selects another row: the colliding-name case, in
//!   which a slug-keyed consumer (memory) resolves to the *other* agent.
//! - `m4.actor_absent.<site>` — an attributed request that names no actor.
//! - `m4.actor_unchecked.<site>` — the caller's row could not be read.
//!
//! **Never refused, never awaited, and nothing is written:** the check runs
//! detached, off the async workers, so no response waits on it (§7); M4c
//! dual-writes the UID. This only measures how often the name and the token
//! disagree before anything relies on the token. Unattributed requests are not
//! checked — there is no row to compare with, and most are the UI (§6.5.1).

use crate::backend::storage::agents::AgentNameMatch;
use crate::backend::storage::error::StoreError;
use crate::backend::storage::store::Store;

use super::caller::Caller;
use super::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Mismatch,
    Ambiguous,
    Absent,
    Unchecked,
}

macro_rules! actor_sites {
    ($($site:ident => $name:literal,)*) => {
        /// Where the actor name came from — the `<site>` in the counters.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum ActorSite {
            $($site,)*
        }

        impl ActorSite {
            pub(crate) fn counter(self, outcome: Outcome) -> &'static str {
                match (self, outcome) {
                    $(
                        (ActorSite::$site, Outcome::Mismatch) => concat!("m4.actor_mismatch.", $name),
                        (ActorSite::$site, Outcome::Ambiguous) => concat!("m4.actor_ambiguous.", $name),
                        (ActorSite::$site, Outcome::Absent) => concat!("m4.actor_absent.", $name),
                        (ActorSite::$site, Outcome::Unchecked) => concat!("m4.actor_unchecked.", $name),
                    )*
                }
            }
        }
    };
}

actor_sites! {
    MemoryList => "memory_list",
    MemoryRead => "memory_read",
    MemoryWrite => "memory_write",
    MemoryHistory => "memory_history",
    MemoryDiff => "memory_diff",
    MemoryRevert => "memory_revert",
    GlobalMemoryWrite => "globalmemory_write",
    GlobalMemoryRevert => "globalmemory_revert",
    IdentityAccounts => "identity_accounts",
    IdentityValidate => "identity_validate",
    HistorySearch => "history_search",
    WorkEnqueue => "work_enqueue",
    WorkClaim => "work_claim",
    WorkHeartbeat => "work_heartbeat",
    WorkComplete => "work_complete",
    WorkRelease => "work_release",
    CronCreate => "cron_create",
    Inject => "inject",
    SupervisorDecision => "supervisor_decision",
    BusSend => "bus_send",
    BusInject => "bus_inject",
    BusBroadcast => "bus_broadcast",
    UiAuth => "ui_auth",
}

/// Whether `actor` would select `row` by name (see the module doc).
pub(crate) fn names_the_row(row: &AgentNameMatch, actor: &str) -> bool {
    let actor = actor.trim();
    !actor.is_empty()
        && (actor == row.slug
            || actor.eq_ignore_ascii_case(&row.name)
            || actor.eq_ignore_ascii_case(&row.instance_name))
}

/// What to count for `actor` against the caller's `row`; `None` when the
/// name is the caller's own. `rows_named` lists every row the name selects
/// (`Store::agents_matching_name`); it is asked only when the name selects
/// the caller's row other than by its slug.
pub(crate) fn classify(
    row: &AgentNameMatch,
    actor: &str,
    rows_named: impl FnOnce(&str) -> Result<Vec<AgentNameMatch>, StoreError>,
) -> Option<Outcome> {
    let actor = actor.trim();
    if actor == row.slug {
        return None;
    }
    if !names_the_row(row, actor) {
        return Some(Outcome::Mismatch);
    }
    match rows_named(actor) {
        Ok(rows) if rows.iter().any(|r| r.id != row.id) => Some(Outcome::Ambiguous),
        Ok(_) => None,
        Err(_) => Some(Outcome::Unchecked),
    }
}

/// Count `actor` against the caller's row at `site`. A no-op for an
/// Unattributed caller. Never fails, never delays the request: the store
/// reads run detached on the blocking pool (inline under test, so tests can
/// read the counters right after the request).
pub(crate) fn check_actor(
    state: &AppState,
    caller: Option<&Caller>,
    site: ActorSite,
    actor: Option<&str>,
) {
    let Some(uid) = caller.and_then(Caller::uid) else {
        return;
    };
    let Some(actor) = actor.map(str::trim).filter(|a| !a.is_empty()) else {
        crate::backend::agent_resolve::record_uid_fallback(site.counter(Outcome::Absent));
        return;
    };
    let (mstore, uid, actor) = (state.mstore.clone(), uid.to_string(), actor.to_string());
    let run = move || count_against_row(&mstore, &uid, site, &actor);
    #[cfg(not(test))]
    drop(tokio::task::spawn_blocking(run));
    #[cfg(test)]
    run();
}

fn count_against_row(store: &Store, uid: &str, site: ActorSite, actor: &str) {
    let outcome = match store.agent_names_by_id(uid) {
        Ok(Some(row)) => classify(&row, actor, |name| store.agents_matching_name(name)),
        Ok(None) | Err(_) => Some(Outcome::Unchecked),
    };
    let Some(outcome) = outcome else {
        return;
    };
    let counter = site.counter(outcome);
    crate::backend::agent_resolve::record_uid_fallback(counter);
    tracing::debug!(
        counter,
        caller_uid = uid,
        actor = %actor.chars().take(128).collect::<String>(),
        "identity M4a-2: the request's actor name is not plainly the calling agent's"
    );
}

/// A work claim also carries the claimer's UID (`agent_uid`, identity M1a).
/// A carried UID that is not the token's is counted on its own, since it is
/// the UID — not the name — that M3 writes to `claimed_by_uid`.
pub(crate) fn check_carried_uid(caller: Option<&Caller>, carried_uid: &str) {
    let Some(uid) = caller.and_then(Caller::uid) else {
        return;
    };
    let carried = carried_uid.trim();
    if !carried.is_empty() && carried != uid {
        crate::backend::agent_resolve::record_uid_fallback("m4.actor_uid_mismatch.work_claim");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, name: &str, slug: &str, instance_name: &str) -> AgentNameMatch {
        AgentNameMatch {
            id: id.into(),
            name: name.into(),
            slug: slug.into(),
            instance_name: instance_name.into(),
        }
    }

    fn only(
        rows: Vec<AgentNameMatch>,
    ) -> impl FnOnce(&str) -> Result<Vec<AgentNameMatch>, StoreError> {
        move |_| Ok(rows)
    }

    #[test]
    fn a_name_the_row_answers_to_matches() {
        let r = row("uid-y", "AgentY", "agenty", "Yardbird");
        assert!(names_the_row(&r, "agenty"));
        assert!(names_the_row(&r, " agenty "));
        assert!(
            names_the_row(&r, "AGENTY"),
            "display name is case-insensitive"
        );
        assert!(names_the_row(&r, "yardbird"));
        assert!(!names_the_row(&r, "agentz"));
        assert!(!names_the_row(&r, ""));
        // Slug is exact, as in the M3 resolver.
        let r = row("uid-y", "Why", "agenty", "");
        assert!(!names_the_row(&r, "AGENTY"));
        assert!(
            !names_the_row(&r, ""),
            "an empty instance name matches nothing"
        );
    }

    #[test]
    fn the_callers_own_slug_is_never_counted_and_asks_nothing() {
        let r = row("uid-y2", "AGENTY", "agenty-2", "");
        assert_eq!(classify(&r, "agenty-2", |_| panic!("not asked")), None);
    }

    #[test]
    fn a_name_that_is_not_the_callers_is_a_mismatch() {
        let r = row("uid-y2", "AGENTY", "agenty-2", "");
        assert_eq!(
            classify(&r, "agentz", |_| panic!("not asked")),
            Some(Outcome::Mismatch)
        );
    }

    /// The colliding-names fixture: the second agent ("AGENTY", agenty-2)
    /// sends the first one's slug `agenty`. It matches the second agent's
    /// display name, so it is no mismatch — but it selects the first agent
    /// too, and a slug-keyed consumer resolves it there (M4a-2 review P1).
    #[test]
    fn a_colliding_name_that_also_selects_another_row_is_ambiguous() {
        let first = row("uid-y", "AgentY", "agenty", "");
        let second = row("uid-y2", "AGENTY", "agenty-2", "");
        assert_eq!(
            classify(&second, "agenty", only(vec![first.clone(), second.clone()])),
            Some(Outcome::Ambiguous)
        );
        // The display name alone, selecting only the caller, is its own.
        assert_eq!(
            classify(&second, "AGENTY", only(vec![second.clone()])),
            None
        );
        assert_eq!(
            classify(&second, "agenty", |_| Err(StoreError::NotFound)),
            Some(Outcome::Unchecked)
        );
    }
}
