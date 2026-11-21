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
/// AgentMux's own config, settings, instructions and skills, and told to read no other
/// ([`with_nothing_of_the_folder`]).
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
        return with_nothing_of_the_folder(args, &work_dir);
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

/// A Claude turn on the host in a folder a container agent can write (`mcp_folder_rule`): the CLI takes its
/// MCP servers, settings, instructions and skills from AgentMux's own files outside the folder
/// (`host_agent_files`), and nothing of the folder's.
///
/// - `--strict-mcp-config` leaves out the servers Claude would read on its own (the folder's `.mcp.json`, the
///   account's, a plugin's), and `--mcp-config` names AgentMux's file. A config that is not there yet, or that
///   names a command by a bare name, leaves the turn with no servers until the next launch or repair writes it.
/// - `--setting-sources user` leaves out the folder's settings (and with them its hooks and environment), its
///   instruction files, skills, commands, subagents, output styles, workflows and the plugins its settings
///   turn on. `--settings` names AgentMux's own settings.
/// - `--add-dir` names the folder that holds the agent's startup instructions and nothing else, and
///   `--plugin-dir` the one that holds its skills and commands.
///
/// What the argv already had of these goes: a flag AgentMux sets itself (`--mcp-config`, `--settings`,
/// `--setting-sources`), and any flag that names a file or folder a container agent can write, or one that is
/// not written as an absolute path. Idempotent.
fn with_nothing_of_the_folder(args: Vec<String>, work_dir: &str) -> Vec<String> {
    use crate::backend::host_agent_files as own;
    const STRICT: &str = "--strict-mcp-config";
    const CONFIG: &str = "--mcp-config";
    const SOURCES: &str = "--setting-sources";
    const SETTINGS: &str = "--settings";
    const ADD_DIR: &str = "--add-dir";
    const PLUGIN_DIR: &str = "--plugin-dir";
    /// Flags AgentMux sets itself there: whatever the argv had of them goes, with every value.
    const OURS: &[&str] = &[CONFIG, SOURCES, SETTINGS];
    /// Flags that name a file or folder: a value stays only when it is outside every folder a container agent
    /// can write.
    const NAMES_A_PATH: &[&str] = &[ADD_DIR, PLUGIN_DIR, "--append-system-prompt-file", "--system-prompt-file", "--agents"];

    let config = crate::backend::mcp_config_file::host_config_path(work_dir);
    let dir = own::dir_of(work_dir);
    let given = |path: std::path::PathBuf, there: bool| there.then(|| path.to_string_lossy().into_owned());
    let settings = dir.as_deref().and_then(|d| given(own::settings_path(d), own::settings_path(d).is_file()));
    let instructions = dir.as_deref().and_then(|d| given(own::instructions_dir(d), own::instructions_path(d).is_file()));
    let plugin = dir.as_deref().and_then(|d| given(own::plugin_dir(d), own::plugin_dir(d).join(".claude-plugin").join("plugin.json").is_file()));
    let is_ours = |value: &str| [&instructions, &plugin].into_iter().flatten().any(|path| path == value);
    // An inline JSON value (`--agents '{...}'`) names no file.
    let stays = |flag: &str, value: &str| {
        let inline = flag == "--agents" && value.trim_start().starts_with('{');
        !is_ours(value) && (inline || (std::path::Path::new(value).is_absolute() && !crate::backend::mcp_folder_rule::holds_in(value)))
    };

    let mut out = Vec::with_capacity(args.len() + 10);
    let mut next = 0;
    while next < args.len() {
        let arg = &args[next];
        next += 1;
        if arg == STRICT {
            continue;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value)),
            _ => (arg.as_str(), None),
        };
        let (ours, names_a_path) = (OURS.contains(&flag), NAMES_A_PATH.contains(&flag));
        if !ours && !names_a_path {
            out.push(arg.clone());
            continue;
        }
        // Its values: the one written with it, or what follows up to the next flag (one value, except for the
        // two flags that take several).
        let mut values: Vec<&str> = inline.into_iter().collect();
        if inline.is_none() {
            let several = flag == CONFIG || flag == ADD_DIR;
            while next < args.len() && !args[next].starts_with('-') && (several || values.is_empty()) {
                values.push(&args[next]);
                next += 1;
            }
        }
        if ours {
            continue;
        }
        let kept: Vec<String> = values.into_iter().filter(|value| stays(flag, value)).map(str::to_string).collect();
        if !kept.is_empty() {
            out.push(flag.to_string());
            out.extend(kept);
        }
    }

    out.push(STRICT.to_string());
    // Only a config that names its commands by their paths is handed over.
    if std::fs::read_to_string(&config).is_ok_and(|text| crate::backend::mcp_folder_rule::names_the_command_by_path(&text)) {
        out.extend([CONFIG.to_string(), config.to_string_lossy().into_owned()]);
    }
    out.extend([SOURCES.to_string(), "user".to_string()]);
    for (flag, value) in [(SETTINGS, settings), (ADD_DIR, instructions), (PLUGIN_DIR, plugin)] {
        if let Some(value) = value {
            out.extend([flag.to_string(), value]);
        }
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

            // A config that names the `agentmux` command by a bare name is not handed over either.
            crate::backend::mcp_config_file::write_relocated(&config, r#"{"mcpServers":{"agentmux":{"command":"agentmux-mcp"}}}"#).unwrap();
            let by_name = with_runtime_flags(&meta, s(&["-p"]));
            assert!(by_name.iter().any(|a| a == "--strict-mcp-config") && !by_name.iter().any(|a| a == "--mcp-config"), "{by_name:?}");

            let by_path = serde_json::json!({ "mcpServers": { "agentmux": { "command": data.path().join("tools").join("agentmux-mcp").to_string_lossy() } } });
            crate::backend::mcp_config_file::write_relocated(&config, &by_path.to_string()).unwrap();
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

    /// The files AgentMux gives a host agent there, as a launch writes them.
    const LAUNCH: &[(&str, &str)] = &[
        ("CLAUDE.md", "be kind"),
        (".claude/settings.json", r#"{"hooks":{}}"#),
        (".claude/skills/review/SKILL.md", "---\nname: review\n---\nbody"),
        (".mcp.json", r#"{"mcpServers":{}}"#),
    ];

    #[test]
    fn a_claude_turn_there_takes_its_settings_instructions_and_skills_from_agentmuxs_own_files() {
        let (data, folder, elsewhere) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let dir = folder.path().to_string_lossy().into_owned();
        crate::backend::mcp_config_file::with_data_home(data.path(), || {
            let mut meta = meta_of("claude");
            meta.insert("cmd:cwd".to_string(), json!(dir));
            crate::backend::mcp_folder_rule::mark(&dir);

            // Before a launch has written anything: the folder's own is left out all the same.
            let bare = with_runtime_flags(&meta, s(&["-p"]));
            assert_eq!(after(&bare, "--setting-sources").as_deref(), Some("user"), "{bare:?}");
            assert!(!bare.iter().any(|a| a == "--settings" || a == "--add-dir" || a == "--plugin-dir"), "nothing is named that is not there: {bare:?}");

            assert_eq!(crate::backend::host_agent_files::write(&dir, LAUNCH.iter().copied()), Ok(true));
            let own = crate::backend::host_agent_files::dir_of(&dir).unwrap();
            let shown = |path: std::path::PathBuf| path.to_string_lossy().into_owned();
            let out = with_runtime_flags(&meta, s(&["-p"]));
            assert_eq!(after(&out, "--setting-sources").as_deref(), Some("user"));
            assert_eq!(after(&out, "--settings"), Some(shown(crate::backend::host_agent_files::settings_path(&own))));
            assert_eq!(after(&out, "--add-dir"), Some(shown(crate::backend::host_agent_files::instructions_dir(&own))));
            assert_eq!(after(&out, "--plugin-dir"), Some(shown(crate::backend::host_agent_files::plugin_dir(&own))));
            assert!(out.iter().any(|a| a == "--strict-mcp-config"));
            assert!(!out.iter().any(|a| std::path::Path::new(a).starts_with(folder.path())), "no flag names a path in the folder: {out:?}");
            assert_eq!(with_runtime_flags(&meta, out.clone()), out, "added once");

            // The agent's own flags: what AgentMux sets itself is replaced, a path in the folder or a relative
            // one goes, a path elsewhere and every other flag stay.
            let kept_dir = elsewhere.path().to_string_lossy().into_owned();
            let in_folder = folder.path().join("sub").to_string_lossy().into_owned();
            let own_flags = s(&[
                "-p", "--setting-sources", "user,project,local", "--settings", "mine.json", "--settings={\"env\":{}}", "--add-dir", &in_folder, &kept_dir, "rel/dir",
                "--plugin-dir", &in_folder, "--plugin-dir", &kept_dir, "--append-system-prompt-file", "notes.md", "--system-prompt-file=prompt.md",
                "--agents", "{\"helper\":{\"description\":\"d\",\"prompt\":\"p\"}}", "--agents", "agents.json", "--model", "sonnet", "--append-system-prompt", "be brief",
            ]);
            let out = with_runtime_flags(&meta, own_flags);
            assert_eq!(out.iter().filter(|a| *a == "--setting-sources").count(), 1, "{out:?}");
            assert_eq!(after(&out, "--setting-sources").as_deref(), Some("user"));
            assert!(!out.iter().any(|a| a == "user,project,local" || a == "mine.json" || a.starts_with("--settings=")), "{out:?}");
            assert_eq!(out.iter().filter(|a| *a == "--settings").count(), 1);
            assert!(!out.iter().any(|a| a == &in_folder || a == "rel/dir" || a == "notes.md" || a == "agents.json" || a.contains("prompt.md")), "{out:?}");
            assert_eq!(out.iter().filter(|a| *a == &kept_dir).count(), 2, "a folder elsewhere stays, for both flags: {out:?}");
            assert!(out.iter().any(|a| a.starts_with("{\"helper\"")), "subagents written inline name no file: {out:?}");
            assert!(!out.iter().any(|a| a == "--append-system-prompt-file" || a == "--system-prompt-file"), "a flag left with no value goes: {out:?}");
            assert_eq!(after(&out, "--model").as_deref(), Some("sonnet"));
            assert_eq!(after(&out, "--append-system-prompt").as_deref(), Some("be brief"));
            assert_eq!(with_runtime_flags(&meta, out.clone()), out, "added once");

            // A folder of the agent's own, the container agent itself and another harness get none of this.
            let mut plain = meta_of("claude");
            plain.insert("cmd:cwd".to_string(), json!(elsewhere.path().to_string_lossy()));
            let mut container = meta.clone();
            container.insert("agentMode".to_string(), json!("container"));
            let mut codex = meta.clone();
            codex.insert("agentProvider".to_string(), json!("codex"));
            for (why, meta) in [("a folder of its own", plain), ("the container agent", container), ("another harness", codex)] {
                let out = with_runtime_flags(&meta, s(&["-p", "--add-dir", "rel/dir"]));
                assert!(!out.iter().any(|a| a == "--setting-sources" || a == "--settings" || a == "--plugin-dir"), "{why}: {out:?}");
                assert_eq!(after(&out, "--add-dir").as_deref(), Some("rel/dir"), "{why}: its own flags are as it wrote them");
            }
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
