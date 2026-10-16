// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An agent asking to upgrade AgentMux: `RequestUpgrade`
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §5.1, phase 4).
//!
//! A restart stops every agent and interrupts the user, and an agent can be
//! prompt-injected, so an agent's word alone never starts one:
//! - the user's own turn asked for it (`self_quit::gate`, as for `QuitSelf`):
//!   the upgrade starts — the release is downloaded if it isn't yet, then
//!   AgentMux restarts once every agent has finished its turn;
//! - anything else: every window asks the user ("QTest asks to upgrade
//!   AgentMux to 0.59.27: <reason>", Upgrade now / Later), and the agent is
//!   told the answer as an `[AgentMux]` note.
//!
//! Either way the launcher still asks the user in its own dialog before it
//! restarts (`crates/launcher/src/upgrade_restart.rs`), and only a release
//! the update check found, verified against GitHub's checksum, is installed:
//! the agent can't name a file or a URL.

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backend::update_check::{self, AppUpdateStatus};

/// How long the flow waits for the download before giving up.
const DOWNLOAD_WAIT: Duration = Duration::from_secs(15 * 60);

/// An agent's request waiting for the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct UpgradeRequest {
    pub id: String,
    /// The agent that asked, by name.
    pub agent: String,
    pub reason: String,
    /// The release it would install.
    pub version: String,
    #[ts(type = "number")]
    pub asked_ms: u64,
    /// The asking agent's pane, to tell it the answer.
    #[serde(skip)]
    #[ts(skip)]
    pub block_id: String,
}

/// `app:upgraderequests`: the agents' upgrade requests waiting for the user.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct UpgradeRequestsEvent {
    pub requests: Vec<UpgradeRequest>,
}

/// The user's answer, relayed by the host (`POST /api/v1/host/upgrade_request`).
#[derive(Debug, Clone, Deserialize)]
pub struct UpgradeRequestAnswer {
    pub id: String,
    /// The release the prompt showed: Upgrade now installs only that one.
    pub version: String,
    /// Upgrade now (true) or Later (false).
    pub accept: bool,
}

/// What `RequestUpgrade` answers the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    /// The user's own turn asked: the upgrade is under way.
    Started { version: String },
    /// Every window asks the user; the agent hears the answer as a note.
    PendingUser { version: String, id: String },
    UpToDate { current: String },
    /// Nothing this AgentMux can install itself; `url` is where the user can.
    Unavailable { message: String, #[serde(skip_serializing_if = "Option::is_none")] url: Option<String> },
}

/// The release a request may install, or why there is none. `wanted` is the
/// version the agent named, if any: only the latest release can be installed.
pub fn installable(status: &AppUpdateStatus, wanted: Option<&str>) -> Result<String, Outcome> {
    match status.state.as_str() {
        "available" => {}
        "up-to-date" => return Err(Outcome::UpToDate { current: status.current.clone() }),
        "local-build" => {
            return Err(Outcome::Unavailable {
                message: "this is a local build, not a release; it doesn't upgrade itself".into(),
                url: status.url.clone(),
            })
        }
        "off" => {
            return Err(Outcome::Unavailable {
                message: "the update check is turned off (Settings: app:updatecheck)".into(),
                url: None,
            })
        }
        _ => {
            return Err(Outcome::Unavailable {
                message: "AgentMux hasn't learned of a newer release yet; try again in a few minutes".into(),
                url: None,
            })
        }
    }
    let latest = status.latest.clone().unwrap_or_default();
    // A restart into an earlier release is under way (the check found a
    // newer one since): that one goes first.
    if let Some(d) = status.download.as_ref().filter(|d| matches!(d.state.as_str(), "waiting" | "restarting")) {
        if d.version != latest {
            return Err(Outcome::Unavailable {
                message: format!("AgentMux is already restarting into {}; ask again once it has", d.version),
                url: None,
            });
        }
    }
    if let Some(w) = wanted.map(|w| w.trim().trim_start_matches('v')).filter(|w| !w.is_empty()) {
        if w != latest {
            return Err(Outcome::Unavailable {
                message: format!("only the latest release, {latest}, can be installed"),
                url: status.url.clone(),
            });
        }
    }
    if !status.can_download {
        return Err(Outcome::Unavailable {
            message: format!(
                "this AgentMux ({}) can't upgrade itself to {latest} yet; the user can download it from the release page",
                status.install
            ),
            url: status.url.clone(),
        });
    }
    Ok(latest)
}

static PENDING: Mutex<Vec<UpgradeRequest>> = Mutex::new(Vec::new());
static ASKING: Mutex<Option<crate::backend::user_attention::Asking>> = Mutex::new(None);

fn pending() -> std::sync::MutexGuard<'static, Vec<UpgradeRequest>> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner())
}

/// The requests waiting for the user (`app.upgrade-requests`).
pub fn list() -> Vec<UpgradeRequest> {
    pending().clone()
}

