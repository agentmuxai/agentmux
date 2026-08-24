// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pure config-building logic for agent definitions.
//!
//! Ports the `buildConfigFiles`, `buildMcpConfig`, and `expandTemplate`
//! functions from `frontend/app/view/agent/agent-model.ts`.
//! Most functions here are pure — no I/O, no async. The exception is the
//! `managed skill files manifest` section near the end: real filesystem
//! I/O shared by the two independent RPC handlers that materialize config
//! files to disk (`agent.open`'s `write_agent_config_files` in
//! `server/app_api/agent_open.rs`, and `writeagentconfig` in
//! `server/editor_handlers.rs` — the latter is the actual "click Launch"
//! path used on every normal agent launch). Lives here rather than in
//! either handler file so the two callers can't drift out of sync on the
//! manifest format or the path-traversal defense (reagent P1, PR #2322 —
//! `writeagentconfig` initially had no cleanup at all).

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use serde_json::{json, Value};

use crate::backend::storage::store::{derive_slug, AgentSkill};

/// `skill_type` value that materializes a skill as an Agent Skills-format
/// `.claude/skills/<slug>/SKILL.md` instead of a `.claude/commands/<trigger>.md`
/// slash command. Any other value (in practice, always `"prompt"` today) keeps
/// the pre-existing slash-command behavior. See
/// `docs/specs/REPORT_ARMORY_BUNDLE_STANDARD_RESEARCH_2026_07_16.md` Phase 0 —
/// `skill_type` already existed end-to-end but was never branched on before
/// this, making it the lowest-friction place to hang the format discriminator
/// rather than adding a new column.
pub const SKILL_TYPE_AGENT_SKILL: &str = "agent-skill";

/// A single file to be written to the agent working directory.
#[derive(Debug, Clone)]
pub struct AgentConfigFile {
    /// Path relative to the agent working directory (e.g. `"CLAUDE.md"`, `".mcp.json"`).
    pub filename: String,
    /// UTF-8 file content.
    pub content: String,
}

/// Build the list of config files to write to the agent working directory.
///
/// Assembles the startup instructions file (`CLAUDE.md`, `AGENTS.md`,
/// `GEMINI.md`, ... — resolved per `provider_id` via
/// `providers::get_provider(provider_id).startup_instructions_filename`;
/// see docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md)
/// from `soul` + `agentmd` + `memory` + skills index, writes each skill as
/// a slash command under `.claude/commands/<trigger>.md`, writes
/// `.claude/hooks.json` if a `hooks` content entry is present, auto-injects
/// the AgentMux MCP server entry, and applies `{{VARIABLE}}` template
/// substitution throughout. An unrecognized `provider_id`, or one with no
/// confirmed native file (currently only `kimi`), gets no instructions
/// file written at all — never a silent `CLAUDE.md` fallback.
///
/// Mirrors `buildConfigFiles()` in `frontend/app/view/agent/agent-model.ts`.
pub fn build_config_files(
    content_map: &HashMap<String, String>,
    skills: &[AgentSkill],
    agent_name: &str,
    agent_id: &str,
    agent_slug: &str,
    working_directory: &str,
    provider_id: &str,
) -> Vec<AgentConfigFile> {
    let mut files: Vec<AgentConfigFile> = Vec::new();

    // Template variables for {{}} substitution
    let mut template_vars: HashMap<String, String> = HashMap::new();
    template_vars.insert("AGENT".to_string(), agent_name.to_string());
    template_vars.insert("AGENT_DISPLAY".to_string(), agent_name.to_string());
    template_vars.insert("AGENT_SLUG".to_string(), agent_slug.to_string());
    template_vars.insert("AGENT_ID".to_string(), agent_id.to_string());
    template_vars.insert("WORKING_DIR".to_string(), working_directory.to_string());
    // DATE in YYYY-MM-DD format, UTC
    template_vars.insert("DATE".to_string(), Utc::now().format("%Y-%m-%d").to_string());

    // ----------------------------------------------------------------
    // Build the startup instructions file: Soul + AgentMD + Memory + Skills index
    // ----------------------------------------------------------------
    let mut instructions_parts: Vec<String> = Vec::new();

    if let Some(soul) = content_map.get("soul") {
        instructions_parts.push(expand_template(soul, &template_vars));
    }
    if let Some(agentmd) = content_map.get("agentmd") {
        if !instructions_parts.is_empty() {
            instructions_parts.push("\n---\n".to_string());
        }
        instructions_parts.push(expand_template(agentmd, &template_vars));
    }
    if let Some(memory) = content_map.get("memory") {
        instructions_parts.push("\n# Memory\n".to_string());
        instructions_parts.push(memory.clone());
    }

    // Append skill index with trigger references
    if !skills.is_empty() {
        instructions_parts.push("\n# Available Skills\n\n".to_string());
        instructions_parts.push("Use `/<trigger>` to invoke a skill.\n\n".to_string());
        for skill in skills {
            let trigger_part = if skill.trigger.is_empty() {
                String::new()
            } else {
                format!(" (trigger: /{})", skill.trigger)
            };
            let desc_part = if skill.description.is_empty() {
                String::new()
            } else {
                format!(" \u{2014} {}", skill.description)
            };
            instructions_parts.push(format!("- **{}**{}{}\n", skill.name, trigger_part, desc_part));
        }
    }

    // Resolved per-provider — `None` for an unrecognized provider_id or one
    // with no confirmed native file (kimi) skips writing this file
    // entirely rather than guessing "CLAUDE.md".
    let instructions_filename = crate::backend::providers::get_provider(provider_id)
        .and_then(|p| p.startup_instructions_filename);
    if !instructions_parts.is_empty() {
        if let Some(filename) = instructions_filename {
            files.push(AgentConfigFile {
                filename: filename.to_string(),
                content: instructions_parts.join(""),
            });
        }
    }

    // ----------------------------------------------------------------
    // Write each skill as either a slash command
    // (.claude/commands/{trigger}.md, default) or an Agent Skills-format
    // SKILL.md (.claude/skills/{slug}/SKILL.md, skill_type ==
    // SKILL_TYPE_AGENT_SKILL) for native Claude Code consumption.
    // ----------------------------------------------------------------
    let mut used_skill_slugs: HashSet<String> = HashSet::new();
    for skill in skills {
        if skill.content.is_empty() {
            continue;
        }
        if skill.skill_type == SKILL_TYPE_AGENT_SKILL {
            let slug = unique_skill_slug(&skill.name, &mut used_skill_slugs);
            let content = expand_template(&skill.content, &template_vars);
            files.push(AgentConfigFile {
                filename: format!(".claude/skills/{slug}/SKILL.md"),
                content: render_skill_md(&slug, &skill.description, &content),
            });
        } else if let Some(safe_trigger) = sanitize_trigger(&skill.trigger) {
            let content = expand_template(&skill.content, &template_vars);
            files.push(AgentConfigFile {
                filename: format!(".claude/commands/{safe_trigger}.md"),
                content,
            });
        }
    }

    // ----------------------------------------------------------------
    // Write .claude/hooks.json — always includes a PreToolUse:Bash
    // entry pointing at `agentmux-bashwrap hook` so the streaming
    // wrapper is invoked for every Bash tool call, plus two
    // PreCompact entries (matcher "manual" / "auto") pointing at
    // `agentmux-bashwrap precompact` so a live "compaction started"
    // signal reaches the sidecar. User-provided hooks (from
    // content_map["hooks"]) are merged on top, with the user's
    // entries winning on key collisions, EXCEPT that our PreToolUse
    // and PreCompact entries are always appended to any user array
    // for those keys so streaming / compaction visibility stay on
    // regardless. See docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md
    // §5 and docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md
    // §4.2.
    // ----------------------------------------------------------------
    let user_hooks = content_map.get("hooks").map(|s| s.as_str());
    let user_settings = content_map.get("settings").map(|s| s.as_str());
    if let Some(settings_json) = build_settings_with_hooks(user_settings, user_hooks) {
        files.push(AgentConfigFile {
            filename: ".claude/settings.json".to_string(),
            content: settings_json,
        });
    }

    // ----------------------------------------------------------------
    // Build .mcp.json with auto-injected AgentMux MCP server
    // ----------------------------------------------------------------
    // agent_bus_id is not in the function signature; callers that have it
    // should call build_mcp_config directly and push the result themselves,
    // or use the variant below.
    let mcp_content = content_map.get("mcp").map(|s| s.as_str());
    if let Some(mcp_json) = build_mcp_config(mcp_content, agent_slug, "") {
        files.push(AgentConfigFile {
            filename: ".mcp.json".to_string(),
            content: mcp_json,
        });
    }

    files
}

/// Build `.mcp.json` content with the auto-injected AgentMux MCP server entry.
///
/// The AgentMux server is always present as `mcpServers.agentmux`.
/// If `user_mcp_content` is `Some`, its `mcpServers` entries are merged on top
/// (user entries win over the auto-injected entry if the key collides).
/// If the user content is not valid JSON the auto-injected-only config is
/// returned and no error is propagated (mirrors TS behavior).
///
/// Returns `None` only if serialization unexpectedly fails (should never happen).
///
/// Mirrors `buildMcpConfig()` in `frontend/app/view/agent/agent-model.ts`.
pub fn build_mcp_config(
    user_mcp_content: Option<&str>,
    agent_slug: &str,
    agent_bus_id: &str,
) -> Option<String> {
    // Auto-injected AgentMux MCP server entry.
    // agent_slug must be the pre-computed stable role slug (e.g. "korp"),
    // NOT the display name — callers are responsible for passing the right
    // value so renamed agents always advertise the same routing ID.
    let mut env_map = serde_json::Map::new();
    if !agent_slug.is_empty() {
        env_map.insert("AGENTMUX_AGENT_ID".to_string(), json!(agent_slug));
    }
    if !agent_bus_id.is_empty() {
        env_map.insert("AGENTMUX_AGENT_BUS_ID".to_string(), json!(agent_bus_id));
    }

    let agentmux_server = json!({
        "type": "stdio",
        "command": "agentmux-mcp",
        "args": [],
        "env": Value::Object(env_map),
    });

    let mut mcp_servers = serde_json::Map::new();
    mcp_servers.insert("agentmux".to_string(), agentmux_server);

    // Merge user-provided MCP config if present
    if let Some(raw) = user_mcp_content {
        match serde_json::from_str::<Value>(raw) {
            Ok(Value::Object(user_obj)) => {
                if let Some(Value::Object(user_servers)) = user_obj.get("mcpServers") {
                    for (k, v) in user_servers {
                        mcp_servers.insert(k.clone(), v.clone());
                    }
                }
            }
            Ok(_) => {
                // User content parsed but isn't an object — skip merge silently
            }
            Err(_) => {
                // Invalid JSON in agent content — keep auto-injected only (mirrors TS behavior)
                tracing::error!("agent_config: invalid MCP JSON in agent content, using auto-injected only");
            }
        }
    }

    let result = json!({ "mcpServers": Value::Object(mcp_servers) });
    match serde_json::to_string_pretty(&result) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::error!("agent_config: failed to serialize MCP config: {e}");
            None
        }
    }
}

