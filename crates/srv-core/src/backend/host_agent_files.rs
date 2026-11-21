// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a host Claude Code agent is given from outside its folder, where a container agent can write the folder.
//!
//! In a folder the rule of [`mcp_folder_rule`](crate::backend::mcp_folder_rule) holds for, a host agent's CLI
//! takes nothing of the folder's own configuration: it is started with the setting sources limited to the
//! person's own (`agent_mcp_flag`), so the folder's settings, hooks, instruction files, skills, commands,
//! subagents, output styles, workflows and plugins are not loaded, and with its scheduler off
//! ([`apply_for_spawn`]), because a scheduled prompt can be kept in the folder. What AgentMux itself gives the
//! agent is written beside its MCP config, in the folder kept for host agents alone, and named to the CLI:
//!
//! - `settings.json`: AgentMux's hooks, each named by the path of the wrapper that ships with this install, and
//!   the agent's own settings ([`settings_with_hooks_by_path`]);
//! - `instructions/CLAUDE.md`: the agent's startup instructions;
//! - `plugin/`: the agent's skills and commands, which the CLI lists as `agentmux:<name>`.
//!
//! A host launch writes none of these into the folder there ([`write`]), and the pane is told what of the
//! folder was not loaded, by kind and count ([`left_out`]).
//!
//! What each flag and variable keeps out was checked against a live CLI, per kind of file, with the version
//! named by [`CHECKED_WITH_CLAUDE_CODE`]. The check is repeated when the pinned version moves.

use std::path::{Path, PathBuf};

use serde_json::Value;

use agentmux_common::mcp_config_path as shared;

/// The Claude Code version the flags and variables of this module were last checked against, kind by kind.
/// It follows the pinned version: a test fails when the pin moves until the check has been run again.
pub const CHECKED_WITH_CLAUDE_CODE: &str = "2.1.321";

/// The plugin AgentMux's own skills and commands are given as: the CLI lists them as `agentmux:<name>`.
pub const PLUGIN_NAME: &str = "agentmux";

const SETTINGS_FILE: &str = "settings.json";
const INSTRUCTIONS_DIR: &str = "instructions";
const INSTRUCTIONS_FILE: &str = "CLAUDE.md";
const PLUGIN_DIR: &str = "plugin";
const WRAPPER: &str = "agentmux-bashwrap";
/// The names of the agent's own MCP servers that the last launch left out of its config: their command is a
/// bare name with no path outside the folder on this computer.
const SERVERS_WITH_NO_PATH: &str = "servers-with-no-path.json";

/// The most entries one folder walk counts: a row says "200+" beyond it.
const MOST_COUNTED: usize = 200;
/// How deep a walk goes under a `.claude` folder of the folder.
const DEEPEST: usize = 4;

/// The variable that turns the CLI's scheduler off: nothing scheduled fires, and `/loop` and `CronCreate` are
/// not offered.
const SCHEDULER_OFF: &str = "CLAUDE_CODE_DISABLE_CRON";
/// The variable that has the CLI read the `CLAUDE.md` of a folder it is given with `--add-dir`.
const READ_ADDED_INSTRUCTIONS: &str = "CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD";

/// Where a host agent of `work_dir` is given its own files, when the rule holds for the folder. Reads the disk
/// and the store.
pub fn dir_of(work_dir: &str) -> Option<PathBuf> {
    if !crate::backend::mcp_folder_rule::holds_in(work_dir) {
        return None;
    }
    let expanded = crate::backend::blockcontroller::core::expand_home_dir(work_dir);
    shared::host_only_dir_for(Path::new(&expanded))
}

/// AgentMux's settings for the agent, in `dir` ([`dir_of`]).
pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join(SETTINGS_FILE)
}

/// The folder that holds the agent's startup instructions and nothing else, in `dir`.
pub fn instructions_dir(dir: &Path) -> PathBuf {
    dir.join(INSTRUCTIONS_DIR)
}

/// The agent's startup instructions, in `dir`.
pub fn instructions_path(dir: &Path) -> PathBuf {
    instructions_dir(dir).join(INSTRUCTIONS_FILE)
}

/// The plugin folder that holds the agent's skills and commands, in `dir`.
pub fn plugin_dir(dir: &Path) -> PathBuf {
    dir.join(PLUGIN_DIR)
}