fn publish(state: &crate::server::AppState) {
    let requests = list();
    let mut asking = ASKING.lock().unwrap_or_else(|e| e.into_inner());
    *asking = requests.first().map(|r| {
        crate::backend::user_attention::Asking::start(
            &state.broker,
            &state.mstore,
            "",
            "upgrade",
            &format!("{} asks to upgrade AgentMux to {}", r.agent, r.version),
        )
    });
    drop(asking);
    state
        .broker
        .publish_event::<crate::backend::mps_events::AppUpgradeRequests>(&UpgradeRequestsEvent { requests }, vec![], 0);
}

/// Tell the agent in `block_id` something, as an `[AgentMux]` note.
fn tell(block_id: &str, text: &str) {
    let handler = crate::backend::reactive::get_global_handler();
    let Some(target) = handler
        .list_agents()
        .into_iter()
        .find(|r| r.block_id == block_id)
        .map(|r| r.uid.unwrap_or(r.agent_id))
    else {
        tracing::info!(block_id, "upgrade request: the asking agent isn't running to be told");
        return;
    };
    if let Err(e) = handler.deliver_system_note(&target, text) {
        tracing::info!(block_id, error = %e, "upgrade request: note not delivered");
    }
}

/// Ask the user on the agent's behalf. One request per agent: asking again
/// replaces its earlier one.
pub fn ask_user(state: &crate::server::AppState, block_id: &str, agent: &str, reason: &str, version: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    {
        let mut p = pending();
        p.retain(|r| r.block_id != block_id);
        p.push(UpgradeRequest {
            id: id.clone(),
            agent: agent.to_string(),
            reason: reason.chars().take(300).collect(),
            version: version.to_string(),
            asked_ms: agentmux_common::time::now_ms_u64(),
            block_id: block_id.to_string(),
        });
    }
    publish(state);
    id
}

/// The user's answer to request `id`, for the release the prompt showed
/// (relayed by the host: `POST /api/v1/host/upgrade_request`). Upgrade now
/// answers every request for that release, since there is one AgentMux to
/// upgrade; if the update check has found a newer release since, it installs
/// nothing and the agent is told to ask again.
pub fn answer(state: &crate::server::AppState, id: &str, version: &str, accept: bool) -> Result<(), String> {
    let latest = update_check::current_status().latest.unwrap_or_default();
    let (answered, stale): (Vec<UpgradeRequest>, bool) = {
        let mut p = pending();
        let Some(pos) = p.iter().position(|r| r.id == id && r.version == version) else {
            return Err("that upgrade request was already answered".into());
        };
        if accept && version == latest {
            let (same, other): (Vec<_>, Vec<_>) = std::mem::take(&mut *p).into_iter().partition(|r| r.version == version);
            *p = other;
            (same, false)
        } else {
            (vec![p.remove(pos)], accept)
        }
    };
    publish(state);
    if stale {
        for r in &answered {
            tell(
                &r.block_id,
                &format!(
                    "[AgentMux] AgentMux has learned of a newer release ({latest}) since you asked to upgrade to {}; nothing was installed. Ask again if it's still wanted.",
                    r.version
                ),
            );
        }
        return Err(format!("a newer release, {latest}, came out since this request; nothing was installed"));
    }
    for r in &answered {
        let text = if accept {
            format!(
                "[AgentMux] The user approved upgrading AgentMux to {}. It downloads now, then AgentMux asks the user once more and restarts after every agent finishes its turn; your session resumes afterwards.",
                r.version
            )
        } else {
            format!("[AgentMux] The user chose not to upgrade AgentMux to {} for now.", r.version)
        };
        tell(&r.block_id, &text);
    }
    if accept {
        start_upgrade(version.to_string(), answered.into_iter().map(|r| r.block_id).collect());
    }
    Ok(())
}

/// The user's own turn started the upgrade to `version`: requests for it
/// that were waiting are answered by that, and their agents told so.
pub fn resolve_started(state: &crate::server::AppState, version: &str) -> Vec<String> {
    let resolved: Vec<UpgradeRequest> = {
        let mut p = pending();
        let (same, other): (Vec<_>, Vec<_>) = std::mem::take(&mut *p).into_iter().partition(|r| r.version == version);
        *p = other;
        same
    };
    if resolved.is_empty() {
        return Vec::new();
    }
    publish(state);
    for r in &resolved {
        tell(
            &r.block_id,
            &format!("[AgentMux] The user asked for the upgrade to {version} themselves; it is under way. Your request is answered by that."),
        );
    }
    resolved.into_iter().map(|r| r.block_id).collect()
}

