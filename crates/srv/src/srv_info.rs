// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What srv tells a UI about the machine it runs on: sent as the `srvinfo`
//! event on every WebSocket connect (`server/websocket.rs`). A UI shows these
//! names and builds paths from the home directory, and the paths have to be
//! srv's, which is only the same machine as the UI's when the UI is the
//! desktop app.

use std::sync::OnceLock;

/// The account-wide AgentMux root (`~/.agentmux`, or its override) as it was
/// when srv started. Read before startup sets `AGENTMUX_DATA_HOME` to the data
/// directory (`bootstrap/stores.rs`), after which the same resolver answers
/// with the data directory instead.
static HOME_DIR: OnceLock<Option<String>> = OnceLock::new();

/// Record the AgentMux root. Call first thing in `main`, before anything
/// changes the environment.
pub fn capture_home_dir() {
    let _ = HOME_DIR.set(
        agentmux_common::data_paths::agentmux_root()
            .ok()
            .map(|p| p.to_string_lossy().into_owned()),
    );
}

/// The account-wide AgentMux root captured at startup (`capture_home_dir`).
pub fn agentmux_home() -> Option<std::path::PathBuf> {
    HOME_DIR.get().cloned().flatten().map(std::path::PathBuf::from)
}

/// The `srvinfo` event: srv's version, and the machine it runs on.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct SrvInfo {
    pub version: String,
    /// srv's OS, as `process.platform` names it (`darwin`, `win32`, `linux`).
    pub platform: String,
    pub user_name: String,
    pub host_name: String,
    /// The account-wide AgentMux root on srv's machine (`~/.agentmux`, or its override).
    pub home_dir: Option<String>,
}

/// The `data` of the `srvinfo` event.
pub fn srv_info(version: &str, host_name: &str) -> SrvInfo {
    SrvInfo {
        version: version.to_string(),
        platform: agentmux_common::platform_name::platform_name().to_string(),
        user_name: whoami::username(),
        host_name: host_name.to_string(),
        home_dir: HOME_DIR.get().cloned().flatten(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srv_info_names_the_machine_and_the_version() {
        let info = serde_json::to_value(srv_info("1.2.3", "box")).unwrap();
        assert_eq!(info["version"], "1.2.3");
        assert_eq!(info["hostName"], "box");
        assert_eq!(
            info["platform"],
            agentmux_common::platform_name::platform_name()
        );
        assert_eq!(info["userName"], whoami::username());
        assert!(info.get("homeDir").is_some());
    }
}
