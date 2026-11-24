// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Browser open` on a site the agent already has a pane on: the answer says so (the ids), so the agent can use that pane
//! (`navigate`) instead of opening a second one. Nothing is refused: a second pane is sometimes what is wanted (another
//! profile, a side-by-side look), so this only tells.

use agentmux_common::popup_rules::origin_of;
use crate::backend::obj::Block;

/// The panes in `panes` (id and the address the pane was opened or last moved to) whose site is `url`'s: the same scheme, host
/// and port. Empty for an address that is not http(s).
pub(crate) fn same_site(panes: &[(String, String)], url: &str) -> Vec<String> {
    let wanted = origin_of(url);
    if wanted.is_empty() {
        return Vec::new();
    }
    panes.iter().filter(|(_, at)| origin_of(at) == wanted).map(|(id, _)| id.clone()).collect()
}

/// The ids of the browser panes `agent` owns that are on `url`'s site now.
pub(crate) async fn owned_on_site(state: &crate::server::AppState, agent: &str, url: &str) -> Vec<String> {
    let owned: Vec<String> = crate::server::browser_owner::all_owners().into_iter().filter(|(_, who)| who == agent).map(|(id, _)| id).collect();
    if owned.is_empty() {
        return Vec::new();
    }
    let panes: Vec<(String, String)> = owned
        .iter()
        .filter_map(|id| {
            let block = state.mstore.get::<Block>(id).ok().flatten()?;
            let at = block.meta.get("url").and_then(|v| v.as_str())?.to_string();
            Some((id.clone(), at))
        })
        .collect();
    same_site(&panes, url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panes() -> Vec<(String, String)> {
        vec![
            ("b1".into(), "https://app.example.com/inbox".into()),
            ("b2".into(), "https://app.example.com:8443/inbox".into()),
            ("b3".into(), "http://app.example.com/".into()),
            ("b4".into(), "https://other.example.org/".into()),
        ]
    }

    #[test]
    fn a_pane_on_the_same_scheme_host_and_port_is_the_same_site() {
        assert_eq!(same_site(&panes(), "https://app.example.com/settings?tab=1"), vec!["b1".to_string()]);
    }

    #[test]
    fn another_port_scheme_or_host_is_another_site() {
        assert_eq!(same_site(&panes(), "https://app.example.com:8443/x"), vec!["b2".to_string()]);
        assert_eq!(same_site(&panes(), "http://app.example.com/x"), vec!["b3".to_string()]);
        assert!(same_site(&panes(), "https://elsewhere.example.net/").is_empty());
    }

    #[test]
    fn an_address_that_is_not_a_web_address_matches_nothing() {
        assert!(same_site(&panes(), "file:///etc/hosts").is_empty());
        assert!(same_site(&panes(), "").is_empty());
        assert!(same_site(&[("b9".into(), "about:blank".into())], "https://app.example.com/").is_empty());
    }
}