/// Download `version` if it isn't yet, then restart once every agent has
/// finished its turn (the launcher asks the user before it does). Only while
/// `version` is still the latest release: the download and restart take the
/// latest, so a newer one appearing meanwhile stops the flow. The agents in
/// `askers` (their panes) are told if it stops short, and why.
pub fn start_upgrade(version: String, askers: Vec<String>) {
    tokio::spawn(async move {
        let stop = |why: &str| {
            tracing::info!(version = %version, why, "upgrade request: stopped");
            for block_id in &askers {
                tell(block_id, &format!("[AgentMux] The upgrade to {version} stopped: {why}. Ask again if it's still wanted."));
            }
        };
        let still_latest = |s: &AppUpdateStatus| s.latest.as_deref() == Some(version.as_str());
        let ready = |s: &AppUpdateStatus| {
            s.download.as_ref().is_some_and(|d| d.version == version && d.state == "ready")
        };
        let busy = |s: &AppUpdateStatus| {
            s.download.as_ref().is_some_and(|d| matches!(d.state.as_str(), "waiting" | "restarting"))
        };
        let status = update_check::current_status();
        if let Some(d) = status.download.as_ref().filter(|_| busy(&status)) {
            // This release's restart is already under way; another's goes first.
            if d.version != version {
                stop(&format!("AgentMux is already restarting into {}", d.version));
            }
            return;
        }
        let newer = |s: &AppUpdateStatus| format!("a newer release, {}, came out", s.latest.clone().unwrap_or_default());
        if !still_latest(&status) {
            return stop(&newer(&status));
        }
        if !ready(&status) {
            if let Err(e) = crate::backend::update_download::start() {
                return stop(&format!("the download couldn't start ({e})"));
            }
            let deadline = tokio::time::Instant::now() + DOWNLOAD_WAIT;
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let s = update_check::current_status();
                if !still_latest(&s) {
                    return stop(&newer(&s));
                }
                if ready(&s) {
                    break;
                }
                if let Some(d) = s.download.as_ref().filter(|d| d.state == "failed") {
                    return stop(&format!("the download failed ({})", d.error.clone().unwrap_or_default()));
                }
                if s.download.is_none() || tokio::time::Instant::now() >= deadline {
                    return stop("the download didn't finish");
                }
            }
        }
        let s = update_check::current_status();
        if !still_latest(&s) {
            return stop(&newer(&s));
        }
        if let Err(e) = crate::backend::update_restart::start(false) {
            stop(&format!("the restart couldn't start ({e})"));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: &str, can_download: bool) -> AppUpdateStatus {
        AppUpdateStatus {
            state: state.into(),
            current: "0.59.26".into(),
            latest: Some("0.59.27".into()),
            url: Some("https://github.com/agentmuxai/agentmux/releases/tag/v0.59.27".into()),
            notes: None,
            published_at: None,
            checked_ms: 1,
            install: if can_download { "windows-portable" } else { "windows-setup" }.into(),
            asset: None,
            can_download,
            download: None,
        }
    }

    #[test]
    fn only_the_latest_release_an_install_can_take_is_installable() {
        assert_eq!(installable(&status("available", true), None), Ok("0.59.27".into()));
        assert_eq!(installable(&status("available", true), Some("v0.59.27")), Ok("0.59.27".into()));
        assert!(matches!(
            installable(&status("available", true), Some("0.59.25")),
            Err(Outcome::Unavailable { .. })
        ));
        assert_eq!(installable(&status("up-to-date", true), None), Err(Outcome::UpToDate { current: "0.59.26".into() }));
        // An install AgentMux can't upgrade itself gets the release page.
        let Err(Outcome::Unavailable { message, url }) = installable(&status("available", false), None) else {
            panic!("expected unavailable")
        };
        assert!(message.contains("windows-setup"), "{message}");
        assert!(url.unwrap().ends_with("v0.59.27"));
        for state in ["local-build", "off", "unknown"] {
            assert!(matches!(installable(&status(state, true), None), Err(Outcome::Unavailable { .. })), "{state}");
        }
        // A restart into an earlier release is under way: that one goes first.
        let mut busy = status("available", true);
        busy.download = Some(crate::backend::update_check::DownloadStatus {
            version: "0.59.26".into(),
            state: "restarting".into(),
            received: 1,
            total: 1,
            error: None,
            folder: None,
            installer: None,
            waiting_for: 0,
        });
        let Err(Outcome::Unavailable { message, .. }) = installable(&busy, None) else { panic!("expected unavailable") };
        assert!(message.contains("already restarting into 0.59.26"), "{message}");
    }

    #[test]
    fn outcomes_read_as_the_tool_expects() {
        let v = serde_json::to_value(Outcome::PendingUser { version: "0.59.27".into(), id: "r1".into() }).unwrap();
        assert_eq!(v, serde_json::json!({ "status": "pending_user", "version": "0.59.27", "id": "r1" }));
        let v = serde_json::to_value(Outcome::Unavailable { message: "m".into(), url: None }).unwrap();
        assert_eq!(v, serde_json::json!({ "status": "unavailable", "message": "m" }));
    }
}
