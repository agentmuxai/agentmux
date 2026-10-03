// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The account line of an agent in a list: the My Agents tile and the "Continue
//! an existing agent" menu (`SPEC_MY_AGENTS_TILES_AUTH_AND_HISTORY_2026_10_03.md` §5.2).
//!
//! An agent is either **bound** to an account that resolves, and the line is that
//! account's name, or it is not, and the line is [`NO_AUTH`]. The old
//! "(missing account)", "(ambient creds)" and "(unknown account)" are gone: a link
//! whose account is no longer there cannot launch the agent, so it is "No auth"
//! too, and a failed lookup claims nothing at all (an empty line).

use std::collections::HashMap;

use crate::backend::storage::store::{AgentIdentityLink, IdentityAccount};

pub(super) const NO_AUTH: &str = "No auth";

/// Account names by id. This channel's own accounts win; the global mirror fills
/// the rest, the order `identity::resolver::resolve_account` uses for display, so
/// an isolated channel whose links point at accounts only the mirror holds does
/// not read as unbound.
pub(super) fn names_by_id<'a>(
    own: &'a [IdentityAccount],
    mirror: &'a [IdentityAccount],
) -> HashMap<&'a str, &'a str> {
    let mut names: HashMap<&str, &str> = mirror.iter().map(|a| (a.id.as_str(), a.name.as_str())).collect();
    names.extend(own.iter().map(|a| (a.id.as_str(), a.name.as_str())));
    names
}

/// The text of the line, and how many of the agent's links pointed at an account
/// that is not there.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct AccountLine {
    pub text: String,
    pub unresolved: usize,
}

/// What could not be read, so a link that resolves to nothing may mean "the account
/// is gone" or just "the lookup failed".
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Degraded {
    /// The lookup of every agent's links failed: "no links" says nothing about this
    /// agent, so the line is left empty rather than "No auth".
    pub links: bool,
    /// The account lookup failed in this channel's store and the global mirror could
    /// not fill in: a link that resolves to nothing may be a perfectly good account,
    /// so it is left empty rather than reported as "No auth".
    pub accounts: bool,
}