/// Build `.mcp.json` from standalone McpServer ref rows (v1 composable model).
///
/// Layering (later wins on key collision, except the reserved `agentmux` key):
///   1. synthetic `agentmux` entry (always injected)
///   2. `user_mcp_blob`'s `mcpServers` — the legacy per-agent blob, merged when
///      the caller passes it (used so a global-only ref set never wipes a
///      legacy agent's user servers)
///   3. `servers` ref rows (the agent's own bound servers + globals)
/// For each ref row the `config` field is a JSON object used as the server
/// value directly. Falls back to `build_mcp_config` when there are no ref rows
/// and no blob.
pub fn build_mcp_config_from_refs(
    servers: &[crate::backend::storage::McpServer],
    user_mcp_blob: Option<&str>,
    agent_slug: &str,
    agent_bus_id: &str,
) -> Option<String> {
    if servers.is_empty() {
        return build_mcp_config(user_mcp_blob, agent_slug, agent_bus_id);
    }

    let mut env_map = serde_json::Map::new();
    if !agent_slug.is_empty() {
        env_map.insert("AGENTMUX_AGENT_ID".to_string(), json!(agent_slug));
    }
    if !agent_bus_id.is_empty() {
        env_map.insert("AGENTMUX_AGENT_BUS_ID".to_string(), json!(agent_bus_id));
    }
    let agentmux_server = json!({
        "type": "stdio",
        "command": "agentmux-mcp",
        "args": [],
        "env": Value::Object(env_map),
    });

    let mut mcp_servers = serde_json::Map::new();
    mcp_servers.insert("agentmux".to_string(), agentmux_server);

    // Layer 2: merge the legacy user blob's servers (skip the reserved key).
    if let Some(raw) = user_mcp_blob {
        match serde_json::from_str::<Value>(raw) {
            Ok(Value::Object(user_obj)) => {
                if let Some(Value::Object(user_servers)) = user_obj.get("mcpServers") {
                    for (k, v) in user_servers {
                        if k == "agentmux" {
                            continue;
                        }
                        mcp_servers.insert(k.clone(), v.clone());
                    }
                }
            }
            Ok(_) => {}
            Err(_) => {
                tracing::error!("agent_config: invalid MCP JSON in legacy blob; skipping blob merge");
            }
        }
    }

    // Layer 3: ref rows (own + global) overlay the blob.
    for server in servers {
        if server.name == "agentmux" {
            continue; // synthetic entry wins; user cannot override the key
        }
        match serde_json::from_str::<Value>(&server.config) {
            Ok(cfg) => {
                mcp_servers.insert(server.name.clone(), cfg);
            }
            Err(e) => {
                tracing::warn!(
                    id = %server.id,
                    name = %server.name,
                    error = %e,
                    "agent_config: invalid JSON in mcp_server.config; skipping entry"
                );
            }
        }
    }

    let result = json!({ "mcpServers": Value::Object(mcp_servers) });
    match serde_json::to_string_pretty(&result) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::error!("agent_config: failed to serialize ref-based MCP config: {e}");
            None
        }
    }
}

/// Convert standalone Skill records into the AgentSkill shape expected by
/// `build_config_files`. Used when the v1 ref tables are non-empty.
pub fn skills_to_agent_skills(
    skills: &[crate::backend::storage::Skill],
    agent_id: &str,
) -> Vec<crate::backend::storage::AgentSkill> {
    skills
        .iter()
        .map(|s| crate::backend::storage::AgentSkill {
            id: s.id.clone(),
            agent_id: agent_id.to_string(),
            name: s.name.clone(),
            trigger: s.trigger.clone(),
            skill_type: s.skill_type.clone(),
            description: s.description.clone(),
            content: s.content.clone(),
            created_at: s.created_at,
        })
        .collect()
}

/// Merge a user-supplied `settings.json`-level hook-array (`PreToolUse`
/// or `PreCompact`) with whatever is already staged in `hooks_obj`
/// under `key` (our auto-injected entries, possibly already carrying
/// legacy `content_map["hooks"]`-merged user entries from the earlier
/// pass in [`build_settings_with_hooks`]). User entries are PREPENDED
/// so their matchers/gates get first refusal; AgentMux's own entries
/// always stay last. A non-array value is warned and dropped rather
/// than silently discarded — same discipline as every other branch in
/// this function (reagent P1 on PR #813).
fn prepend_user_hook_array(
    hooks_obj: &mut serde_json::Map<String, Value>,
    key: &str,
    user_value: Value,
) {
    match user_value {
        Value::Array(mut user_arr) => {
            if let Some(Value::Array(ours)) = hooks_obj.remove(key) {
                user_arr.extend(ours);
            }
            hooks_obj.insert(key.to_string(), Value::Array(user_arr));
        }
        _ => {
            tracing::warn!(
                "agent_config: user settings.hooks.{key} is not an array; dropped"
            );
        }
    }
}

/// Build `.claude/settings.json` content with the auto-injected
/// PreToolUse Bash hook and PreCompact hooks (under the `"hooks"`
/// key). PreToolUse redirects Bash invocations into the streaming
/// wrapper (`agentmux-bashwrap exec`); PreCompact (two entries,
/// matcher `"manual"` / `"auto"`) pings the sidecar the instant
/// compaction starts (`agentmux-bashwrap precompact`) — see
/// `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`
/// §4.2. User-supplied settings.json (from the agent's
/// `content_map["settings"]`) is parsed and merged at the top level;
/// user-supplied legacy hooks content (from `content_map["hooks"]`)
/// is merged into `settings.hooks`.
///
/// **File location matters.** Claude Code reads project hooks from
/// `<project>/.claude/settings.json` under the `"hooks"` key.
/// A standalone `.claude/hooks.json` is NOT a Claude Code
/// discovery location — that was the v0.33.804 streaming-bug root
/// cause: the file was written but Claude never read it, so the
/// PreToolUse hook never fired and live streaming silently failed.
///
/// See `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` §5
/// and Claude Code docs: https://code.claude.com/docs/en/hooks.md
pub fn build_settings_with_hooks(
    user_settings_content: Option<&str>,
    user_hooks_content: Option<&str>,
) -> Option<String> {
    use serde_json::Value;
    let agentmux_pretooluse = json!({
        "matcher": "^(Bash|.*[Bb]ash.*)$",
        "hooks": [
            {
                "type": "command",
                "command": "agentmux-bashwrap hook"
            }
        ]
    });
    // `PreCompact` requires an explicit `matcher` (`"manual"` or
    // `"auto"`) — Claude Code has no confirmed wildcard-all value for
    // this hook — so two separate entries are registered, each with a
    // different static `--trigger=` argv baked in so the binary knows
    // which fired without needing it from stdin (PreCompact's stdin
    // payload carries no `trigger` field; see `precompact.rs`). See
    // `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`
    // §4.2.
    let agentmux_precompact_manual = json!({
        "matcher": "manual",
        "hooks": [
            {
                "type": "command",
                "command": "agentmux-bashwrap precompact --trigger=manual"
            }
        ]
    });
    let agentmux_precompact_auto = json!({
        "matcher": "auto",
        "hooks": [
            {
                "type": "command",
                "command": "agentmux-bashwrap precompact --trigger=auto"
            }
        ]
    });
    let mut hooks_obj = serde_json::Map::new();
    let mut pretooluse_entries: Vec<Value> = Vec::new();
    let mut precompact_entries: Vec<Value> = Vec::new();

    // Start with user hooks if present + parseable. Parse failures or
    // non-Object top-levels are logged at WARN so the diagnostic trail
    // surfaces — silent swallowing made user hooks disappear with no
    // signal (reagent P2 on PR #809).
    if let Some(raw) = user_hooks_content {
        match serde_json::from_str::<Value>(raw) {
            Ok(Value::Object(user_obj)) => {
                for (k, v) in user_obj {
                    if k == "PreToolUse" {
                        if let Value::Array(arr) = v {
                            pretooluse_entries.extend(arr);
                        } else {
                            tracing::warn!(
                                "agent_config: user hooks.PreToolUse is not an array; dropped"
                            );
                        }
                    } else if k == "PreCompact" {
                        if let Value::Array(arr) = v {
                            precompact_entries.extend(arr);
                        } else {
                            tracing::warn!(
                                "agent_config: user hooks.PreCompact is not an array; dropped"
                            );
                        }
                    } else {
                        hooks_obj.insert(k, v);
                    }
                }
            }
            Ok(other) => {
                tracing::warn!(
                    kind = ?other,
                    "agent_config: user hooks top-level value is not an object; dropped"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "agent_config: failed to parse user hooks JSON; dropped"
                );
            }
        }
    }
    // Append our entries last so user matchers (deny rules etc.) get a chance
    // to short-circuit before our rewrite / observation hooks. PreCompact
    // gets both matcher entries, in matcher order (manual, then auto).
    pretooluse_entries.push(agentmux_pretooluse);
    hooks_obj.insert("PreToolUse".to_string(), Value::Array(pretooluse_entries));
    precompact_entries.push(agentmux_precompact_manual);
    precompact_entries.push(agentmux_precompact_auto);
    hooks_obj.insert("PreCompact".to_string(), Value::Array(precompact_entries));

    // Build the settings.json object: start from user-supplied settings.json
    // (if any), then overlay our hooks key. User keys other than `hooks`
    // pass through unchanged.
    let mut settings_obj = serde_json::Map::new();
    if let Some(raw) = user_settings_content {
        match serde_json::from_str::<Value>(raw) {
            Ok(Value::Object(user_obj)) => {
                for (k, v) in user_obj {
                    settings_obj.insert(k, v);
                }
            }
            Ok(_other) => {
                tracing::warn!(
                    "agent_config: user settings.json top-level is not an object; dropped"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "agent_config: failed to parse user settings.json; dropped"
                );
            }
        }
    }
    // Merge: any existing hooks key from user settings is merged with our
    // additions. For PreToolUse and PreCompact specifically, user matchers
    // from settings.json are PREPENDED (not dropped) so they short-circuit
    // before our auto-injected entries — same ordering rule we apply to
    // legacy content_map["hooks"] entries for these two keys. For other
    // event types (PostToolUse, Stop, etc.) we keep user's entries
    // verbatim. Reagent P1 on PR #813 (the `continue` was a silent drop —
    // caught a real merge bug); PreCompact is deliberately folded into the
    // same array-merge discipline rather than the generic `or_insert`
    // below, or a user-supplied PreCompact entry would hit that path and
    // be silently and permanently dropped the moment PreCompact became
    // auto-injected too — the exact bug class this file already guards
    // PreToolUse against. See
    // `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`
    // §4.2.
    if let Some(Value::Object(existing_hooks)) = settings_obj.get("hooks").cloned() {
        for (k, v) in existing_hooks {
            if k == "PreToolUse" || k == "PreCompact" {
                prepend_user_hook_array(&mut hooks_obj, &k, v);
                continue;
            }
            hooks_obj.entry(k).or_insert(v);
        }
    }
    settings_obj.insert("hooks".to_string(), Value::Object(hooks_obj));

    // Claude Code requires the bashwrap exec command (produced by the hook rewrite)
    // to be in permissions.allow — otherwise it raises a permissions error and the
    // agent cannot run any bash commands. Merge with any user-supplied allow list
    // rather than overwriting it.
    {
        // Space before * enforces a command-name boundary: matches
        // "agentmux-bashwrap <args>" only, not other executables that
        // happen to share the prefix (e.g. agentmux-bashwrapXYZ).
        let bashwrap_allow = Value::String("Bash(agentmux-bashwrap *)".to_string());
        let mut allow_arr = match settings_obj.get("permissions") {
            Some(Value::Object(perms)) => match perms.get("allow") {
                Some(Value::Array(arr)) => arr.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        if !allow_arr.iter().any(|v| v == &bashwrap_allow) {
            allow_arr.push(bashwrap_allow);
        }
        let mut perms_obj = match settings_obj.remove("permissions") {
            Some(Value::Object(obj)) => obj,
            _ => serde_json::Map::new(),
        };
        perms_obj.insert("allow".to_string(), Value::Array(allow_arr));
        settings_obj.insert("permissions".to_string(), Value::Object(perms_obj));
    }

    match serde_json::to_string_pretty(&Value::Object(settings_obj)) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::error!("agent_config: failed to serialize settings.json: {e}");
            None
        }
    }
}