/// Where the launch file `name` (as a launch names it, relative to the working folder) goes under the agent's
/// own folder, or `None` for a file that is not given to a host Claude Code agent from outside.
fn place_of(name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    if name == "CLAUDE.md" {
        return Some(format!("{INSTRUCTIONS_DIR}/{INSTRUCTIONS_FILE}"));
    }
    if name == ".claude/settings.json" {
        return Some(SETTINGS_FILE.to_string());
    }
    for (from, to) in [(".claude/skills/", "skills/"), (".claude/commands/", "commands/")] {
        if let Some(rest) = name.strip_prefix(from).filter(|rest| !rest.is_empty()) {
            return Some(format!("{PLUGIN_DIR}/{to}{rest}"));
        }
    }
    None
}

/// Write a host launch's files for `work_dir`, a folder the rule holds for: each goes to the agent's own folder
/// outside it, and nothing is written into the folder. `files` are (name, content) as the launch rendered them.
/// The skills and instructions of an earlier launch are removed first. A file with no place outside (another
/// harness's startup file) is not written: no other harness is started there. `Ok(false)` when the rule does
/// not hold and the caller writes into the folder as before.
pub fn write<'a>(work_dir: &str, files: impl Iterator<Item = (&'a str, &'a str)>) -> Result<bool, String> {
    let Some(dir) = dir_of(work_dir) else { return Ok(false) };
    for stale in [instructions_dir(&dir), plugin_dir(&dir)] {
        match std::fs::remove_dir_all(&stale) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("failed to clear {}: {e}", stale.display())),
        }
    }
    let mut in_plugin = false;
    for (name, content) in files {
        if name == shared::MCP_JSON {
            let (text, no_path) = crate::backend::mcp_folder_rule::with_commands_by_path(content, work_dir);
            write_one(&dir.join(shared::RELOCATED_MCP_JSON), &text)?;
            write_one(&dir.join(SERVERS_WITH_NO_PATH), &serde_json::json!(no_path).to_string())?;
            continue;
        }
        let Some(place) = place_of(name) else {
            tracing::debug!(work_dir, name, "not written: a host agent in a folder a container agent can write is given it from nowhere");
            continue;
        };
        let path = crate::backend::base::safe_join_within_base(&dir, &place).map_err(|e| format!("path traversal denied: {name} ({e})"))?;
        if place == SETTINGS_FILE {
            write_one(&path, &settings_with_hooks_by_path(content, work_dir))?;
        } else if place.starts_with(INSTRUCTIONS_DIR) {
            write_one(&path, &as_agentmux_wrote_it(content))?;
        } else {
            in_plugin = true;
            write_one(&path, content)?;
        }
    }
    if in_plugin {
        write_plugin_manifest(&dir)?;
    }
    Ok(true)
}

/// The agent's skills and commands for `work_dir` again, as a controller start refreshes them: only the plugin
/// folder outside is rewritten, and nothing is written into the folder. `Ok(false)` when the rule does not hold.
pub fn refresh_skills<'a>(work_dir: &str, files: impl Iterator<Item = (&'a str, &'a str)>) -> Result<bool, String> {
    let Some(dir) = dir_of(work_dir) else { return Ok(false) };
    match std::fs::remove_dir_all(plugin_dir(&dir)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("failed to clear {}: {e}", plugin_dir(&dir).display())),
    }
    let mut in_plugin = false;
    for (name, content) in files {
        let Some(place) = place_of(name).filter(|place| place.starts_with(PLUGIN_DIR)) else { continue };
        let path = crate::backend::base::safe_join_within_base(&dir, &place).map_err(|e| format!("path traversal denied: {name} ({e})"))?;
        in_plugin = true;
        write_one(&path, content)?;
    }
    if in_plugin {
        write_plugin_manifest(&dir)?;
    }
    Ok(true)
}

fn write_plugin_manifest(dir: &Path) -> Result<(), String> {
    let manifest = serde_json::json!({ "name": PLUGIN_NAME, "version": "1.0.0", "description": "The skills and commands AgentMux gives this agent" });
    write_one(&plugin_dir(dir).join(".claude-plugin").join("plugin.json"), &manifest.to_string())
}

/// The agent's own MCP servers the last launch left out of its config in `dir` ([`dir_of`]), by name: each is
/// set with a bare command that has no path outside the folder on this computer.
pub fn servers_with_no_path(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join(SERVERS_WITH_NO_PATH))
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

