// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pure config-building logic for Forge agents.
//!
//! Ports the `buildConfigFiles`, `buildMcpConfig`, and `expandTemplate`
//! functions from `frontend/app/view/agent/agent-model.ts`.
//! All functions are pure — no I/O, no async.

use std::collections::HashMap;

use chrono::Utc;
use serde_json::{json, Value};

use crate::backend::storage::wstore::ForgeSkill;

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
/// Assembles `CLAUDE.md` from `soul` + `agentmd` + `memory` + skills index,
/// writes each skill as a slash command under `.claude/commands/<trigger>.md`,
/// writes `.claude/hooks.json` if a `hooks` content entry is present,
/// auto-injects the AgentMux MCP server entry, and applies `{{VARIABLE}}`
/// template substitution throughout.
///
/// Mirrors `buildConfigFiles()` in `frontend/app/view/agent/agent-model.ts`.
pub fn build_config_files(
    content_map: &HashMap<String, String>,
    skills: &[ForgeSkill],
    agent_name: &str,
    agent_id: &str,
) -> Vec<AgentConfigFile> {
    let mut files: Vec<AgentConfigFile> = Vec::new();

    // Template variables for {{}} substitution
    let mut template_vars: HashMap<String, String> = HashMap::new();
    template_vars.insert("AGENT".to_string(), agent_name.to_string());
    template_vars.insert("AGENT_DISPLAY".to_string(), agent_name.to_string());
    template_vars.insert("AGENT_ID".to_string(), agent_id.to_string());
    // DATE in YYYY-MM-DD format, UTC
    template_vars.insert("DATE".to_string(), Utc::now().format("%Y-%m-%d").to_string());
    // WORKING_DIR is not available in this signature; leave it empty for callers
    // that don't pass it — expansion will leave {{WORKING_DIR}} intact if absent.

    // ----------------------------------------------------------------
    // Build CLAUDE.md: Soul + AgentMD + Memory + Skills index
    // ----------------------------------------------------------------
    let mut claude_md_parts: Vec<String> = Vec::new();

    if let Some(soul) = content_map.get("soul") {
        claude_md_parts.push(expand_template(soul, &template_vars));
    }
    if let Some(agentmd) = content_map.get("agentmd") {
        if !claude_md_parts.is_empty() {
            claude_md_parts.push("\n---\n".to_string());
        }
        claude_md_parts.push(expand_template(agentmd, &template_vars));
    }
    if let Some(memory) = content_map.get("memory") {
        claude_md_parts.push("\n# Memory\n".to_string());
        claude_md_parts.push(memory.clone());
    }

    // Append skill index with trigger references
    if !skills.is_empty() {
        claude_md_parts.push("\n# Available Skills\n\n".to_string());
        claude_md_parts.push("Use `/<trigger>` to invoke a skill.\n\n".to_string());
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
            claude_md_parts.push(format!("- **{}**{}{}\n", skill.name, trigger_part, desc_part));
        }
    }

    if !claude_md_parts.is_empty() {
        files.push(AgentConfigFile {
            filename: "CLAUDE.md".to_string(),
            content: claude_md_parts.join(""),
        });
    }

    // ----------------------------------------------------------------
    // Write each skill as a slash command: .claude/commands/{trigger}.md
    // ----------------------------------------------------------------
    for skill in skills {
        if !skill.trigger.is_empty() && !skill.content.is_empty() {
            let content = expand_template(&skill.content, &template_vars);
            files.push(AgentConfigFile {
                filename: format!(".claude/commands/{}.md", skill.trigger),
                content,
            });
        }
    }

    // ----------------------------------------------------------------
    // Write .claude/hooks.json if hooks content is present
    // ----------------------------------------------------------------
    if let Some(hooks) = content_map.get("hooks") {
        files.push(AgentConfigFile {
            filename: ".claude/hooks.json".to_string(),
            content: hooks.clone(),
        });
    }

    // ----------------------------------------------------------------
    // Build .mcp.json with auto-injected AgentMux MCP server
    // ----------------------------------------------------------------
    // agent_bus_id is not in the function signature; callers that have it
    // should call build_mcp_config directly and push the result themselves,
    // or use the variant below.
    let mcp_content = content_map.get("mcp").map(|s| s.as_str());
    if let Some(mcp_json) = build_mcp_config(mcp_content, agent_name, "") {
        files.push(AgentConfigFile {
            filename: ".mcp.json".to_string(),
            content: mcp_json,
        });
    }

    files
}

