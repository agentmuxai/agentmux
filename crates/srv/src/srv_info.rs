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

/// The `data` of the `srvinfo` event.
pub fn srv_info(version: &str, host_name: &str) -> serde_json::Value {
    serde_json::json!({
        "version": version,
        "platform": agentmux_common::platform_name::platform_name(),
        "userName": whoami::username(),
        "hostName": host_name,
        "homeDir": HOME_DIR.get().cloned().flatten(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srv_info_names_the_machine_and_the_version() {
        let info = srv_info("1.2.3", "box");
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
