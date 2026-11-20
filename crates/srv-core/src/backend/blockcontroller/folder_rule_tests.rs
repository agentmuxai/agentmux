// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Each controller type in a folder a container agent can write (`mcp_folder_rule`): the ones that start the
//! agent's CLI themselves start no host agent there, whatever its harness, and the turn-based ones leave the
//! question to the turn.

use std::path::Path;

use serde_json::json;

use super::resync_steps::start_refusal;
use super::*;
use crate::backend::mcp_folder_rule::{turn_refusal, REFUSAL, TURN_REFUSAL};
use crate::backend::obj::MetaMapType;

const HARNESSES: [&str; 4] = ["claude", "codex", "gemini", "opencode"];

fn pane(mode: &str, provider: &str, cwd: &Path) -> MetaMapType {
    [("agentMode", json!(mode)), ("agentProvider", json!(provider)), ("cmd", json!("not-a-real-cli-of-this-test")), ("cmd:cwd", json!(cwd.to_string_lossy()))]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

/// A data root for the test, a folder bound into a container, and one no container agent has to do with.
fn folders(container: &str) -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
    let (data, bound, own) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    crate::backend::fs_ops::container_folders::add_for_test(bound.path(), container, "/workspace");
    (data, bound, own)
}

#[test]
fn no_controller_that_starts_the_cli_itself_is_created_for_a_host_agent_there() {
    let (data, bound, own) = folders("agentmux-controllers-a");
    crate::backend::mcp_config_file::with_data_home(data.path(), || {
        for controller in [BLOCK_CONTROLLER_SHELL, BLOCK_CONTROLLER_CMD, BLOCK_CONTROLLER_ACP, BLOCK_CONTROLLER_MSP, BLOCK_CONTROLLER_APP_SERVER] {
            for harness in HARNESSES {
                assert_eq!(start_refusal(controller, &pane("host", harness, bound.path())).as_deref(), Some(REFUSAL), "{controller} {harness}");
                assert_eq!(start_refusal(controller, &pane("host", harness, own.path())), None, "{controller} {harness} in a folder of its own");
                assert_eq!(start_refusal(controller, &pane("container", harness, bound.path())), None, "{controller} {harness} in the container");
            }
            assert_eq!(start_refusal(controller, &pane("host", "", bound.path())), None, "{controller}: a pane that is no agent's");
        }
    });
}

#[test]
fn a_turn_based_controller_leaves_it_to_the_turn_which_lets_only_claude_code_through() {
    let (data, bound, own) = folders("agentmux-controllers-b");
    crate::backend::mcp_config_file::with_data_home(data.path(), || {
        for controller in [BLOCK_CONTROLLER_SUBPROCESS, BLOCK_CONTROLLER_PERSISTENT] {
            for harness in HARNESSES {
                assert_eq!(start_refusal(controller, &pane("host", harness, bound.path())), None, "{controller} {harness}: no CLI starts with it");
            }
        }
        for harness in HARNESSES {
            let refused = turn_refusal(&pane("host", harness, bound.path()));
            if harness == "claude" {
                assert_eq!(refused, None, "its turn is given AgentMux's config alone");
            } else {
                assert_eq!(refused.as_deref(), Some(TURN_REFUSAL), "{harness}");
            }
            assert_eq!(turn_refusal(&pane("host", harness, own.path())), None, "{harness} in a folder of its own");
            assert_eq!(turn_refusal(&pane("container", harness, bound.path())), None, "{harness} in the container");
        }
    });
}

/// The controllers themselves, started directly (a restart of one that exists skips its creation): Claude Code
/// is refused there too, before anything is read from the pane's command.
#[test]
fn a_terminal_session_or_app_server_controller_does_not_start_claude_code_there() {
    let (data, bound, own) = folders("agentmux-controllers-c");
    crate::backend::mcp_config_file::with_data_home(data.path(), || {
        let there = pane("host", "claude", bound.path());
        let mut elsewhere = pane("host", "claude", own.path());
        elsewhere.remove("cmd");
        let shell = |kind: &str| shell::ShellController::new(kind.to_string(), "tab".to_string(), format!("block-rule-{kind}"), None, None, None, None, String::new());
        let controllers: Vec<(&str, Box<dyn Controller>)> = vec![
            ("shell", Box::new(shell(BLOCK_CONTROLLER_SHELL))),
            ("cmd", Box::new(shell(BLOCK_CONTROLLER_CMD))),
            ("acp", Box::new(acp::AcpController::new("tab".to_string(), "block-rule-acp".to_string(), None, None, None, None))),
            ("msp", Box::new(msp::MspController::new("block-rule-msp".to_string(), None, None, None, None))),
            ("app-server", Box::new(app_server_controller::AppServerController::new("block-rule-app-server".to_string(), None, None, None, None))),
        ];
        for (name, controller) in &controllers {
            assert_eq!(controller.start(there.clone(), None, false), Err(REFUSAL.to_string()), "{name}");
        }
        // In a folder of its own the same start gets past the rule (and stops at the pane having no command).
        for (name, controller) in controllers.iter().filter(|(name, _)| matches!(*name, "acp" | "msp")) {
            let refused = controller.start(elsewhere.clone(), None, false).unwrap_err();
            assert_ne!(refused, REFUSAL, "{name}");
            assert!(refused.contains("no cmd"), "{name}: {refused}");
        }
    });
}