/// Replace `{{VARIABLE}}` placeholders in `content` with values from `vars`.
///
/// Placeholders that have no corresponding key in `vars` are left unchanged
/// (the original `{{VARIABLE}}` text is preserved).
///
/// Mirrors `expandTemplate()` in `frontend/app/view/agent/agent-model.ts`.
pub fn expand_template(content: &str, vars: &HashMap<String, String>) -> String {
    // Hand-rolled replacement to avoid pulling in a regex dependency.
    // Scans for `{{`, extracts the key name up to `}}`, and substitutes.
    let mut result = String::with_capacity(content.len());
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Look for '{{'
        if i + 1 < len && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // Find closing '}}'
            if let Some(rel) = content[i + 2..].find("}}") {
                let key = &content[i + 2..i + 2 + rel];
                // Only substitute if key is a simple word (alphanumeric + underscore)
                if key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                    if let Some(val) = vars.get(key) {
                        result.push_str(val);
                    } else {
                        // No match — preserve the original placeholder
                        result.push_str(&content[i..i + 2 + rel + 2]);
                    }
                    i += 2 + rel + 2; // skip past '}}'
                    continue;
                }
            }
        }
        // Not a placeholder start — copy character verbatim
        // Safety: i is always on a valid char boundary because we only advance
        // by 1 when not inside a placeholder, and UTF-8 single-byte characters
        // are the only ones we index directly.
        let ch = content[i..].chars().next().unwrap();
        result.push(ch);
        i += ch.len_utf8();
    }

    result
}

/// Agent Skills spec caps `description` at 1024 characters.
const SKILL_DESCRIPTION_MAX_LEN: usize = 1024;

/// Truncate `s` to at most `max_units` UTF-16 code units, on a UTF-8 char
/// boundary. Mirrors JS `.slice(0, n)` semantics (JS strings are indexed in
/// UTF-16 code units, so a Rust byte-length cap diverges from the TS mirror
/// for any non-ASCII text — reagent P2, PR #2322).
fn truncate_utf16_units(s: &str, max_units: usize) -> String {
    let mut units = 0usize;
    for (byte_idx, ch) in s.char_indices() {
        units += ch.len_utf16();
        if units > max_units {
            return s[..byte_idx].to_string();
        }
    }
    s.to_string()
}

/// Render an Agent Skills-format `SKILL.md`: YAML frontmatter with the two
/// required fields (`name`, `description` — per the spec's six-field
/// frontmatter; AgentMux doesn't populate the four optional ones today:
/// `license`, `compatibility`, `metadata`, `allowed-tools`), followed by the
/// skill's content as the Markdown body. See https://agentskills.io/specification.
///
/// `slug` (not the skill's raw display name) is REQUIRED here — the spec
/// requires `name` be lowercase/hyphenated and match its parent directory;
/// callers must pass the same value used to build the `.claude/skills/<slug>/`
/// path (reagent P1, PR #2322). `description` is validated: the spec requires
/// a non-empty value (falls back to a placeholder) capped at 1024 **UTF-16
/// code units** — matching the TS mirror's `.slice(0, 1024)` (JS string
/// indexing is UTF-16-code-unit based), since the UI permits arbitrary-length
/// free text with no spec-awareness and non-ASCII descriptions previously
/// truncated to different byte vs. UTF-16 lengths between the two paths
/// (reagent P2, PR #2322).
///
/// YAML double-quoted scalars use JSON-compatible escaping (YAML 1.2
/// §7.3.1), so `serde_json::to_string` on a plain string produces a valid,
/// correctly-escaped YAML value — this avoids hand-rolling YAML escaping or
/// adding a yaml crate dependency (this workspace has neither today) just
/// for two scalar fields.
pub(crate) fn render_skill_md(slug: &str, description: &str, body: &str) -> String {
    let owned_description;
    let description = if description.trim().is_empty() {
        "No description provided."
    } else {
        owned_description = truncate_utf16_units(description, SKILL_DESCRIPTION_MAX_LEN);
        owned_description.as_str()
    };
    let name_yaml =
        serde_json::to_string(slug).expect("string serialization is infallible");
    let description_yaml =
        serde_json::to_string(description).expect("string serialization is infallible");
    format!("---\nname: {name_yaml}\ndescription: {description_yaml}\n---\n\n{body}")
}

/// Validate a skill's `trigger` is safe to use as a single path segment in
/// `.claude/commands/<trigger>.md`. `trigger` is free-form user input with
/// no format validation anywhere upstream (the skill create/update RPCs and
/// the frontend form all accept it as-is), so a trigger containing a path
/// separator or a `..` segment previously let the resulting filename
/// resolve OUTSIDE the agent's working directory -- both for this write and,
/// once stale-file cleanup existed, for the corresponding delete (reagent
/// P1, PR #2322). Rejects (returns `None`) anything containing `/` or `\`,
/// or that is exactly `.`/`..`; callers skip writing that skill's command
/// file entirely rather than silently rewriting the trigger into something
/// the user didn't ask for.
fn sanitize_trigger(trigger: &str) -> Option<&str> {
    if trigger.is_empty() || trigger == "." || trigger == ".." {
        return None;
    }
    if trigger.contains('/') || trigger.contains('\\') {
        return None;
    }
    Some(trigger)
}

/// Derive a slug for an Agent Skill name that is valid per the Agent Skills
/// `name` grammar: lowercase letters, digits, and hyphens ONLY (no
/// underscores). `derive_slug` is shared with agent role-slugs, which
/// deliberately DO permit underscores, so it isn't spec-valid here as-is —
/// hyphenate underscores (and re-collapse any resulting run of hyphens)
/// rather than reusing it directly (Codex P1, PR #2322).
fn skill_name_slug(name: &str) -> String {
    let base = derive_slug(name).replace('_', "-");
    let collapsed: String = base
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        "skill".to_string()
    } else {
        collapsed
    }
}

/// Derive a filesystem-safe, COLLISION-FREE, spec-valid slug for an Agent
/// Skill name within one caller-scoped `used` set. `skill_name_slug` alone
/// can produce identical output for distinct names that differ only in
/// punctuation/whitespace (e.g. "Deploy Checklist" and "Deploy!!!Checklist"
/// both -> "deploy-checklist"), which would otherwise silently overwrite one
/// skill's `SKILL.md` with another's on disk (reagent P1, PR #2322). Appends
/// `-2`, `-3`, ... until the slug is unique within `used`, truncating the
/// base first so the suffixed result never exceeds the spec's 64-character
/// max (Codex P2, PR #2322 — a 64-char base plus `-2` was previously 66
/// chars). `bundle_export.rs` also reuses this for MCP server export
/// filenames, where the underscore-free/64-char constraints are stricter
/// than strictly required but remain filesystem-safe, so sharing this
/// implementation is still correct there.
pub(crate) fn unique_skill_slug(name: &str, used: &mut HashSet<String>) -> String {
    const MAX_LEN: usize = 64;
    let base = skill_name_slug(name);
    if used.insert(base.clone()) {
        return base;
    }
    let mut n: u32 = 2;
    loop {
        let suffix = format!("-{n}");
        let max_base_len = MAX_LEN.saturating_sub(suffix.len());
        let mut truncated_base = base.clone();
        if truncated_base.len() > max_base_len {
            let mut end = max_base_len;
            while end > 0 && !truncated_base.is_char_boundary(end) {
                end -= 1;
            }
            truncated_base.truncate(end);
        }
        let candidate = format!("{truncated_base}{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

// ============================================================
// Managed skill files manifest (I/O — see module doc comment)
// ============================================================

/// Hidden manifest (relative to the agent working directory) tracking which
/// skill-derived paths (`.claude/commands/*.md`, `.claude/skills/*/SKILL.md`)
/// AgentMux itself wrote on the last materialization, so a subsequent one
/// can delete ones that are no longer current without touching any
/// user-authored `.claude/commands`/`.claude/skills` content.
pub const MANAGED_SKILL_FILES_MANIFEST: &str = ".claude/.agentmux-managed-skill-files.json";

/// Compute the subset of `filenames` that are skill-derived managed paths
/// (the ones tracked in [`MANAGED_SKILL_FILES_MANIFEST`]) — everything else
/// (`CLAUDE.md`, `.mcp.json`, `.claude/settings.json`, ...) is always fully
/// regenerated at a fixed path every launch, so it has no staleness problem
/// to track.
pub fn managed_skill_file_paths<'a>(
    filenames: impl Iterator<Item = &'a str>,
) -> std::collections::BTreeSet<String> {
    filenames
        .filter(|f| f.starts_with(".claude/commands/") || f.starts_with(".claude/skills/"))
        .map(|f| f.to_string())
        .collect()
}

/// Delete skill-derived files a PREVIOUS materialization wrote (per the
/// on-disk manifest) but that are no longer part of `new_managed_paths` --
/// e.g. a skill's format switched between `"prompt"`/`"agent-skill"`, or it
/// was renamed/removed. Without this, the stale file stays on disk and
/// Claude keeps treating it as active alongside the newly selected format
/// (reagent P1 + Codex P1/P2, PR #2322).
///
/// MUST be called before writing the new files (so a deletion never races a
/// write to the same path); callers must call
/// [`write_managed_skill_file_manifest`] after writing to record the new
/// set for the next materialization. Best-effort: any individual read/parse
/// failure is treated as "no prior manifest" (nothing to clean up yet)
/// rather than propagated, since a missing/corrupt manifest must never
/// block the write that follows.
///
/// Every path is resolved through [`crate::backend::base::safe_join_within_base`]
/// before deletion — defense in depth against a manifest path that somehow
/// escapes the working directory (e.g. a future bypass of trigger
/// sanitization upstream, or manual tampering with the manifest file
/// itself); such a path is skipped with a warning, never followed.
pub fn cleanup_stale_managed_skill_files(
    base_path: &std::path::Path,
    new_managed_paths: &std::collections::BTreeSet<String>,
) {
    let manifest_path = base_path.join(MANAGED_SKILL_FILES_MANIFEST);
    let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
        return;
    };
    let Ok(old_paths) = serde_json::from_str::<Vec<String>>(&raw) else {
        return;
    };
    for old in &old_paths {
        if new_managed_paths.contains(old) {
            continue;
        }
        let old_path = match crate::backend::base::safe_join_within_base(base_path, old) {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!(
                    work_dir = %base_path.display(),
                    path = %old,
                    "cleanup_stale_managed_skill_files: refusing to delete a manifest path \
                     that escapes the working directory"
                );
                continue;
            }
        };
        let _ = std::fs::remove_file(&old_path);
        // Agent Skills format nests under .claude/skills/<slug>/ -- clean up
        // the now-empty slug directory too (no-op/fails silently if
        // anything else still lives there, e.g. a future scripts/ dir).
        if let Some(parent) = old_path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

/// Record `new_managed_paths` as the manifest for the NEXT call to
/// [`cleanup_stale_managed_skill_files`]. Best-effort: a write failure is
/// logged, not propagated — losing this write only means the next
/// materialization's stale-file cleanup is skipped once, not a correctness
/// issue for the current one.
pub fn write_managed_skill_file_manifest(
    base_path: &std::path::Path,
    new_managed_paths: &std::collections::BTreeSet<String>,
) {
    let manifest_path = base_path.join(MANAGED_SKILL_FILES_MANIFEST);
    if let Ok(manifest_json) = serde_json::to_string(new_managed_paths) {
        if let Err(e) = std::fs::write(&manifest_path, manifest_json) {
            tracing::warn!(
                work_dir = %base_path.display(),
                error = %e,
                "write_managed_skill_file_manifest: failed to write manifest; \
                 stale file cleanup may be skipped on the next materialization"
            );
        }
    }
}

// ============================================================
// CLAUDE.md ownership protection (I/O — see module doc comment)
// ============================================================

/// First line AgentMux writes on any `CLAUDE.md` it fully owns (freshly
/// created, or regenerated on every launch since). Its ABSENCE on an
/// EXISTING file is what marks that file as foreign — predates AgentMux
/// touching this working directory, or a human replaced AgentMux's file
/// with their own — and is never overwritten again. See
/// `docs/specs/SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md`.
pub const CLAUDE_MD_MANAGED_MARKER: &str = "<!-- agentmux:managed-claude-md -->";

/// AgentMux-owned side file carrying the full Soul+AgentMD+Memory+Skills
/// composition when the real `CLAUDE.md` is foreign — always safe to
/// regenerate in place every launch, unlike `CLAUDE.md` itself in that case.
pub const AGENTMUX_MEMORY_FILENAME: &str = ".claude/AGENTMUX_MEMORY.md";

/// One-time marker recording whether the `@import` line (below) has
/// already been offered for this working directory. Checked INSTEAD of
/// re-scanning `CLAUDE.md` content on every launch — without it, a user
/// who deliberately deletes the import line (opting out of AgentMux
/// content entirely) would see it silently reappear on their next launch.
const CLAUDE_MD_OWNERSHIP_MARKER_PATH: &str = ".claude/.agentmux-claude-md-ownership.json";

/// Comment wrapping the `@import` line so its origin — and how to remove
/// it — is unambiguous to anyone reading a foreign `CLAUDE.md` by hand.
const CLAUDE_MD_IMPORT_MARKER_COMMENT: &str =
    "<!-- agentmux:managed-import (safe to delete this line to opt out) -->";

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct ClaudeMdOwnershipMarker {
    import_line_offered: bool,
}

/// Resolve [`AGENTMUX_MEMORY_FILENAME`] and [`CLAUDE_MD_OWNERSHIP_MARKER_PATH`]
/// against `base_path`, verifying neither escapes it via a symlinked
/// ancestor (e.g. `.claude` itself existing as a symlink pointing outside
/// the working directory) — same defense-in-depth the config-file write
/// loops already apply to their own paths (codex P1 on PR #2747). Both
/// constants are fixed, not user-controllable, so `safe_join_within_base`
/// itself can never fail here; the symlink check is the one that matters.
/// Returns `None` (having already logged why) if either check fails —
/// callers treat that as "skip the foreign-file side effects this
/// launch," not a hard error.
fn resolve_claude_md_side_paths(
    base_path: &std::path::Path,
    base_canonical: &std::path::Path,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let resolve = |relative: &str| -> Option<std::path::PathBuf> {
        let path = match crate::backend::base::safe_join_within_base(base_path, relative) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(base = %base_path.display(), relative, error = %e, "write_claude_md_respecting_ownership: path resolution failed");
                return None;
            }
        };
        if let Err(e) = crate::backend::base::verify_no_symlink_escape(&path, base_canonical) {
            tracing::warn!(path = %path.display(), error = %e, "write_claude_md_respecting_ownership: refusing to write a path that escapes the working directory via a symlink");
            return None;
        }
        Some(path)
    };
    let memory_path = resolve(AGENTMUX_MEMORY_FILENAME)?;
    let ownership_marker_path = resolve(CLAUDE_MD_OWNERSHIP_MARKER_PATH)?;
    Some((memory_path, ownership_marker_path))
}

