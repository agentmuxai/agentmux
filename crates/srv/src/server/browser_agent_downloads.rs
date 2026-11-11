// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a download in an agent's own browser pane may be saved without a dialog
//! (docs/specs/SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md §7.5, D3b). The host decides inside a CEF callback
//! that can't wait for srv, so srv pushes it each pane's workspace (`browser_host_sync`), only for a pane an
//! agent owns and that the person hasn't taken over or been handed, and that does not browse as the
//! person (a pane in the Personal profile keeps the dialog: it has the person's sign-ins, and what it
//! downloads is the person's to place). The host adds `downloads/` itself and checks the folder before
//! writing. Each download, saved or refused, is a row in the Audit view (`browser_dialog`).

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::server::AppState;

/// The setting that turns it off: the person is asked in the dialog as before. On unless false.
pub(crate) const SETTING: &str = "browser:downloadagentsave";

pub(crate) fn enabled(extra: &HashMap<String, Value>) -> bool {
    extra.get(SETTING).and_then(|v| v.as_bool()).unwrap_or(true)
}

/// The panes that may save into a workspace: owned by an agent that has one, and not handed to the person.
/// `owners` is (pane, workspace), the workspace empty when the agent has none on record.
fn workdirs_for(owners: &[(String, String)], handed: &[String], on: bool) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    if !on {
        return out;
    }
    for (pane, workdir) in owners {
        if workdir.is_empty() || handed.contains(pane) {
            continue;
        }
        out.insert(pane.clone(), json!(workdir));
    }
    out
}

/// Whether the pane with `meta` browses as the person (no identity of its own): the Personal profile.
fn browses_as_the_person(meta: &crate::backend::obj::MetaMapType) -> bool {
    crate::server::browser_identity::browsing_as(meta) == "personal"
}

/// What the host is sent: pane -> its owner's workspace on this computer (a container agent's `/workspace` is
/// bound from the same folder).
pub(crate) fn workdirs(state: &AppState, owned: &[String], handed: &[String]) -> Value {
    let on = enabled(&state.config_watcher.get_full_config().settings.extra);
    // A pane we can't read, or one in the Personal profile, is left out like a handed pane: the dialog stays.
    let handed: Vec<String> = handed
        .iter()
        .cloned()
        .chain(owned.iter().filter(|pane| {
            state.mstore.get::<crate::backend::obj::Block>(pane).ok().flatten().map_or(true, |b| browses_as_the_person(&b.meta))
        }).cloned())
        .collect();
    let owners: Vec<(String, String)> = owned
        .iter()
        .map(|pane| {
            let workdir = crate::server::browser_owner::owner_of(pane)
                .and_then(|agent| crate::backend::agent_resolve::resolve_agent_id(&state.mstore, &agent).ok())
                .and_then(|id| state.mstore.agent_def_get(&id).ok().flatten())
                .map(|def| crate::backend::base::expand_home_dir_safe(&def.working_directory).to_string_lossy().to_string())
                .filter(|dir| !dir.is_empty())
                .unwrap_or_default();
            (pane.clone(), workdir)
        })
        .collect();
    Value::Object(workdirs_for(&owners, &handed, on))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owners() -> Vec<(String, String)> {
        vec![
            ("p1".into(), "/w/a".into()),
            ("p2".into(), "/w/b".into()),
            ("p3".into(), String::new()),
        ]
    }

    #[test]
    fn only_a_pane_with_a_workspace_that_is_not_handed_over_may_save_without_asking() {
        let m = workdirs_for(&owners(), &["p2".to_string()], true);
        assert_eq!(m.len(), 1);
        assert_eq!(m["p1"], "/w/a");
    }

    #[test]
    fn a_pane_in_the_personal_profile_is_the_person_s_and_a_pane_with_its_own_jar_is_not() {
        let meta = |identity: Option<&str>| {
            let mut m = crate::backend::obj::MetaMapType::new();
            if let Some(i) = identity {
                m.insert(crate::server::browser_identity::IDENTITY_META_KEY.to_string(), json!(i));
            }
            m
        };
        assert!(browses_as_the_person(&meta(None)));
        assert!(!browses_as_the_person(&meta(Some("incognito:0123456789abcdef"))));
        assert!(!browses_as_the_person(&meta(Some("profile:agent-posa"))));
    }

    #[test]
    fn the_setting_turns_it_off_and_is_on_by_default() {
        assert!(workdirs_for(&owners(), &[], false).is_empty());
        let mut extra = HashMap::new();
        assert!(enabled(&extra));
        extra.insert(SETTING.to_string(), json!(false));
        assert!(!enabled(&extra));
        extra.insert(SETTING.to_string(), json!(true));
        assert!(enabled(&extra));
    }
}