/// Write the agent's startup instructions for `work_dir` again (a refresh while it runs). `Ok(false)` when the
/// rule does not hold for the folder, or no launch has written them yet.
pub fn refresh_instructions(work_dir: &str, content: &str) -> Result<bool, String> {
    let Some(dir) = dir_of(work_dir) else { return Ok(false) };
    let path = instructions_path(&dir);
    if !path.is_file() {
        return Ok(false);
    }
    write_one(&path, &as_agentmux_wrote_it(content)).map(|()| true)
}

/// The startup instructions as the file holds them: under the line that says AgentMux wrote the file, as the
/// startup file in an agent's own folder is, so what reads it back knows it for AgentMux's.
fn as_agentmux_wrote_it(content: &str) -> String {
    format!("{}\n\n{content}", crate::backend::agent_config::CLAUDE_MD_MANAGED_MARKER)
}

fn write_one(path: &Path, content: &str) -> Result<(), String> {
    crate::backend::mcp_config_file::write_relocated(path, content).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

/// The hook command `command` with AgentMux's wrapper named by `wrapper`'s path; `None` for a command that is
/// not the wrapper's. A path is quoted, with forward slashes, so one shell word names it on every platform.
fn wrapper_by_path(command: &str, wrapper: &Path) -> Option<String> {
    let rest = command.strip_prefix(WRAPPER)?;
    if !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    let path = wrapper.to_string_lossy().replace('\\', "/");
    Some(format!("\"{path}\"{rest}"))
}

/// Whether `path` can stand in a quoted shell word as it is.
fn plain_in_quotes(path: &Path) -> bool {
    !path.to_string_lossy().chars().any(|c| matches!(c, '"' | '$' | '`' | '\n'))
}

/// `settings` (the text of the settings AgentMux renders for an agent) as it is given to a host agent of
/// `work_dir`: each of AgentMux's own hooks names the wrapper by the absolute path of the one that ships with
/// this install, never by a bare name. Where none ships outside the folder, those hooks are taken out: no hook
/// rather than a command found by name. Text that is not a JSON object is returned as it is.
pub fn settings_with_hooks_by_path(settings: &str, work_dir: &str) -> String {
    let wrapper = crate::backend::tool_store::bashwrap_path()
        .filter(|path| plain_in_quotes(path) && crate::backend::mcp_folder_rule::is_outside(path, work_dir));
    if wrapper.is_none() {
        tracing::warn!(work_dir, "no agentmux-bashwrap ships outside this folder: the agent's settings are written without AgentMux's hooks");
    }
    hooks_by_path(settings, wrapper.as_deref())
}

fn hooks_by_path(settings: &str, wrapper: Option<&Path>) -> String {
    let Ok(mut doc) = serde_json::from_str::<Value>(settings) else { return settings.to_string() };
    let Some(events) = doc.get_mut("hooks").and_then(Value::as_object_mut) else { return settings.to_string() };
    for entries in events.values_mut().filter_map(Value::as_array_mut) {
        for entry in entries.iter_mut() {
            let Some(hooks) = entry.get_mut("hooks").and_then(Value::as_array_mut) else { continue };
            hooks.retain_mut(|hook| {
                let Some(command) = hook.get("command").and_then(Value::as_str) else { return true };
                let is_ours = command == WRAPPER || command.starts_with(&format!("{WRAPPER} "));
                if !is_ours {
                    return true;
                }
                match wrapper.and_then(|path| wrapper_by_path(command, path)) {
                    Some(by_path) => {
                        hook["command"] = Value::String(by_path);
                        true
                    }
                    None => false,
                }
            });
        }
        entries.retain(|entry| entry.get("hooks").and_then(Value::as_array).is_none_or(|hooks| !hooks.is_empty()));
    }
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| settings.to_string())
}

/// What of a folder's own files a host agent there is not given, by kind: how many of each. Counted, never
/// read as configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct LeftOut {
    /// `.claude/settings.json` and `.claude/settings.local.json`.
    pub settings: usize,
    /// `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md` and the rules under `.claude/rules`.
    pub instructions: usize,
    pub skills: usize,
    pub commands: usize,
    pub agents: usize,
    /// Output styles, workflows and the file of scheduled prompts.
    pub other: usize,
}