/// Materialize `generated_content` (the composed Soul+AgentMD+Memory+Skills
/// `CLAUDE.md` body `build_config_files` already produced) against whatever
/// is already on disk at `base_path/CLAUDE.md`, WITHOUT ever overwriting a
/// foreign file's content:
///
/// - No file yet, or the existing file starts with
///   [`CLAUDE_MD_MANAGED_MARKER`]: AgentMux owns it — write `CLAUDE.md`
///   directly (marker + `generated_content`), exactly as every call site
///   did unconditionally before this function existed. **This is the one
///   path that still hard-fails on an I/O error** (propagated via `?`,
///   same as every call site's pre-existing behavior) — AgentMux's own
///   config file failing to write is a real launch-blocking problem.
/// - Existing file present WITHOUT the marker, **or present but
///   unreadable** (permission denied, non-UTF-8 content, transient I/O
///   error — a `read_to_string` error here must never collapse into "no
///   file yet," or an unreadable foreign file gets silently clobbered,
///   defeating this whole function's purpose; reagent P0 + codex P1 on
///   PR #2747): treated identically as foreign. `CLAUDE.md` itself is
///   never written to again. `generated_content` instead goes to
///   [`AGENTMUX_MEMORY_FILENAME`] (always safe to regenerate — it's 100%
///   AgentMux's own content), pulled in via a single `@import` line
///   appended to the real `CLAUDE.md` — offered at most once per working
///   directory (see [`CLAUDE_MD_OWNERSHIP_MARKER_PATH`]'s doc comment).
///   If the file is unreadable specifically, the `@import` offer is
///   skipped for this launch (nothing to safely append to/check
///   idempotency against) — retried on the next launch.
///
/// Everything in the foreign-file branch is best-effort, in the same
/// spirit as [`write_managed_skill_file_manifest`] and unlike the
/// AgentMux-owned branch above: an I/O failure writing the side file,
/// appending the import line, or writing the ownership marker is logged
/// and this function still returns `Ok(())` — losing one of these only
/// means the import line isn't offered (or the side file isn't
/// refreshed) on this one launch, not a correctness issue serious enough
/// to fail the whole agent launch (codex P1 on PR #2747: the original
/// version used `?` throughout this branch, contradicting this doc
/// comment and propagating a side-file write failure into a hard launch
/// failure).
///
/// The `@import` append is a true O_APPEND write
/// (`OpenOptions::append(true)`), never a read-modify-write of the whole
/// file — if the foreign file changes between this function's read and
/// the append (edited by the user or another process mid-launch), a
/// full-file rewrite would silently discard that intervening edit; a
/// true append can only ever add bytes at whatever the end happens to be
/// (codex P2 on PR #2747).
///
/// Every new path this function writes ([`AGENTMUX_MEMORY_FILENAME`],
/// [`CLAUDE_MD_OWNERSHIP_MARKER_PATH`]) is resolved through
/// [`crate::backend::base::safe_join_within_base`] and verified against
/// [`crate::backend::base::verify_no_symlink_escape`] before any write —
/// both constants are fixed, not user-controllable, but `.claude/` itself
/// could exist as a symlink escaping the working directory, and without
/// this check a write would silently follow it outside the selected
/// project (codex P1 on PR #2747, same defense-in-depth the other
/// config-file write loops already apply to their own paths).
///
/// First line AgentMux writes on any non-`CLAUDE.md` startup-instructions
/// file it fully owns (freshly created, or one it wrote on a prior
/// launch) — mirrors [`CLAUDE_MD_MANAGED_MARKER`]'s role for `CLAUDE.md`,
/// generic across filenames since `AGENTS.md`/`GEMINI.md`/`QWEN.md`/pi's
/// `APPEND_SYSTEM.md` don't each need distinct marker text. An HTML
/// comment renders invisibly in every markdown viewer and every one of
/// these providers reads its instructions file as plain text fed into a
/// prompt, so a leading comment line is universally harmless regardless
/// of provider — no per-provider syntax needed for the marker itself
/// (only the `@import`-equivalent side-file fallback CLAUDE.md also gets
/// would need that, and this function deliberately doesn't attempt it —
/// see the doc comment below).
const STARTUP_INSTRUCTIONS_MANAGED_MARKER: &str = "<!-- agentmux:managed-startup-instructions -->";

