// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Whether a shell pane runs durable (SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md
//! §7.1): plain, durable because a setting says so, or durable by default.

use crate::backend::obj::{self, MetaMapType};

use super::durable_ssh::META_KEY_DURABLE;

/// How a shell pane runs (spec §7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Plain,
    /// Durable because the pane, its connection or Settings say so: without
    /// the helper the pane stops and says why.
    Chosen,
    /// Durable by default (nothing set): on a host without the helper,
    /// installing it asks first (`helper_consent`), and if it isn't installed
    /// the pane becomes a plain SSH terminal rather than stopping.
    Default,
}

/// Whether a shell pane runs durable ([`mode`]).
pub fn wants(meta: &MetaMapType, config: Option<&crate::backend::wconfig::FullConfigType>) -> bool {
    mode(meta, config) != Mode::Plain
}

/// How a shell pane runs. Durable only on an SSH connection: the pane's own
/// `term:durable` first, then its connection's settings, then the global
/// setting; with none of them set, durable by default, unless the helper
/// failed to get onto the host since srv started.
pub fn mode(meta: &MetaMapType, config: Option<&crate::backend::wconfig::FullConfigType>) -> Mode {
    let conn = obj::meta_get_string(meta, super::META_KEY_CONNECTION, "");
    if !matches!(
        crate::backend::remote::ConnTarget::parse(&conn),
        Ok(crate::backend::remote::ConnTarget::Ssh(_))
    ) {
        return Mode::Plain;
    }
    // An agent's SSH shell (`PtyShell`, marked by http_pty_shell.rs) is never
    // durable, whatever the defaults: its ssh's prompts must reach the user as
    // the agent's (its own askpass grant), and the agent tools do not reattach
    // a session yet (the `Shell` tool's `durable`, spec §8.1, still to come).
    if !obj::meta_get_string(
        meta,
        crate::backend::remote::askpass::META_KEY_AGENT_BLOCK,
        "",
    )
    .is_empty()
    {
        return Mode::Plain;
    }
    // The user doesn't want the helper on this host (Never, or "Not now"
    // since srv started) and it isn't there: a plain SSH terminal
    // (SPEC_REMOTES_PANE_2026_10_05.md §4.9).
    if crate::backend::remote::helper_consent::plain_instead(&conn)
        && !crate::backend::remote::helper_hosts::known(&conn)
    {
        return Mode::Plain;
    }
    // A host found to be one the helper can't run on (Windows) is plain even
    // when a stale record says it has the helper and couldn't be forgotten.
    if crate::backend::remote::helper_consent::unsupported_host(&conn) {
        return Mode::Plain;
    }
    let conn_setting = config.and_then(|c| {
        c.connections
            .iter()
            .find(|(name, _)| crate::backend::remote::conn::same_connection(name, &conn))
            .and_then(|(_, k)| k.term_durable)
    });
    resolve_durable(
        meta.get(META_KEY_DURABLE).and_then(|v| v.as_bool()),
        conn_setting,
        config.and_then(|c| c.settings.term_durable),
        || crate::backend::remote::helper_hosts::known(&conn),
        || crate::backend::remote::helper_consent::not_installed(&conn),
    )
}

/// §7.1's order: pane, connection, global, then whether the host has the
/// helper, or failed to get it since srv started (each asked only when
/// nothing is set).
pub fn resolve_durable(
    pane: Option<bool>,
    connection: Option<bool>,
    global: Option<bool>,
    host_has_helper: impl FnOnce() -> bool,
    helper_not_installed: impl FnOnce() -> bool,
) -> Mode {
    match pane.or(connection).or(global) {
        Some(true) => Mode::Chosen,
        Some(false) => Mode::Plain,
        None if host_has_helper() => Mode::Default,
        None if helper_not_installed() => Mode::Plain,
        None => Mode::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Not now" to the helper makes the host's durable panes plain SSH
    /// terminals until srv restarts; an Install puts them back
    /// (SPEC_REMOTES_PANE_2026_10_05.md §4.9).
    #[test]
    fn a_host_the_user_declined_the_helper_on_is_not_durable() {
        use crate::backend::remote::helper_consent::{install_answering_yes, note_answer, Answer};
        install_answering_yes();
        let mut meta = MetaMapType::new();
        meta.insert(super::super::META_KEY_CONNECTION.into(), serde_json::json!("declined-durable-host"));
        meta.insert(META_KEY_DURABLE.into(), serde_json::json!(true));
        assert!(wants(&meta, None));

        let answer = |approve| Answer { answered: true, approve, remember: false };
        note_answer("declined-durable-host", answer(false));
        assert!(!wants(&meta, None), "Not now: a plain SSH terminal");
        note_answer("declined-durable-host", answer(true));
        assert!(wants(&meta, None), "Install: durable again");
    }

    #[test]
    fn only_an_ssh_pane_with_durable_on_is_durable() {
        let meta = |conn: &str, durable: Option<bool>| {
            let mut m = MetaMapType::new();
            m.insert(
                super::super::META_KEY_CONNECTION.into(),
                serde_json::json!(conn),
            );
            if let Some(d) = durable {
                m.insert(META_KEY_DURABLE.into(), serde_json::json!(d));
            }
            m
        };
        assert!(wants(&meta("area54", Some(true)), None));
        assert!(!wants(&meta("area54", Some(false)), None));
        assert!(!wants(&meta("local", Some(true)), None));
        assert!(!wants(&meta("wsl://Ubuntu", Some(true)), None));

        // The connection's setting, then the global one, under the pane's.
        let mut config = crate::backend::wconfig::FullConfigType::default();
        config.settings.term_durable = Some(false);
        config.connections.insert(
            "durable-test-host".into(),
            crate::backend::wconfig::ConnKeywords {
                term_durable: Some(true),
                ..Default::default()
            },
        );
        assert!(wants(&meta("durable-test-host", None), Some(&config)));
        assert!(!wants(
            &meta("durable-test-host", Some(false)),
            Some(&config)
        ));
        assert!(!wants(&meta("durable-test-other", None), Some(&config)));

        // An agent's SSH shell: never, not even when asked or by default.
        let mut agent = meta("durable-test-host", Some(true));
        agent.insert(
            crate::backend::remote::askpass::META_KEY_AGENT_BLOCK.into(),
            serde_json::json!("agent-block"),
        );
        assert!(!wants(&agent, Some(&config)));
    }

    #[test]
    fn durable_is_the_pane_then_the_connection_then_global_then_the_helper() {
        let never = || panic!("asked the host only when nothing is set");
        assert_eq!(resolve_durable(Some(true), Some(false), Some(false), never, never), Mode::Chosen);
        assert_eq!(resolve_durable(None, Some(false), Some(true), never, never), Mode::Plain);
        assert_eq!(resolve_durable(None, None, Some(true), never, never), Mode::Chosen);
        assert_eq!(resolve_durable(None, None, None, || true, never), Mode::Default);
        // Nothing set and no helper yet: durable is offered (finding 7 of
        // #4513), unless the helper couldn't be put there since srv started.
        assert_eq!(resolve_durable(None, None, None, || false, || false), Mode::Default);
        assert_eq!(resolve_durable(None, None, None, || false, || true), Mode::Plain);
    }

    #[test]
    fn a_new_hosts_terminal_offers_durable_by_default() {
        let mut meta = MetaMapType::new();
        meta.insert(super::super::META_KEY_CONNECTION.into(), serde_json::json!("me@offered-host"));
        assert_eq!(mode(&meta, None), Mode::Default);
        assert!(wants(&meta, None));
    }
}