/// Build the list of config files with a known `agent_bus_id`.
///
/// Same as [`build_config_files`] but also accepts an `agent_bus_id` so the
/// MCP server entry can include `AGENTMUX_AGENT_BUS_ID`.  Prefer this overload
/// when the caller has the full `ForgeAgent` available.
pub fn build_config_files_with_bus(
    content_map: &HashMap<String, String>,
    skills: &[ForgeSkill],
    agent_name: &str,
    agent_id: &str,
    agent_bus_id: &str,
    working_directory: &str,
) -> Vec<AgentConfigFile> {
    let mut files: Vec<AgentConfigFile> = Vec::new();

    let mut template_vars: HashMap<String, String> = HashMap::new();
    template_vars.insert("AGENT".to_string(), agent_name.to_string());
    template_vars.insert("AGENT_DISPLAY".to_string(), agent_name.to_string());
    template_vars.insert("AGENT_ID".to_string(), agent_id.to_string());
    template_vars.insert("WORKING_DIR".to_string(), working_directory.to_string());
    template_vars.insert("DATE".to_string(), Utc::now().format("%Y-%m-%d").to_string());

    // CLAUDE.md
    let mut claude_md_parts: Vec<String> = Vec::new();
    if let Some(soul) = content_map.get("soul") {
        claude_md_parts.push(expand_template(soul, &template_vars));
    }
    if let Some(agentmd) = content_map.get("agentmd") {
        if !claude_md_parts.is_empty() {
            claude_md_parts.push("\n---\n".to_string());
        }
        claude_md_parts.push(expand_template(agentmd, &template_vars));
    }
    if let Some(memory) = content_map.get("memory") {
        claude_md_parts.push("\n# Memory\n".to_string());
        claude_md_parts.push(memory.clone());
    }
    if !skills.is_empty() {
        claude_md_parts.push("\n# Available Skills\n\n".to_string());
        claude_md_parts.push("Use `/<trigger>` to invoke a skill.\n\n".to_string());
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
            claude_md_parts.push(format!("- **{}**{}{}\n", skill.name, trigger_part, desc_part));
        }
    }
    if !claude_md_parts.is_empty() {
        files.push(AgentConfigFile {
            filename: "CLAUDE.md".to_string(),
            content: claude_md_parts.join(""),
        });
    }

    // Skill slash commands
    for skill in skills {
        if !skill.trigger.is_empty() && !skill.content.is_empty() {
            let content = expand_template(&skill.content, &template_vars);
            files.push(AgentConfigFile {
                filename: format!(".claude/commands/{}.md", skill.trigger),
                content,
            });
        }
    }

    // Hooks
    if let Some(hooks) = content_map.get("hooks") {
        files.push(AgentConfigFile {
            filename: ".claude/hooks.json".to_string(),
            content: hooks.clone(),
        });
    }

    // MCP — use full bus_id variant
    let mcp_content = content_map.get("mcp").map(|s| s.as_str());
    if let Some(mcp_json) = build_mcp_config(mcp_content, agent_name, agent_bus_id) {
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
    agent_name: &str,
    agent_bus_id: &str,
) -> Option<String> {
    // Auto-injected AgentMux MCP server entry
    let mut env_map = serde_json::Map::new();
    if !agent_name.is_empty() {
        env_map.insert("AGENTMUX_AGENT_ID".to_string(), json!(agent_name));
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
                // Invalid JSON in forge content — keep auto-injected only (mirrors TS behavior)
                tracing::error!("agent_config: invalid MCP JSON in forge content, using auto-injected only");
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

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_skill(name: &str, trigger: &str, description: &str, content: &str) -> ForgeSkill {
        ForgeSkill {
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
        let result = build_mcp_config(None, "Aria", "bus-42").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let servers = &parsed["mcpServers"];
        assert!(servers["agentmux"].is_object());
        assert_eq!(servers["agentmux"]["command"], "agentmux-mcp");
        assert_eq!(servers["agentmux"]["env"]["AGENTMUX_AGENT_ID"], "Aria");
        assert_eq!(servers["agentmux"]["env"]["AGENTMUX_AGENT_BUS_ID"], "bus-42");
    }

    #[test]
    fn test_build_mcp_config_merges_user_servers() {
        let user_mcp = r#"{"mcpServers": {"mytool": {"type": "stdio", "command": "mytool"}}}"#;
        let result = build_mcp_config(Some(user_mcp), "Aria", "").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let servers = &parsed["mcpServers"];
        assert!(servers["agentmux"].is_object());
        assert!(servers["mytool"].is_object());
    }

    #[test]
    fn test_build_mcp_config_invalid_user_json_uses_auto_injected() {
        let result = build_mcp_config(Some("not json {{"), "Aria", "").unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["mcpServers"]["agentmux"].is_object());
    }

    #[test]
    fn test_build_config_files_claude_md_assembled() {
        let mut content_map = HashMap::new();
        content_map.insert("soul".to_string(), "You are {{AGENT}}.".to_string());
        content_map.insert("agentmd".to_string(), "## Instructions\nDo stuff.".to_string());

        let files = build_config_files(&content_map, &[], "Aria", "agent-1");
        let claude_md = files.iter().find(|f| f.filename == "CLAUDE.md").unwrap();
        assert!(claude_md.content.contains("You are Aria."));
        assert!(claude_md.content.contains("---"));
        assert!(claude_md.content.contains("## Instructions"));
    }

    #[test]
    fn test_build_config_files_skills_index_and_commands() {
        let content_map = HashMap::new();
        let skills = vec![
            make_skill("Deploy", "deploy", "Deploy the app", "Run: deploy all"),
            make_skill("Test", "test", "Run tests", "Run: test suite"),
        ];

        let files = build_config_files(&content_map, &skills, "Aria", "agent-1");

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
    fn test_build_config_files_hooks_written() {
        let mut content_map = HashMap::new();
        content_map.insert("hooks".to_string(), r#"{"hooks":[]}"#.to_string());

        let files = build_config_files(&content_map, &[], "Aria", "agent-1");
        assert!(files.iter().any(|f| f.filename == ".claude/hooks.json"));
    }

    #[test]
    fn test_build_config_files_mcp_written() {
        let content_map = HashMap::new();
        let files = build_config_files(&content_map, &[], "Aria", "agent-1");
        let mcp = files.iter().find(|f| f.filename == ".mcp.json").unwrap();
        let parsed: Value = serde_json::from_str(&mcp.content).unwrap();
        assert!(parsed["mcpServers"]["agentmux"].is_object());
    }
}