/// Write a NON-`CLAUDE.md` startup-instructions file (`AGENTS.md`,
/// `GEMINI.md`, `QWEN.md`, `.pi/APPEND_SYSTEM.md`, ...) WITHOUT ever
/// overwriting a pre-existing, non-AgentMux-authored file at that path.
///
/// codex P1, PR #2788: before
/// `docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md`,
/// every provider's agent got `CLAUDE.md` written regardless of provider —
/// wrong, but harmless to a real Codex/Gemini/etc. project, since AgentMux
/// was never writing to the filename that project's own real `AGENTS.md`/
/// `GEMINI.md` actually lived at. Once `build_config_files` started
/// resolving the CORRECT native filename per provider, an unconditional
/// write (the plain `std::fs::write` every other config file still uses)
/// would silently destroy a pre-existing, user-authored project file the
/// moment its name collided with the now-correctly-resolved target — a
/// real, novel data-loss regression introduced BY fixing the filename,
/// not present before.
///
/// Mirrors [`write_claude_md_respecting_ownership`]'s OWNED-vs-foreign
/// marker check (freely regenerate if AgentMux wrote it, either freshly or
/// on a prior launch; never touch it otherwise) — WITHOUT that function's
/// `@import`-line side-file fallback offer for the foreign case. That
/// fuller mechanism is deliberately Claude-Code-`@import`-syntax-specific;
/// whether `AGENTS.md`/`GEMINI.md`/`QWEN.md`/pi's `APPEND_SYSTEM.md`
/// support an equivalent include directive their own harness actually
/// honors is unverified per-provider research this spec didn't do (§5/§6
/// of the spec above). The tradeoff accepted here: if the target file is
/// foreign, this agent's Soul/AgentMD/Memory content is simply not
/// delivered via a file for this launch (logged, not silently dropped) —
/// a real capability gap, but strictly safer than overwriting a stranger's
/// file. Revisit once each provider's own include syntax is confirmed.
pub fn write_startup_instructions_respecting_existing(
    base_path: &std::path::Path,
    filename: &str,
    content: &str,
) -> std::io::Result<()> {
    let path = base_path.join(filename);

    // `None` = genuinely no file yet. `Some(Ok(content))` = read fine.
    // `Some(Err(_))` = exists but unreadable — MUST be treated the same as
    // "foreign," never as "no file yet" (same reasoning as
    // write_claude_md_respecting_ownership's own doc comment).
    let existing = match std::fs::read_to_string(&path) {
        Ok(content) => Some(Ok(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => Some(Err(e)),
    };
    let agentmux_owns_it =
        matches!(&existing, Some(Ok(content)) if content.starts_with(STARTUP_INSTRUCTIONS_MANAGED_MARKER));

    if agentmux_owns_it || existing.is_none() {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let marked_content = format!("{STARTUP_INSTRUCTIONS_MANAGED_MARKER}\n\n{content}");
        return std::fs::write(&path, marked_content);
    }

    tracing::warn!(
        path = %path.display(),
        "write_startup_instructions_respecting_existing: pre-existing, \
         non-AgentMux-authored file — leaving it untouched; this agent's \
         Global Memory/Soul/AgentMD content is not delivered via this file \
         for this launch"
    );
    Ok(())
}

/// Shared by `agent.open` (`server/app_api/agent_open.rs`) and the
/// `WriteAgentConfig` "click Launch" path (`server/editor_handlers.rs`) —
/// per this module's own doc comment, the two config-materializing call
/// sites that must not drift out of sync. See
/// `docs/specs/SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md`.
pub fn write_claude_md_respecting_ownership(
    base_path: &std::path::Path,
    generated_content: &str,
) -> std::io::Result<()> {
    let claude_md_path = base_path.join("CLAUDE.md");

    // `None` = genuinely no file yet. `Some(Ok(content))` = read fine.
    // `Some(Err(_))` = exists but unreadable — MUST be treated the same
    // as "foreign," never as "no file yet" (see doc comment above).
    let existing = match std::fs::read_to_string(&claude_md_path) {
        Ok(content) => Some(Ok(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => Some(Err(e)),
    };

    let agentmux_owns_it =
        matches!(&existing, Some(Ok(content)) if content.starts_with(CLAUDE_MD_MANAGED_MARKER));

    // Known, accepted TOCTOU window (codex P2, third review round on
    // PR #2747): if a foreign CLAUDE.md is created/swapped in between the
    // read above and this write — e.g. this exact working directory
    // becoming a real project mid-launch — that unconditional write would
    // still clobber it once. Not closed here: doing so would need real
    // file locking (flock/LockFile) across the read-decide-write sequence,
    // which every other config file this module writes (.mcp.json, skill
    // files, hooks.json) has the identical unaddressed race against
    // (agent_open.rs's own "no collision resolution... overwrites
    // whatever's there" comment, about a directory-level version of the
    // same class of race). Singling out CLAUDE.md's OWNED-file fast path
    // for stronger protection than every sibling write in this same
    // function would be inconsistent scope for what this PR set out to
    // fix (a stable, at-rest foreign file being clobbered on every
    // ordinary launch) — narrower and far less likely than that. The
    // foreign-file branch below (where this PR's actual guarantee lives)
    // does not have this gap: it never writes CLAUDE.md's own content.
    if agentmux_owns_it || existing.is_none() {
        let content = format!("{CLAUDE_MD_MANAGED_MARKER}\n\n{generated_content}");
        return std::fs::write(&claude_md_path, content);
    }

    // Resolve + symlink-verify the two new paths once, up front — neither
    // write below proceeds if this fails.
    let base_canonical = match base_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(path = %base_path.display(), error = %e, "write_claude_md_respecting_ownership: failed to canonicalize base_path; skipping side-file write this launch");
            return Ok(());
        }
    };
    let Some((memory_path, ownership_marker_path)) =
        resolve_claude_md_side_paths(base_path, &base_canonical)
    else {
        return Ok(());
    };

    // Foreign (or unreadable) file — CLAUDE.md's own content is never
    // written to from here on. Everything from here down is best-effort
    // (see doc comment): a failure must not fail the whole agent launch.
    if let Some(parent) = memory_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(path = %parent.display(), error = %e, "write_claude_md_respecting_ownership: failed to create .claude/; side file not written this launch");
            return Ok(());
        }
    }
    if let Err(e) = std::fs::write(&memory_path, generated_content) {
        tracing::warn!(path = %memory_path.display(), error = %e, "write_claude_md_respecting_ownership: failed to write AGENTMUX_MEMORY.md this launch");
        return Ok(());
    }

    // Unreadable (not just "no marker") — nothing safe to check
    // idempotency against or append to. Skip the @import offer this
    // launch; the side file above is still fresh and ready whenever the
    // file becomes readable again.
    let Some(Ok(existing_content)) = existing else {
        tracing::warn!(path = %claude_md_path.display(), "write_claude_md_respecting_ownership: existing CLAUDE.md unreadable; treating as foreign, skipping @import offer this launch");
        return Ok(());
    };

    if let Some(parent) = ownership_marker_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(path = %parent.display(), error = %e, "write_claude_md_respecting_ownership: failed to create dir for ownership marker");
            return Ok(());
        }
    }

    // Atomic "am I the first to offer this" gate, not a read-then-write
    // check: `create_new` fails with `AlreadyExists` if another
    // concurrent call already won this race. Two agents sharing a
    // working directory (agent_open.rs's shared-workdir fallback,
    // `~/.agentmux/agents/<slug>`, launching concurrently) previously
    // could both read "not yet offered" and both append the import
    // line, duplicating it — the only existing serialization
    // (`agent_open_lock`) is keyed by agent_id, not by working
    // directory, so it doesn't cover this case (reagent P2, second
    // review round on PR #2747). Only the caller whose `create_new`
    // succeeds proceeds to append; every other caller — including a
    // genuine concurrent racer, and every later launch once the marker
    // exists — sees `AlreadyExists` and skips straight past.
    let marker_json = serde_json::to_string(&ClaudeMdOwnershipMarker { import_line_offered: true })
        .unwrap_or_default();
    let create_result = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&ownership_marker_path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, marker_json.as_bytes()));

    match create_result {
        Ok(()) => {
            let import_needle = format!("@{AGENTMUX_MEMORY_FILENAME}");
            // Idempotent even having won the race: don't duplicate if
            // the import somehow already appears (e.g. a user copied it
            // in by hand before AgentMux ever ran here).
            if !existing_content.contains(&import_needle) {
                let import_block =
                    format!("\n\n{CLAUDE_MD_IMPORT_MARKER_COMMENT}\n{import_needle}\n");
                // True append (never a read-modify-write of the whole
                // file) — an edit racing this function's earlier read is
                // preserved, not silently discarded (codex P2 on
                // PR #2747).
                let append_result = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&claude_md_path)
                    .and_then(|mut f| std::io::Write::write_all(&mut f, import_block.as_bytes()));
                if let Err(e) = append_result {
                    tracing::warn!(path = %claude_md_path.display(), error = %e, "write_claude_md_respecting_ownership: failed to append the @import line this launch");
                    // Roll back the marker we just created — winning the
                    // race doesn't mean the offer actually completed. Without
                    // this, the marker alone would permanently record
                    // "offered" even though the import line was never
                    // added, and every later launch's create_new would hit
                    // AlreadyExists and skip forever, with no retry path
                    // (reagent P1 + codex, third review round on PR #2747).
                    // Best-effort: if the removal itself fails, a future
                    // launch just stays stuck the way it would have been
                    // without this fix — not worse, and not worth
                    // escalating a cleanup failure into a launch failure.
                    let _ = std::fs::remove_file(&ownership_marker_path);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Already offered — by us on a prior launch, or a concurrent
            // racer that won just now. Nothing to do.
        }
        Err(e) => {
            tracing::warn!(path = %ownership_marker_path.display(), error = %e, "write_claude_md_respecting_ownership: failed to create ownership marker");
        }
    }

    Ok(())
}