impl LeftOut {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The files under `dir` (to [`DEEPEST`]) that `counts` says yes to, at most [`MOST_COUNTED`]. Links are not
/// followed.
fn count_under(dir: &Path, counts: &dyn Fn(&Path) -> bool) -> usize {
    fn walk(dir: &Path, depth: usize, seen: &mut usize, counts: &dyn Fn(&Path) -> bool, total: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            *seen += 1;
            if *seen > MOST_COUNTED * 10 || *total >= MOST_COUNTED {
                return;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() && depth < DEEPEST {
                walk(&path, depth + 1, seen, counts, total);
            } else if kind.is_file() && counts(&path) {
                *total += 1;
            }
        }
    }
    let (mut seen, mut total) = (0, 0);
    walk(dir, 0, &mut seen, counts, &mut total);
    total
}

fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|x| x.eq_ignore_ascii_case("md"))
}

/// The skill and command files AgentMux itself wrote into `folder` at an earlier launch, as its manifest there
/// lists them: they are AgentMux's own, given from outside now, and not counted as the folder's.
fn written_by_agentmux(folder: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(folder.join(crate::backend::agent_config::MANAGED_SKILL_FILES_MANIFEST))
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
        .into_iter()
        .take(MOST_COUNTED)
        .map(|rel| folder.join(rel))
        .collect()
}

/// What of `folder`'s own files a host agent there is not given. A file AgentMux wrote there itself (its
/// startup file, its settings, the skills its manifest lists) is not counted: the agent is given that from
/// outside. Only looks at names, and at the first bytes of two files to tell AgentMux's own.
pub fn left_out(folder: &str) -> LeftOut {
    let folder = Path::new(folder);
    let claude = folder.join(".claude");
    let ours = written_by_agentmux(folder);
    let theirs = |path: &Path| !ours.iter().any(|own| own == path);
    let starts_with = |path: &Path, marker: &str| {
        use std::io::Read;
        let mut head = vec![0u8; marker.len()];
        std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut head)).is_ok() && head == marker.as_bytes()
    };
    let own_settings = |path: &Path| std::fs::read_to_string(path).is_ok_and(|text| text.len() < 1 << 20 && text.contains(WRAPPER));

    let mut out = LeftOut::default();
    let settings = claude.join("settings.json");
    out.settings = usize::from(settings.is_file() && !own_settings(&settings)) + usize::from(claude.join("settings.local.json").is_file());
    let startup = folder.join("CLAUDE.md");
    out.instructions = usize::from(startup.is_file() && !starts_with(&startup, crate::backend::agent_config::CLAUDE_MD_MANAGED_MARKER))
        + usize::from(claude.join("CLAUDE.md").is_file())
        + usize::from(folder.join("CLAUDE.local.md").is_file())
        + count_under(&claude.join("rules"), &is_markdown);
    out.skills = count_under(&claude.join("skills"), &|path| path.file_name().is_some_and(|n| n == "SKILL.md") && theirs(path));
    out.commands = count_under(&claude.join("commands"), &|path| is_markdown(path) && theirs(path));
    out.agents = count_under(&claude.join("agents"), &is_markdown);
    out.other = count_under(&claude.join("output-styles"), &is_markdown)
        + count_under(&claude.join("workflows"), &|_| true)
        + usize::from(claude.join("scheduled_tasks.json").is_file());
    out
}

/// The environment of a host Claude Code agent's CLI in a folder the rule holds for: its scheduler is off (a
/// scheduled prompt can be kept in the folder, and would run as a turn of this agent), and it reads the
/// startup instructions of the folder AgentMux adds for it. Set over whatever the agent's own environment
/// says. Every other agent's environment is left as it is.
pub fn apply_for_spawn(env_vars: &mut std::collections::HashMap<String, String>, block_meta: &crate::backend::obj::MetaMapType) {
    let provider = crate::backend::providers::resolve_provider_alias(&crate::backend::obj::meta_get_string(block_meta, "agentProvider", ""));
    if provider != "claude" || crate::backend::mcp_folder_rule::folder_of(block_meta).is_none() {
        return;
    }
    env_vars.insert(SCHEDULER_OFF.to_string(), "1".to_string());
    env_vars.insert(READ_ADDED_INSTRUCTIONS.to_string(), "1".to_string());
}

#[cfg(test)]
#[path = "host_agent_files_tests.rs"]
mod tests;