pub(super) fn account_line(
    links: Option<&[&AgentIdentityLink]>,
    names: &HashMap<&str, &str>,
    degraded: Degraded,
) -> AccountLine {
    let links = links.unwrap_or(&[]);
    if links.is_empty() {
        let text = if degraded.links { String::new() } else { NO_AUTH.to_string() };
        return AccountLine { text, unresolved: 0 };
    }
    let mut resolved: Vec<String> = links.iter().filter_map(|l| names.get(l.account_id.as_str()).map(|n| n.to_string())).collect();
    let unresolved = links.len() - resolved.len();
    resolved.sort();
    resolved.dedup();
    let text = match (resolved.is_empty(), degraded.accounts) {
        (false, _) => resolved.join(", "),
        // Nothing resolved and the accounts could not be read: unknown, not unbound.
        (true, true) => String::new(),
        (true, false) => NO_AUTH.to_string(),
    };
    // Only a lookup that worked can say an account is gone.
    AccountLine { text, unresolved: if degraded.accounts { 0 } else { unresolved } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(id: &str, name: &str) -> IdentityAccount {
        IdentityAccount {
            id: id.into(),
            name: name.into(),
            provider: "claude".into(),
            kind: "oauth".into(),
            display_name: String::new(),
            secret_ref: crate::backend::storage::store::SecretRef::OAuthConfigDir { dir: "/x".into() },
            context: serde_json::json!({}),
            status: "valid".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    const OK: Degraded = Degraded { links: false, accounts: false };

    fn link(agent: &str, account: &str) -> AgentIdentityLink {
        AgentIdentityLink { agent_id: agent.into(), account_id: account.into(), provider: "claude".into() }
    }

    #[test]
    fn a_bound_agent_shows_its_accounts_sorted_and_deduplicated() {
        let own = [acct("a1", "work"), acct("a2", "home")];
        let names = names_by_id(&own, &[]);
        let ls = [link("ag", "a1"), link("ag", "a2"), link("ag", "a1")];
        let refs: Vec<&AgentIdentityLink> = ls.iter().collect();
        assert_eq!(account_line(Some(&refs), &names, OK), AccountLine { text: "home, work".into(), unresolved: 0 });
    }

    #[test]
    fn an_unbound_agent_says_no_auth() {
        let names = names_by_id(&[], &[]);
        assert_eq!(account_line(None, &names, OK).text, NO_AUTH);
        assert_eq!(account_line(Some(&[]), &names, OK).text, NO_AUTH);
    }

    #[test]
    fn a_link_to_an_account_that_is_gone_says_no_auth_and_is_counted() {
        let names = names_by_id(&[], &[]);
        let ls = [link("ag", "deleted")];
        let refs: Vec<&AgentIdentityLink> = ls.iter().collect();
        assert_eq!(account_line(Some(&refs), &names, OK), AccountLine { text: NO_AUTH.into(), unresolved: 1 });
    }

    #[test]
    fn a_partly_resolvable_agent_shows_the_accounts_it_can_launch_with() {
        let own = [acct("a1", "work")];
        let names = names_by_id(&own, &[]);
        let ls = [link("ag", "a1"), link("ag", "deleted")];
        let refs: Vec<&AgentIdentityLink> = ls.iter().collect();
        assert_eq!(account_line(Some(&refs), &names, OK), AccountLine { text: "work".into(), unresolved: 1 });
    }

    #[test]
    fn the_global_mirror_resolves_what_this_channel_does_not_and_this_channel_wins() {
        let own = [acct("a1", "own name")];
        let mirror = [acct("a1", "mirror name"), acct("a2", "only in mirror")];
        let names = names_by_id(&own, &mirror);
        let ls = [link("ag", "a1"), link("ag", "a2")];
        let refs: Vec<&AgentIdentityLink> = ls.iter().collect();
        assert_eq!(account_line(Some(&refs), &names, OK).text, "only in mirror, own name");
    }

    #[test]
    fn a_failed_links_lookup_claims_nothing() {
        let names = names_by_id(&[], &[]);
        let links_down = Degraded { links: true, accounts: false };
        assert_eq!(account_line(None, &names, links_down).text, "", "unknown is not 'No auth'");
        // ...but an agent that does have links still shows them
        let own = [acct("a1", "work")];
        let names = names_by_id(&own, &[]);
        let ls = [link("ag", "a1")];
        let refs: Vec<&AgentIdentityLink> = ls.iter().collect();
        assert_eq!(account_line(Some(&refs), &names, links_down).text, "work");
    }

    #[test]
    fn an_unreadable_accounts_store_makes_a_link_unknown_not_no_auth() {
        let names = names_by_id(&[], &[]);
        let ls = [link("ag", "perfectly-good")];
        let refs: Vec<&AgentIdentityLink> = ls.iter().collect();
        let accounts_down = Degraded { links: false, accounts: true };
        // the account may well exist: the lookup failed, so say nothing...
        assert_eq!(account_line(Some(&refs), &names, accounts_down), AccountLine { text: String::new(), unresolved: 0 });
        // ...and an agent with no links at all is still plainly unbound
        assert_eq!(account_line(None, &names, accounts_down).text, NO_AUTH);
        // ...and an account that DID resolve is still shown
        let own = [acct("perfectly-good", "work")];
        let names = names_by_id(&own, &[]);
        assert_eq!(account_line(Some(&refs), &names, accounts_down).text, "work");
    }

    #[test]
    fn the_old_labels_are_gone() {
        for old in ["(missing account)", "(ambient creds)", "(unknown account)"] {
            assert!(!NO_AUTH.contains(old));
        }
    }
}