/// Best-effort: mint-or-reuse `agent_slug`'s jekt/LAN signing keys
/// (`SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md` §2.2,
/// `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` §2.1) and patch them into a
/// `.mcp.json` file's `mcpServers.agentmux.env` block. Returns `None` on
/// any parse failure, missing `mcpServers.agentmux.env` object, or if
/// neither key could be ensured — callers must keep the original content
/// in that case; this must never block writing the agent's config.
///
/// Shared by `agent.open` (`server/app_api/agent_open.rs`) and the
/// `WriteAgentConfig` "click Launch" path (`server/editor_handlers.rs`) —
/// per this module's own doc comment, the two `.mcp.json`-materializing
/// call sites that must not drift out of sync. Before this function
/// existed, only `agent.open` injected these keys — `WriteAgentConfig`
/// (the path a normal Launch-button click actually goes through) silently
/// never did, so ordinarily-launched agents' jekts rendered
/// `TRUST=self-declared` even on builds well past both features shipping.
/// See `docs/specs/REPORT_JEKT_SIGNING_KEY_INJECTION_GAP_2026_08_16.md`.
pub fn inject_jekt_signing_keys_into_mcp_json(
    content: &str,
    wstore: &crate::backend::storage::store::Store,
    agent_slug: &str,
) -> Option<String> {
    let mut mcp_json: Value = serde_json::from_str(content).ok()?;
    let env = mcp_json
        .pointer_mut("/mcpServers/agentmux/env")
        .and_then(|v| v.as_object_mut())?;

    let mut patched = false;
    if let Ok(key) = wstore.agent_jekt_key_ensure(agent_slug) {
        use base64::Engine as _;
        let key_b64 = base64::engine::general_purpose::STANDARD.encode(&key);
        env.insert("AGENTMUX_JEKT_KEY".to_string(), json!(key_b64));
        patched = true;
    }
    if let Ok(keypair) = wstore.agent_lan_key_ensure(agent_slug) {
        env.insert("AGENTMUX_LAN_KEY".to_string(), json!(keypair.private_key));
        patched = true;
    }
    if !patched {
        return None;
    }
    serde_json::to_string_pretty(&mcp_json).ok()
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill(name: &str, trigger: &str, description: &str, content: &str) -> AgentSkill {
        AgentSkill {
            id: format!("skill-{}", trigger),
            agent_id: "agent-1".to_string(),
            name: name.to_string(),
            trigger: trigger.to_string(),
            skill_type: "prompt".to_string(),
            description: description.to_string(),
            content: content.to_string(),
            created_at: 0,
        }
    }

    fn make_agent_skill(name: &str, description: &str, content: &str) -> AgentSkill {
        AgentSkill {
            id: format!("skill-{}", derive_slug(name)),
            agent_id: "agent-1".to_string(),
            name: name.to_string(),
            trigger: String::new(),
            skill_type: SKILL_TYPE_AGENT_SKILL.to_string(),
            description: description.to_string(),
            content: content.to_string(),
            created_at: 0,
        }
    }

    #[test]
    fn test_sanitize_trigger_rejects_path_traversal() {
        // reagent P1, PR #2322: a trigger with a path separator or ".."
        // must never be allowed to steer .claude/commands/<trigger>.md
        // outside the working directory.
        assert_eq!(sanitize_trigger("../../../../.ssh/authorized_keys"), None);
        assert_eq!(sanitize_trigger("../evil"), None);
        assert_eq!(sanitize_trigger("sub/evil"), None);
        assert_eq!(sanitize_trigger("sub\\evil"), None);
        assert_eq!(sanitize_trigger(".."), None);
        assert_eq!(sanitize_trigger("."), None);
        assert_eq!(sanitize_trigger(""), None);
        assert_eq!(sanitize_trigger("deploy"), Some("deploy"));
    }

    #[test]
    fn test_build_config_files_skips_prompt_skill_with_traversal_trigger() {
        // reagent P1, PR #2322: build_config_files must not materialize a
        // command file for a malicious trigger, not even under a sanitized
        // name -- skip it outright.
        let content_map = HashMap::new();
        let skills = vec![make_skill(
            "Evil",
            "../../../../.ssh/authorized_keys",
            "desc",
            "malicious content",
        )];
        let files = build_config_files(&content_map, &skills, "Aria", "agent-1", "aria", "/tmp/aria", "claude");
        assert!(
            files.iter().all(|f| !f.filename.contains("..")),
            "no config file path may contain '..': {:?}",
            files.iter().map(|f| &f.filename).collect::<Vec<_>>()
        );
        assert!(!files.iter().any(|f| f.filename.starts_with(".claude/commands/")));
    }

    #[test]
    fn test_expand_template_basic() {
        let mut vars = HashMap::new();
        vars.insert("AGENT".to_string(), "Aria".to_string());
        vars.insert("DATE".to_string(), "2026-04-10".to_string());

        let out = expand_template("Hello {{AGENT}}, today is {{DATE}}.", &vars);
        assert_eq!(out, "Hello Aria, today is 2026-04-10.");
    }

    #[test]
    fn test_expand_template_unknown_placeholder_preserved() {
        let vars = HashMap::new();
        let out = expand_template("Value: {{UNKNOWN}}", &vars);
        assert_eq!(out, "Value: {{UNKNOWN}}");
    }

    #[test]
    fn test_expand_template_empty_vars() {
        let vars = HashMap::new();
        let out = expand_template("No placeholders here.", &vars);
        assert_eq!(out, "No placeholders here.");
    }

    #[test]
    fn test_build_mcp_config_no_user_content() {
        let result = build_mcp_config(None, "aria", "bus-42").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let servers = &parsed["mcpServers"];
        assert!(servers["agentmux"].is_object());
        assert_eq!(servers["agentmux"]["command"], "agentmux-mcp");
        assert_eq!(servers["agentmux"]["env"]["AGENTMUX_AGENT_ID"], "aria");
        assert_eq!(servers["agentmux"]["env"]["AGENTMUX_AGENT_BUS_ID"], "bus-42");
    }

    #[test]
    fn test_build_mcp_config_merges_user_servers() {
        let user_mcp = r#"{"mcpServers": {"mytool": {"type": "stdio", "command": "mytool"}}}"#;
        let result = build_mcp_config(Some(user_mcp), "aria", "").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let servers = &parsed["mcpServers"];
        assert!(servers["agentmux"].is_object());
        assert!(servers["mytool"].is_object());
    }

    #[test]
    fn test_build_mcp_config_invalid_user_json_uses_auto_injected() {
        let result = build_mcp_config(Some("not json {{"), "aria", "").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["mcpServers"]["agentmux"].is_object());
    }

    #[test]
    fn test_build_config_files_claude_md_assembled() {
        let mut content_map = HashMap::new();
        content_map.insert("soul".to_string(), "You are {{AGENT}}.".to_string());
        content_map.insert("agentmd".to_string(), "## Instructions\nDo stuff.".to_string());

        let files = build_config_files(&content_map, &[], "Aria", "agent-1", "aria", "/tmp/aria", "claude");
        let claude_md = files.iter().find(|f| f.filename == "CLAUDE.md").unwrap();
        assert!(claude_md.content.contains("You are Aria."));
        assert!(claude_md.content.contains("---"));
        assert!(claude_md.content.contains("## Instructions"));
    }

    // docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §7:
    // per-provider filename resolution, pinned against §2's researched
    // table so it can't silently drift.
    #[test]
    fn test_build_config_files_resolves_provider_specific_filename() {
        let cases: &[(&str, &str)] = &[
            ("claude", "CLAUDE.md"),
            ("codex", "AGENTS.md"),
            ("gemini", "GEMINI.md"),
            ("qwen", "QWEN.md"),
            ("copilot", "AGENTS.md"),
            ("openclaw", "AGENTS.md"),
            ("pi", ".pi/APPEND_SYSTEM.md"),
            ("antigravity", "GEMINI.md"),
            ("muxcode", "CLAUDE.md"),
        ];
        for (provider_id, expected_filename) in cases {
            let mut content_map = HashMap::new();
            content_map.insert("soul".to_string(), "You are Aria.".to_string());
            let files = build_config_files(&content_map, &[], "Aria", "agent-1", "aria", "/tmp/aria", provider_id);
            let instructions_files: Vec<&str> = files
                .iter()
                .filter(|f| f.filename == *expected_filename)
                .map(|f| f.content.as_str())
                .collect();
            assert_eq!(
                instructions_files.len(),
                1,
                "provider '{provider_id}' should produce exactly one file named '{expected_filename}': {:?}",
                files.iter().map(|f| &f.filename).collect::<Vec<_>>()
            );
            assert!(instructions_files[0].contains("You are Aria."));
        }
    }

    #[test]
    fn test_build_config_files_kimi_gets_no_instructions_file() {
        // Confirmed absence (SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md
        // §2): Kimi has no native file-based startup-instructions
        // discovery — writing one would be inert output nobody reads.
        let mut content_map = HashMap::new();
        content_map.insert("soul".to_string(), "You are Aria.".to_string());
        content_map.insert("agentmd".to_string(), "## Instructions".to_string());
        content_map.insert("memory".to_string(), "Some memory content".to_string());
        let files = build_config_files(&content_map, &[], "Aria", "agent-1", "aria", "/tmp/aria", "kimi");
        assert!(
            files.iter().all(|f| !f.filename.ends_with(".md") || f.filename.starts_with(".claude/")),
            "kimi must get no top-level instructions file, even with soul/agentmd/memory content present: {:?}",
            files.iter().map(|f| &f.filename).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_build_config_files_unrecognized_provider_gets_no_instructions_file() {
        // An unrecognized provider_id (typo, not-yet-registered) must not
        // silently fall back to "CLAUDE.md" — same no-op path as kimi.
        // .claude/settings.json and .mcp.json are still written (they're
        // provider-unconditional today, unaffected by this spec — §5's
        // ".claude/-namespaced files... out of scope"), so this checks
        // specifically for the absence of any known instructions filename,
        // not an empty result.
        let mut content_map = HashMap::new();
        content_map.insert("soul".to_string(), "You are Aria.".to_string());
        let files = build_config_files(&content_map, &[], "Aria", "agent-1", "aria", "/tmp/aria", "not-a-real-provider");
        let known_instructions_filenames = [
            "CLAUDE.md", "AGENTS.md", "GEMINI.md", "QWEN.md", ".pi/APPEND_SYSTEM.md",
        ];
        assert!(
            !files.iter().any(|f| known_instructions_filenames.contains(&f.filename.as_str())),
            "no instructions file should be written for an unrecognized provider: {:?}",
            files.iter().map(|f| &f.filename).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_build_config_files_skills_index_and_commands() {
        let content_map = HashMap::new();
        let skills = vec![
            make_skill("Deploy", "deploy", "Deploy the app", "Run: deploy all"),
            make_skill("Test", "test", "Run tests", "Run: test suite"),
        ];

        let files = build_config_files(&content_map, &skills, "Aria", "agent-1", "aria", "/tmp/aria", "claude");

        // CLAUDE.md should have the skills index
        let claude_md = files.iter().find(|f| f.filename == "CLAUDE.md").unwrap();
        assert!(claude_md.content.contains("Available Skills"));
        assert!(claude_md.content.contains("/deploy"));
        assert!(claude_md.content.contains("/test"));

        // Individual skill command files
        assert!(files.iter().any(|f| f.filename == ".claude/commands/deploy.md"));
        assert!(files.iter().any(|f| f.filename == ".claude/commands/test.md"));
    }

    #[test]
    fn test_build_config_files_agent_skill_format_writes_skill_md() {
        let content_map = HashMap::new();
        let skills = vec![make_agent_skill(
            "Deploy Checklist",
            "Runs the pre-deploy checklist",
            "1. Run tests\n2. Check migrations\n3. Deploy",
        )];

        let files = build_config_files(&content_map, &skills, "Aria", "agent-1", "aria", "/tmp/aria", "claude");

        // Materializes to .claude/skills/<slug>/SKILL.md, not .claude/commands/
        let skill_file = files
            .iter()
            .find(|f| f.filename == ".claude/skills/deploy-checklist/SKILL.md")
            .expect("expected .claude/skills/deploy-checklist/SKILL.md");
        assert!(!files.iter().any(|f| f.filename.starts_with(".claude/commands/")));

        // YAML frontmatter with the two required Agent Skills fields.
        // `name` is the slug (matching its parent directory per the Agent
        // Skills spec), NOT the raw display name -- reagent P1 on #2322.
        assert!(skill_file.content.starts_with("---\n"));
        assert!(skill_file.content.contains("name: \"deploy-checklist\""));
        assert!(skill_file
            .content
            .contains("description: \"Runs the pre-deploy checklist\""));
        assert!(skill_file.content.contains("---\n\n1. Run tests"));

        // Skills index in CLAUDE.md still lists it (trigger-agnostic)
        let claude_md = files.iter().find(|f| f.filename == "CLAUDE.md").unwrap();
        assert!(claude_md.content.contains("Deploy Checklist"));
    }

    #[test]
    fn test_build_config_files_agent_skill_format_escapes_yaml_special_chars() {
        // Names/descriptions with colons, quotes, or newlines must not
        // corrupt the YAML frontmatter -- serde_json's escaping (valid YAML
        // double-quoted-scalar syntax per YAML 1.2 §7.3.1) is what protects
        // this; regression-test it explicitly rather than trusting the
        // dependency silently.
        let content_map = HashMap::new();
        let skills = vec![make_agent_skill(
            "Weird: \"Name\"",
            "Has a colon: and \"quotes\"",
            "body",
        )];

        let files = build_config_files(&content_map, &skills, "Aria", "agent-1", "aria", "/tmp/aria", "claude");
        let skill_file = files
            .iter()
            .find(|f| f.filename.starts_with(".claude/skills/") && f.filename.ends_with("SKILL.md"))
            .expect("expected a SKILL.md file");

        // Frontmatter must parse as exactly 3 lines before the closing ---
        // (name, description, and nothing else leaking onto a new line).
        let end = skill_file.content.find("\n---\n\n").expect("closing frontmatter delimiter");
        let frontmatter = &skill_file.content[4..end]; // skip leading "---\n"
        let lines: Vec<&str> = frontmatter.lines().collect();
        assert_eq!(lines.len(), 2, "frontmatter should be exactly name + description lines, got: {lines:?}");
    }

    #[test]
    fn test_build_config_files_agent_skill_format_name_is_the_slug_not_display_name() {
        // reagent P1 (PR #2322): the Agent Skills spec requires `name` be
        // lowercase/hyphenated and match its parent directory -- the raw
        // display name (e.g. "Deploy Checklist") is spec-invalid.
        let content_map = HashMap::new();
        let skills = vec![make_agent_skill("Deploy Checklist", "desc", "body")];
        let files = build_config_files(&content_map, &skills, "Aria", "agent-1", "aria", "/tmp/aria", "claude");
        let skill_file = files.iter().find(|f| f.filename.ends_with("SKILL.md")).unwrap();
        assert!(skill_file.content.contains("name: \"deploy-checklist\""));
        assert!(!skill_file.content.contains("name: \"Deploy Checklist\""));
    }

    #[test]
    fn test_build_config_files_agent_skill_format_empty_description_gets_fallback() {
        // reagent P1 (PR #2322): the Agent Skills spec requires a non-empty
        // description; the UI permits creating a skill with none.
        let content_map = HashMap::new();
        let skills = vec![make_agent_skill("Deploy Checklist", "", "body")];
        let files = build_config_files(&content_map, &skills, "Aria", "agent-1", "aria", "/tmp/aria", "claude");
        let skill_file = files.iter().find(|f| f.filename.ends_with("SKILL.md")).unwrap();
        assert!(!skill_file.content.contains("description: \"\""), "empty description must not reach the spec-invalid empty string: {}", skill_file.content);
        assert!(skill_file.content.contains("description: \"No description provided.\""));
    }

    #[test]
    fn test_build_config_files_agent_skill_format_truncates_long_description() {
        // reagent P1 (PR #2322): the Agent Skills spec caps description at
        // 1024 characters.
        let content_map = HashMap::new();
        let long_description = "x".repeat(2000);
        let skills = vec![make_agent_skill("Deploy Checklist", &long_description, "body")];
        let files = build_config_files(&content_map, &skills, "Aria", "agent-1", "aria", "/tmp/aria", "claude");
        let skill_file = files.iter().find(|f| f.filename.ends_with("SKILL.md")).unwrap();
        // Extract the description value between the quotes on its line.
        let desc_line = skill_file.content.lines().find(|l| l.starts_with("description: ")).unwrap();
        let quoted = desc_line.trim_start_matches("description: ");
        let inner_len = quoted.len() - 2; // strip surrounding quotes
        assert!(inner_len <= 1024, "description exceeds spec max of 1024 chars: {inner_len}");
    }

    #[test]
    fn test_build_config_files_agent_skill_format_dedupes_colliding_slugs() {
        // reagent P1 (PR #2322): two distinct skill names that derive_slug
        // collapses to the same slug must NOT silently overwrite each
        // other's SKILL.md file.
        let content_map = HashMap::new();
        let skills = vec![
            make_agent_skill("Deploy Checklist", "First skill", "body one"),
            make_agent_skill("Deploy!!!Checklist", "Second skill", "body two"),
            make_agent_skill("Deploy   Checklist", "Third skill", "body three"),
        ];

        let files = build_config_files(&content_map, &skills, "Aria", "agent-1", "aria", "/tmp/aria", "claude");

        let skill_files: Vec<&AgentConfigFile> = files
            .iter()
            .filter(|f| f.filename.starts_with(".claude/skills/") && f.filename.ends_with("SKILL.md"))
            .collect();
        assert_eq!(skill_files.len(), 3, "all three skills must materialize to distinct files");

        let filenames: HashSet<&str> = skill_files.iter().map(|f| f.filename.as_str()).collect();
        assert_eq!(filenames.len(), 3, "filenames must be unique: {filenames:?}");
        assert!(filenames.contains(".claude/skills/deploy-checklist/SKILL.md"));
        assert!(filenames.contains(".claude/skills/deploy-checklist-2/SKILL.md"));
        assert!(filenames.contains(".claude/skills/deploy-checklist-3/SKILL.md"));

        // Each file's content must correspond to the correct skill (not just
        // present -- verify no cross-contamination from the dedup logic).
        let first = skill_files.iter().find(|f| f.filename.ends_with("deploy-checklist/SKILL.md")).unwrap();
        assert!(first.content.contains("body one"));
        let second = skill_files.iter().find(|f| f.filename.ends_with("deploy-checklist-2/SKILL.md")).unwrap();
        assert!(second.content.contains("body two"));
        let third = skill_files.iter().find(|f| f.filename.ends_with("deploy-checklist-3/SKILL.md")).unwrap();
        assert!(third.content.contains("body three"));
    }

    #[test]
    fn test_unique_skill_slug_replaces_underscores_with_hyphens() {
        // Codex P1, PR #2322: derive_slug (shared with agent role-slugs) keeps
        // underscores, which is spec-invalid for an Agent Skills `name`.
        let mut used = HashSet::new();
        assert_eq!(unique_skill_slug("code_review", &mut used), "code-review");
    }

    #[test]
    fn test_unique_skill_slug_suffixed_slug_stays_within_64_chars() {
        // Codex P2, PR #2322: a 64-char base plus "-2" was previously 66 chars.
        let mut used = HashSet::new();
        let long = "a".repeat(100);
        let first = unique_skill_slug(&long, &mut used);
        assert_eq!(first.len(), 64);
        let second = unique_skill_slug(&long, &mut used);
        assert!(second.len() <= 64, "suffixed slug exceeds 64 chars: {second} ({})", second.len());
        assert!(second.ends_with("-2"));
    }

    #[test]
    fn test_render_skill_md_truncates_by_utf16_units_matching_ts_slice() {
        // reagent P2, PR #2322: Rust previously capped by byte length while
        // the TS mirror caps by UTF-16 code units (`.slice(0, 1024)`), so
        // non-ASCII descriptions truncated to different lengths between the
        // two paths. A 3-byte-per-char UTF-8 string (each char = 1 UTF-16
        // unit) makes the byte-vs-unit divergence obvious: byte-based
        // truncation would cut this off around char 341, not 1024.
        let description: String = "\u{4e2d}".repeat(2000); // "中" x2000 (3 bytes each, 1 UTF-16 unit each)
        let md = render_skill_md("slug", &description, "body");
        let desc_line = md.lines().find(|l| l.starts_with("description: ")).unwrap();
        let quoted = desc_line.trim_start_matches("description: ");
        let inner = &quoted[1..quoted.len() - 1]; // strip surrounding quotes
        let utf16_len: usize = inner.chars().map(|c| c.len_utf16()).sum();
        assert_eq!(utf16_len, 1024, "expected exactly 1024 UTF-16 units, got {utf16_len}");
    }

    #[test]
    fn test_build_config_files_settings_merges_user_hooks() {
        // PR #813 moved hooks from `.claude/hooks.json` (a Claude Code
        // dead-letter path) to `.claude/settings.json` under the
        // `"hooks"` key. This test exercises the merge path: user
        // PreToolUse entries must be PREPENDED (not silently
        // dropped) to the auto-injected bashwrap entry so streaming
        // stays on while user-supplied gates fire first.
        let mut content_map = HashMap::new();
        content_map.insert(
            "hooks".to_string(),
            r#"{"PreToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"my-audit"}]}]}"#
                .to_string(),
        );
        let files = build_config_files(&content_map, &[], "Aria", "agent-1", "aria", "/tmp/aria", "claude");
        let settings = files
            .iter()
            .find(|f| f.filename == ".claude/settings.json")
            .expect("settings.json emitted");
        let parsed: Value = serde_json::from_str(&settings.content).unwrap();
        let pre_tool_use = parsed["hooks"]["PreToolUse"]
            .as_array()
            .expect("PreToolUse is an array");
        // User's "Read" matcher prepended first, then our Bash matcher.
        assert!(
            pre_tool_use
                .iter()
                .any(|e| e["matcher"].as_str() == Some("Read")),
            "user-supplied PreToolUse:Read must survive the merge"
        );
        assert!(
            pre_tool_use
                .iter()
                .any(|e| e["matcher"].as_str().unwrap_or("").contains("Bash")),
            "auto-injected PreToolUse:Bash must still be present"
        );
    }

    #[test]
    fn test_build_config_files_settings_merges_user_precompact_hooks() {
        // SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.2:
        // PreCompact becomes auto-injected (two entries, matcher
        // "manual"/"auto") the same way PreToolUse already is. This is
        // the exact regression class PR #813's PreToolUse fix guards
        // against — a user-supplied PreCompact entry (from the legacy
        // content_map["hooks"] path) must survive the merge, not get
        // silently and permanently dropped by the generic
        // `hooks_obj.entry(k).or_insert(v)` path once PreCompact stops
        // being "just another key".
        let mut content_map = HashMap::new();
        content_map.insert(
            "hooks".to_string(),
            r#"{"PreCompact":[{"matcher":"manual","hooks":[{"type":"command","command":"my-precompact-audit"}]}]}"#
                .to_string(),
        );
        let files = build_config_files(&content_map, &[], "Aria", "agent-1", "aria", "/tmp/aria", "claude");
        let settings = files
            .iter()
            .find(|f| f.filename == ".claude/settings.json")
            .expect("settings.json emitted");
        let parsed: Value = serde_json::from_str(&settings.content).unwrap();
        let pre_compact = parsed["hooks"]["PreCompact"]
            .as_array()
            .expect("PreCompact is an array");
        assert_eq!(
            pre_compact.len(),
            3,
            "expected user entry + 2 auto-injected (manual, auto) entries, got {pre_compact:?}"
        );
        assert!(
            pre_compact
                .iter()
                .any(|e| e["hooks"][0]["command"].as_str() == Some("my-precompact-audit")),
            "user-supplied PreCompact entry must survive the merge"
        );
        assert!(
            pre_compact.iter().any(|e| e["hooks"][0]["command"].as_str()
                == Some("agentmux-bashwrap precompact --trigger=manual")),
            "auto-injected PreCompact manual entry must still be present"
        );
        assert!(
            pre_compact.iter().any(|e| e["hooks"][0]["command"].as_str()
                == Some("agentmux-bashwrap precompact --trigger=auto")),
            "auto-injected PreCompact auto entry must still be present"
        );
    }

    #[test]
    fn test_build_config_files_settings_json_merges_user_precompact_hooks() {
        // Same regression as above, but exercising the OTHER merge
        // branch: a user-supplied PreCompact entry living in
        // settings.json's own `"hooks"` key (content_map["settings"])
        // rather than the legacy content_map["hooks"] path. Both
        // branches in `build_settings_with_hooks` needed the fix.
        let mut content_map = HashMap::new();
        content_map.insert(
            "settings".to_string(),
            r#"{"hooks":{"PreCompact":[{"matcher":"auto","hooks":[{"type":"command","command":"my-settings-precompact"}]}]}}"#
                .to_string(),
        );
        let files = build_config_files(&content_map, &[], "Aria", "agent-1", "aria", "/tmp/aria", "claude");
        let settings = files
            .iter()
            .find(|f| f.filename == ".claude/settings.json")
            .expect("settings.json emitted");
        let parsed: Value = serde_json::from_str(&settings.content).unwrap();
        let pre_compact = parsed["hooks"]["PreCompact"]
            .as_array()
            .expect("PreCompact is an array");
        assert!(
            pre_compact
                .iter()
                .any(|e| e["hooks"][0]["command"].as_str() == Some("my-settings-precompact")),
            "user-supplied settings.json PreCompact entry must survive the merge"
        );
        assert!(
            pre_compact.iter().any(|e| e["hooks"][0]["command"].as_str()
                == Some("agentmux-bashwrap precompact --trigger=manual")),
            "auto-injected PreCompact manual entry must still be present"
        );
        assert!(
            pre_compact.iter().any(|e| e["hooks"][0]["command"].as_str()
                == Some("agentmux-bashwrap precompact --trigger=auto")),
            "auto-injected PreCompact auto entry must still be present"
        );
        // Settings-level user entries PREPEND before AgentMux's own, per
        // this merge path's ordering rule (mirrors PreToolUse).
        assert_eq!(
            pre_compact[0]["hooks"][0]["command"].as_str(),
            Some("my-settings-precompact"),
            "user PreCompact entries must prepend before AgentMux's own"
        );
    }

    #[test]
    fn test_build_config_files_mcp_written() {
        let content_map = HashMap::new();
        let files = build_config_files(&content_map, &[], "Aria", "agent-1", "aria", "/tmp/aria", "claude");
        let mcp = files.iter().find(|f| f.filename == ".mcp.json").unwrap();
        let parsed: Value = serde_json::from_str(&mcp.content).unwrap();
        assert!(parsed["mcpServers"]["agentmux"].is_object());
    }

    #[test]
    fn test_build_config_files_expands_agent_slug_and_working_dir() {
        // REPORT_REPO_HEALTH_AUDIT_2026_07_20.md §1.3: AGENT_SLUG/WORKING_DIR
        // were missing from build_config_files's template vars (present in
        // the TS mirror, agent-model.ts:747-748), so any soul/agentmd content
        // using {{AGENT_SLUG}} or {{WORKING_DIR}} was left unexpanded.
        let mut content_map = HashMap::new();
        content_map.insert(
            "soul".to_string(),
            "I am {{AGENT_SLUG}}, working in {{WORKING_DIR}}.".to_string(),
        );
        let files = build_config_files(
            &content_map,
            &[],
            "Aria",
            "agent-1",
            "aria",
            "/home/user/my-project",
            "claude",
        );
        let claude_md = files.iter().find(|f| f.filename == "CLAUDE.md").unwrap();
        assert!(claude_md.content.contains("I am aria, working in /home/user/my-project."));
    }

    // docs/specs/REPORT_JEKT_SIGNING_KEY_INJECTION_GAP_2026_08_16.md: this
    // function exists specifically so `agent.open` and `WriteAgentConfig`
    // (the actual "click Launch" path) can't drift out of sync on jekt/LAN
    // key injection the way they did before this fix.
    #[test]
    fn inject_jekt_signing_keys_into_mcp_json_patches_both_keys_into_the_env_block() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let content = serde_json::to_string(&json!({
            "mcpServers": { "agentmux": { "type": "stdio", "command": "agentmux-mcp", "env": { "AGENTMUX_AGENT_ID": "aria" } } }
        }))
        .unwrap();

        let rewritten = inject_jekt_signing_keys_into_mcp_json(&content, &store, "aria")
            .expect("both keys should ensure successfully against a fresh in-memory store");
        let parsed: Value = serde_json::from_str(&rewritten).unwrap();
        let env = &parsed["mcpServers"]["agentmux"]["env"];
        assert_eq!(env["AGENTMUX_AGENT_ID"], "aria", "existing fields must survive the patch");
        assert!(env["AGENTMUX_JEKT_KEY"].is_string() && !env["AGENTMUX_JEKT_KEY"].as_str().unwrap().is_empty());
        assert!(env["AGENTMUX_LAN_KEY"].is_string() && !env["AGENTMUX_LAN_KEY"].as_str().unwrap().is_empty());
    }

    #[test]
    fn inject_jekt_signing_keys_into_mcp_json_reuses_the_same_key_on_a_second_call() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let content = serde_json::to_string(&json!({
            "mcpServers": { "agentmux": { "env": { "AGENTMUX_AGENT_ID": "aria" } } }
        }))
        .unwrap();

        let first = inject_jekt_signing_keys_into_mcp_json(&content, &store, "aria").unwrap();
        let second = inject_jekt_signing_keys_into_mcp_json(&content, &store, "aria").unwrap();
        let first_key = serde_json::from_str::<Value>(&first).unwrap()["mcpServers"]["agentmux"]["env"]["AGENTMUX_JEKT_KEY"].clone();
        let second_key = serde_json::from_str::<Value>(&second).unwrap()["mcpServers"]["agentmux"]["env"]["AGENTMUX_JEKT_KEY"].clone();
        assert_eq!(first_key, second_key, "minted-on-first-use key must be reused, not re-minted, on every materialization");
    }

    #[test]
    fn inject_jekt_signing_keys_into_mcp_json_returns_none_when_theres_no_env_object_to_patch() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        // No mcpServers.agentmux.env at all -- nothing to patch into.
        let content = serde_json::to_string(&json!({"hello": "world"})).unwrap();
        assert!(inject_jekt_signing_keys_into_mcp_json(&content, &store, "aria").is_none());
    }

    #[test]
    fn inject_jekt_signing_keys_into_mcp_json_returns_none_on_malformed_json() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        assert!(inject_jekt_signing_keys_into_mcp_json("not json", &store, "aria").is_none());
    }

    // ============================================================
    // write_startup_instructions_respecting_existing
    // (SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md — codex P1, PR #2788)
    // ============================================================

    #[test]
    fn startup_instructions_fresh_working_dir_writes_directly_with_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        write_startup_instructions_respecting_existing(dir.path(), "AGENTS.md", "Soul + AgentMD content")
            .unwrap();

        let content = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.starts_with(STARTUP_INSTRUCTIONS_MANAGED_MARKER));
        assert!(content.contains("Soul + AgentMD content"));
    }

    #[test]
    fn startup_instructions_agentmux_owned_file_is_freely_regenerated() {
        let dir = tempfile::tempdir().unwrap();
        write_startup_instructions_respecting_existing(dir.path(), "QWEN.md", "first version").unwrap();
        write_startup_instructions_respecting_existing(dir.path(), "QWEN.md", "second version").unwrap();

        let content = std::fs::read_to_string(dir.path().join("QWEN.md")).unwrap();
        assert!(content.contains("second version"));
        assert!(!content.contains("first version"), "regeneration must replace, not accumulate");
    }

    #[test]
    fn startup_instructions_foreign_file_content_is_never_touched() {
        // The exact regression codex P1 (PR #2788) flagged: a real project's
        // own AGENTS.md must survive an agent launch untouched, byte for
        // byte — no AgentMux marker, no partial merge, nothing.
        let dir = tempfile::tempdir().unwrap();
        let human_content = "# My real project\n\nHand-written AGENTS.md, no AgentMux involvement.";
        std::fs::write(dir.path().join("AGENTS.md"), human_content).unwrap();

        write_startup_instructions_respecting_existing(dir.path(), "AGENTS.md", "AgentMux's generated content")
            .unwrap();

        let content = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert_eq!(content, human_content, "every byte of the original file must survive");
        assert!(!content.contains("AgentMux's generated content"));
    }

    #[test]
    fn startup_instructions_foreign_empty_file_is_never_touched() {
        // An empty pre-existing file (no marker) is still foreign — the
        // decision hinges on the marker prefix, not on non-emptiness.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("GEMINI.md"), "").unwrap();

        write_startup_instructions_respecting_existing(dir.path(), "GEMINI.md", "AgentMux's generated content")
            .unwrap();

        let content = std::fs::read_to_string(dir.path().join("GEMINI.md")).unwrap();
        assert_eq!(content, "");
    }

    #[test]
    fn startup_instructions_non_utf8_file_is_treated_as_foreign_not_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, [0x23, 0x20, 0xFF, 0xFE, 0x0A]).unwrap();
        let raw_before = std::fs::read(&path).unwrap();

        write_startup_instructions_respecting_existing(dir.path(), "AGENTS.md", "generated content").unwrap();

        let raw_after = std::fs::read(&path).unwrap();
        assert_eq!(raw_before, raw_after, "an unreadable file must be treated as foreign, never as absent");
    }

    #[test]
    fn startup_instructions_creates_parent_directories_for_nested_paths() {
        // pi's target is .pi/APPEND_SYSTEM.md — a nested path with no
        // existing parent directory on a fresh working dir.
        let dir = tempfile::tempdir().unwrap();
        write_startup_instructions_respecting_existing(dir.path(), ".pi/APPEND_SYSTEM.md", "content").unwrap();

        let content = std::fs::read_to_string(dir.path().join(".pi/APPEND_SYSTEM.md")).unwrap();
        assert!(content.contains("content"));
    }

    // ============================================================
    // write_claude_md_respecting_ownership
    // (SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md)
    // ============================================================

    #[test]
    fn claude_md_fresh_working_dir_writes_directly_with_the_marker() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_md_respecting_ownership(dir.path(), "Soul + AgentMD + Memory + Skills").unwrap();

        let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.starts_with(CLAUDE_MD_MANAGED_MARKER));
        assert!(content.contains("Soul + AgentMD + Memory + Skills"));
        // No foreign-file side effects on the common (fresh dir) path.
        assert!(!dir.path().join(AGENTMUX_MEMORY_FILENAME).exists());
    }

    #[test]
    fn claude_md_agentmux_owned_file_is_freely_regenerated() {
        let dir = tempfile::tempdir().unwrap();
        write_claude_md_respecting_ownership(dir.path(), "first version").unwrap();
        write_claude_md_respecting_ownership(dir.path(), "second version").unwrap();

        let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.contains("second version"));
        assert!(!content.contains("first version"), "regeneration must replace, not accumulate");
    }

    #[test]
    fn claude_md_foreign_file_content_is_never_touched() {
        let dir = tempfile::tempdir().unwrap();
        let human_content = "# My real project\n\nHand-written rules, no AgentMux marker.";
        std::fs::write(dir.path().join("CLAUDE.md"), human_content).unwrap();

        write_claude_md_respecting_ownership(dir.path(), "AgentMux's generated content").unwrap();

        let content = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(content.starts_with(human_content), "every byte of the original file must survive, at the start");
        assert!(!content.contains("AgentMux's generated content"), "generated content must never land in the real file");
    }

    #[test]
    fn claude_md_non_utf8_file_is_treated_as_foreign_not_missing() {
        // reagent P0 + codex P1 on PR #2747: read_to_string(...).ok()
        // collapsed ANY read error (not just NotFound) into "no file
        // yet," so a non-UTF-8 (or otherwise unreadable) foreign file
        // got silently clobbered — the exact data-loss bug this whole
        // function exists to prevent.
        let dir = tempfile::tempdir().unwrap();
        let claude_md_path = dir.path().join("CLAUDE.md");
        // Invalid UTF-8: a lone continuation byte. read_to_string must
        // fail on this (not silently lossy-convert it).
        std::fs::write(&claude_md_path, [0x23, 0x20, 0xFF, 0xFE, 0x0A]).unwrap();
        let raw_before = std::fs::read(&claude_md_path).unwrap();

        write_claude_md_respecting_ownership(dir.path(), "AgentMux's generated content").unwrap();

        let raw_after = std::fs::read(&claude_md_path).unwrap();
        assert_eq!(raw_before, raw_after, "an unreadable foreign file's bytes must be completely untouched, not overwritten");
        // Best-effort side file is still written even though the
        // @import offer itself is skipped (nothing safe to append to).
        let side_file = std::fs::read_to_string(dir.path().join(AGENTMUX_MEMORY_FILENAME)).unwrap();
        assert_eq!(side_file, "AgentMux's generated content");
    }

    #[test]
    fn claude_md_foreign_file_gets_a_side_file_and_one_import_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# Real project\n").unwrap();

        write_claude_md_respecting_ownership(dir.path(), "the generated body").unwrap();

        let side_file = std::fs::read_to_string(dir.path().join(AGENTMUX_MEMORY_FILENAME)).unwrap();
        assert_eq!(side_file, "the generated body");

        let claude_md = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(claude_md.contains(&format!("@{AGENTMUX_MEMORY_FILENAME}")));
        assert!(claude_md.contains(CLAUDE_MD_IMPORT_MARKER_COMMENT));
    }

    #[test]
    fn claude_md_import_line_is_never_duplicated_across_launches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# Real project\n").unwrap();

        write_claude_md_respecting_ownership(dir.path(), "v1").unwrap();
        write_claude_md_respecting_ownership(dir.path(), "v2").unwrap();
        write_claude_md_respecting_ownership(dir.path(), "v3").unwrap();

        let claude_md = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        let import_needle = format!("@{AGENTMUX_MEMORY_FILENAME}");
        assert_eq!(claude_md.matches(&import_needle).count(), 1, "the import line must appear exactly once, no matter how many launches");
        // The side file, unlike CLAUDE.md, keeps regenerating freely.
        let side_file = std::fs::read_to_string(dir.path().join(AGENTMUX_MEMORY_FILENAME)).unwrap();
        assert_eq!(side_file, "v3");
    }

    #[test]
    fn claude_md_concurrent_launches_against_a_shared_workdir_never_duplicate_the_import() {
        // reagent P2 (second review round) on PR #2747: two agents
        // sharing a working directory (agent_open.rs's shared-workdir
        // fallback) launching concurrently previously could both read
        // "not yet offered" and both append the import line — the only
        // existing serialization (agent_open_lock) is keyed by
        // agent_id, not by working directory. Real threads, not a
        // sequential simulation, to actually exercise the race.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# Real project\n").unwrap();
        let dir_path = dir.path().to_path_buf();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let dir_path = dir_path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait(); // maximize actual overlap
                    write_claude_md_respecting_ownership(&dir_path, &format!("content-{i}")).unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let claude_md = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        let import_needle = format!("@{AGENTMUX_MEMORY_FILENAME}");
        assert_eq!(
            claude_md.matches(&import_needle).count(),
            1,
            "8 concurrent launches against the same foreign CLAUDE.md must still produce exactly one import line, not one per racer"
        );
    }

    #[test]
    fn claude_md_marker_is_rolled_back_when_the_append_fails() {
        // reagent P1 + codex, third review round on PR #2747: winning the
        // create_new race must not permanently record "offered" if the
        // append itself then fails -- otherwise every later launch's
        // create_new hits AlreadyExists and the import is never offered
        // again, with no retry path.
        let dir = tempfile::tempdir().unwrap();
        let claude_md_path = dir.path().join("CLAUDE.md");
        std::fs::write(&claude_md_path, "# Real project\n").unwrap();

        // Read-only so the append's .open(...) fails, even though the
        // read earlier in the function (which only needs read access)
        // succeeds fine.
        let mut perms = std::fs::metadata(&claude_md_path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&claude_md_path, perms).unwrap();

        write_claude_md_respecting_ownership(dir.path(), "v1").unwrap();

        let marker_path = dir.path().join(CLAUDE_MD_OWNERSHIP_MARKER_PATH);
        assert!(!marker_path.exists(), "a failed append must roll back the ownership marker so a future launch can retry");

        // Restore write access and confirm a later launch actually succeeds.
        let mut perms = std::fs::metadata(&claude_md_path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&claude_md_path, perms).unwrap();

        write_claude_md_respecting_ownership(dir.path(), "v2").unwrap();
        let claude_md = std::fs::read_to_string(&claude_md_path).unwrap();
        let import_needle = format!("@{AGENTMUX_MEMORY_FILENAME}");
        assert_eq!(claude_md.matches(&import_needle).count(), 1, "retry after the file becomes writable again must succeed");
    }

    #[test]
    fn claude_md_user_deleted_import_line_is_never_reinserted() {
        // The whole point of the ownership marker file: once offered, a
        // user's deliberate removal of the import line must stick.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# Real project\n").unwrap();
        write_claude_md_respecting_ownership(dir.path(), "v1").unwrap();

        // User opts out: deletes the import line by hand.
        std::fs::write(dir.path().join("CLAUDE.md"), "# Real project\n").unwrap();

        write_claude_md_respecting_ownership(dir.path(), "v2").unwrap();

        let claude_md = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert_eq!(claude_md, "# Real project\n", "import line must not silently reappear after the user removed it");
        // The side file still refreshes even though nothing links to it
        // from CLAUDE.md anymore — harmless, and keeps it ready if the
        // user re-adds the import line themselves later.
        let side_file = std::fs::read_to_string(dir.path().join(AGENTMUX_MEMORY_FILENAME)).unwrap();
        assert_eq!(side_file, "v2");
    }
}
