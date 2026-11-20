// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The MCP config flags of a Claude turn: which config file Claude is pointed at, and when it is told to read
//! no other. `agent_runtime::with_runtime_flags` adds them after the model and permission flags.

use crate::backend::obj::MetaMapType;

/// Point a Claude turn at the agent's MCP config when it lives outside the working directory
/// (#3680): a launch into the user's own project writes it under AgentMux's data root, which
/// Claude does not find on its own. `--mcp-config` adds it to whatever Claude already reads, so
/// the project's own `.mcp.json` keeps reaching it. Every other harness is handed the servers by
/// srv (`mcp_config_file::read_for_turn`), so it needs no flag.
///
/// Never overrides a `--mcp-config` already in the argv (an agent's own provider flags), and
/// idempotent, so a pane that spawns through more than one of the paths gets one flag.
///
/// A host agent in a folder a container agent can write is the exception: there Claude is given
/// AgentMux's config and told to read no other ([`with_only_agentmux_servers`]).
pub(crate) fn with_mcp_config_flag(meta: &MetaMapType, mut args: Vec<String>) -> Vec<String> {
    let provider = crate::backend::providers::resolve_provider_alias(&crate::backend::obj::meta_get_string(meta, "agentProvider", ""));
    if provider != "claude" {
        return args;
    }
    let work_dir = crate::backend::obj::meta_get_string(meta, "cmd:cwd", "");
    if work_dir.trim().is_empty() {
        return args;
    }
    if crate::backend::mcp_folder_rule::folder_of(meta).is_some() {
        return with_only_agentmux_servers(args, &crate::backend::mcp_config_file::host_config_path(&work_dir));
    }
    if crate::backend::agent_runtime::flag_value(&args, &["--mcp-config"]).is_some() {
        return args;
    }
    let Some(path) = crate::backend::mcp_config_file::relocated_path(&work_dir) else {
        return args;
    };
    if !path.is_file() {
        return args;
    }
    let runtime = crate::backend::runtime_adapter::RuntimeKind::of_block_meta(meta);
    let shown = runtime
        .adapter()
        .map_mount(crate::backend::runtime_adapter::MountRole::McpConfig, &path.to_string_lossy());
    args.push("--mcp-config".to_string());
    args.push(shown);
    args
}

/// A Claude turn on the host in a folder a container agent can write (`mcp_folder_rule`): Claude starts the
/// servers of `config`, AgentMux's own file outside the folder, and no other. `--strict-mcp-config` leaves
/// out what Claude would read on its own (the folder's `.mcp.json`, the account's servers, a plugin's), and
/// a `--mcp-config` the argv already had goes too, so nothing names a file in the folder. A config that is
/// not there yet leaves the turn with no servers until the next launch or repair writes it. Idempotent.
fn with_only_agentmux_servers(args: Vec<String>, config: &std::path::Path) -> Vec<String> {
    const STRICT: &str = "--strict-mcp-config";
    const CONFIG: &str = "--mcp-config";
    let mut out = Vec::with_capacity(args.len() + 3);
    let mut values_follow = false;
    for arg in args {
        if arg == CONFIG {
            // It takes one or more values: everything up to the next flag.
            values_follow = true;
            continue;
        }
        if values_follow && !arg.starts_with('-') {
            continue;
        }
        values_follow = false;
        if arg == STRICT || arg.starts_with("--mcp-config=") {
            continue;
        }
        out.push(arg);
    }
    out.push(STRICT.to_string());
    if config.is_file() {
        out.push(CONFIG.to_string());
        out.push(config.to_string_lossy().into_owned());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::agent_runtime::with_runtime_flags;
    use serde_json::json;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    fn after(args: &[String], flag: &str) -> Option<String> {
        args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
    }
    fn meta_of(provider: &str) -> MetaMapType {
        let mut m = MetaMapType::new();
        m.insert("agentProvider".to_string(), json!(provider));
        m
    }

    #[test]
    fn a_claude_turn_in_a_folder_a_container_agent_can_write_reads_agentmuxs_config_alone() {
        let (data, folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let dir = folder.path().to_string_lossy().into_owned();
        crate::backend::mcp_config_file::with_data_home(data.path(), || {
            let mut meta = meta_of("claude");
            meta.insert("cmd:cwd".to_string(), json!(dir));
            crate::backend::mcp_folder_rule::mark(&dir);
            let config = crate::backend::mcp_config_file::host_config_path(&dir);
            let config_text = config.to_string_lossy().into_owned();

            // Before AgentMux's config is written: no servers at all, never the folder's.
            let bare = with_runtime_flags(&meta, s(&["-p"]));
            assert!(bare.iter().any(|a| a == "--strict-mcp-config") && !bare.iter().any(|a| a == "--mcp-config"), "{bare:?}");

            crate::backend::mcp_config_file::write_relocated(&config, r#"{"mcpServers":{"agentmux":{}}}"#).unwrap();
            let own_flags = s(&["-p", "--mcp-config", ".mcp.json", "extra.json", "--model", "sonnet", "--mcp-config=other.json"]);
            let out = with_runtime_flags(&meta, own_flags);
            assert_eq!(out.iter().filter(|a| *a == "--strict-mcp-config").count(), 1, "{out:?}");
            assert_eq!(out.iter().filter(|a| *a == "--mcp-config").count(), 1, "{out:?}");
            assert_eq!(after(&out, "--mcp-config").as_deref(), Some(config_text.as_str()), "AgentMux's file outside the folder");
            assert!(!out.iter().any(|a| a == ".mcp.json" || a == "extra.json" || a.starts_with("--mcp-config=")), "no other config is named: {out:?}");
            assert_eq!(after(&out, "--model").as_deref(), Some("sonnet"), "the other flags stay");
            assert_eq!(with_runtime_flags(&meta, out.clone()), out, "added once");

            // The container agent itself, and a harness that is not Claude Code, get no such flags.
            let mut container = meta.clone();
            container.insert("agentMode".to_string(), json!("container"));
            assert!(!with_runtime_flags(&container, s(&["-p"])).iter().any(|a| a == "--strict-mcp-config"));
            let mut codex = meta.clone();
            codex.insert("agentProvider".to_string(), json!("codex"));
            assert!(!with_runtime_flags(&codex, s(&["exec", "-"])).iter().any(|a| a == "--strict-mcp-config"));
        });
    }

    #[test]
    fn a_claude_turn_in_a_folder_of_its_own_gets_no_strict_flag() {
        let (data, folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        crate::backend::mcp_config_file::with_data_home(data.path(), || {
            let mut meta = meta_of("claude");
            meta.insert("cmd:cwd".to_string(), json!(folder.path().to_string_lossy()));
            let own = s(&["-p", "--mcp-config", "mine.json"]);
            assert_eq!(with_runtime_flags(&meta, own.clone()).iter().filter(|a| a.contains("mcp-config")).collect::<Vec<_>>(), vec!["--mcp-config"], "as before: the agent's own flag stays, nothing is added");
        });
    }
}
