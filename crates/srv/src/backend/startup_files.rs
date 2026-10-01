// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The files a Claude Code session reads by itself at startup, listed for the
//! "Given to the agent" card
//! (SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md §4,
//! phase LC2). Before this the card showed only what the SessionStart hook
//! carried, while the CLI had also loaded the user's `CLAUDE.md`, every
//! `CLAUDE.md` up the folder tree (the hand-maintained
//! `~/.agentmux/agents/CLAUDE.md` among them), their `@imports`, the skill
//! listing and the MCP servers.
//!
//! This mirrors Claude Code's documented loading rules: it is what the CLI
//! *should* have read, not proof that it did.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Claude Code follows `@imports` this many hops deep.
const MAX_IMPORT_DEPTH: usize = 5;

/// Who writes a file: AgentMux, the user (a file in the agent's own
/// workspace that AgentMux doesn't manage), or someone else (outside the
/// workspace and not written by AgentMux, e.g. the hand-maintained shared
/// `agents/CLAUDE.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Agentmux,
    User,
    External,
}

impl Owner {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agentmux => "agentmux",
            Self::User => "user",
            Self::External => "external",
        }
    }
}

/// One startup item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupFile {
    /// `user_instructions`, `instructions`, `instructions_import`, `skills`
    /// or `mcp_servers`.
    pub role: &'static str,
    /// Shown in the card: `~/…` for a path under the home dir.
    pub name: String,
    pub path: PathBuf,
    pub owner: Owner,
    /// The text as read; empty for the skills and MCP items, which list.
    pub text: String,
    /// For the skills and MCP items: how many are listed.
    pub count: Option<usize>,
    /// AgentMux sections the file carries: `global_memory`, `skills_index`.
    pub contains: Vec<&'static str>,
}

/// Claude's startup files for a session in `cwd`, in the order the CLI loads
/// them: the user's `CLAUDE.md` (in `config_dir`, else `~/.claude`), then
/// each folder from the filesystem root down to `cwd` (its `CLAUDE.md`,
/// `.claude/CLAUDE.md`, `CLAUDE.local.md`), each file followed by what it
/// imports; then the skill listing and the MCP servers of `cwd`.
pub fn claude_startup_files(cwd: &Path, config_dir: Option<&Path>, home: Option<&Path>) -> Vec<StartupFile> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let user_md = config_dir
        .map(Path::to_path_buf)
        .or_else(|| home.map(|h| h.join(".claude")))
        .map(|d| d.join("CLAUDE.md"));
    if let Some(p) = user_md {
        add_instructions(&p, "user_instructions", cwd, home, 0, &mut seen, &mut out);
    }
    let mut dirs: Vec<&Path> = cwd.ancestors().collect();
    // "Up to, but not including, the root directory."
    dirs.retain(|d| d.parent().is_some());
    dirs.reverse();
    for dir in dirs {
        for name in ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"] {
            add_instructions(&dir.join(name), "instructions", cwd, home, 0, &mut seen, &mut out);
        }
    }
    if let Some(skills) = skills_item(cwd, home) {
        out.push(skills);
    }
    if let Some(mcp) = mcp_item(cwd, home) {
        out.push(mcp);
    }
    out
}

fn add_instructions(
    path: &Path,
    role: &'static str,
    cwd: &Path,
    home: Option<&Path>,
    depth: usize,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<StartupFile>,
) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(key) {
        return;
    }
    let owner = owner_of(path, &text, cwd);
    let contains = contains(&text);
    let imports = if depth < MAX_IMPORT_DEPTH { imports_of(&text, path, home) } else { Vec::new() };
    out.push(StartupFile {
        role,
        name: display_name(path, home),
        path: path.to_path_buf(),
        owner,
        text,
        count: None,
        contains,
    });
    for import in imports {
        add_instructions(&import, "instructions_import", cwd, home, depth + 1, seen, out);
    }
}

fn owner_of(path: &Path, text: &str, cwd: &Path) -> Owner {
    let agentmux_written = text.contains(crate::backend::agent_config::CLAUDE_MD_MANAGED_MARKER)
        || text.contains("AgentMux: intentionally empty")
        || path == cwd.join(crate::backend::agent_config::AGENTMUX_MEMORY_FILENAME);
    if agentmux_written {
        Owner::Agentmux
    } else if path.starts_with(cwd) {
        Owner::User
    } else {
        Owner::External
    }
}

fn contains(text: &str) -> Vec<&'static str> {
    let mut c = Vec::new();
    if text.contains("# [AgentMux System] ") || text.contains("# [Workspace] ") {
        c.push("global_memory");
    }
    if text.starts_with("# Available Skills") || text.contains("\n# Available Skills") {
        c.push("skills_index");
    }
    c
}

/// `@path` imports in `text`, resolved against the importing file's folder
/// (`~/` against the home dir), skipping fenced and inline code, which
/// Claude Code doesn't evaluate. Only files that exist.
fn imports_of(text: &str, file: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let base = file.parent().unwrap_or(Path::new("."));
    let mut out = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        // Odd segments between backticks are inline code.
        for (i, segment) in line.split('`').enumerate() {
            if i % 2 == 1 {
                continue;
            }
            for token in segment.split_whitespace() {
                let Some(raw) = token.strip_prefix('@') else { continue };
                let raw = raw.trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | ')' | ']'));
                if raw.is_empty() || raw.contains("://") {
                    continue;
                }
                let path = match (raw.strip_prefix("~/"), home) {
                    (Some(rest), Some(h)) => h.join(rest),
                    (Some(_), None) => continue,
                    (None, _) => {
                        let p = Path::new(raw);
                        if p.is_absolute() { p.to_path_buf() } else { base.join(p) }
                    }
                };
                if path.is_file() {
                    out.push(path);
                }
            }
        }
    }
    out
}

/// The skill listing Claude Code builds from `.claude/commands/*.md` and
/// `.claude/skills/*/SKILL.md`: one item with a count, no text (the CLI lists
/// each by name and description).
fn skills_item(cwd: &Path, home: Option<&Path>) -> Option<StartupFile> {
    let mut count = 0usize;
    if let Ok(entries) = std::fs::read_dir(cwd.join(".claude/commands")) {
        count += entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
            .count();
    }
    if let Ok(entries) = std::fs::read_dir(cwd.join(".claude/skills")) {
        count += entries.flatten().filter(|e| e.path().join("SKILL.md").is_file()).count();
    }
    (count > 0).then(|| StartupFile {
        role: "skills",
        name: format!("Skills ({count}) in {}", display_name(&cwd.join(".claude"), home)),
        path: cwd.join(".claude"),
        owner: if cwd.join(crate::backend::agent_config::MANAGED_SKILL_FILES_MANIFEST).is_file() {
            Owner::Agentmux
        } else {
            Owner::User
        },
        text: String::new(),
        count: Some(count),
        contains: Vec::new(),
    })
}

/// The MCP servers in `cwd/.mcp.json`, by name only: the file carries the
/// agent's signing keys, so its text never leaves this function.
fn mcp_item(cwd: &Path, home: Option<&Path>) -> Option<StartupFile> {
    let path = cwd.join(".mcp.json");
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
    let names: Vec<&str> = json.get("mcpServers")?.as_object()?.keys().map(String::as_str).collect();
    if names.is_empty() {
        return None;
    }
    Some(StartupFile {
        role: "mcp_servers",
        name: format!("MCP servers: {} ({})", names.join(", "), display_name(&path, home)),
        path,
        owner: if names.contains(&"agentmux") { Owner::Agentmux } else { Owner::User },
        text: String::new(),
        count: Some(names.len()),
        contains: Vec::new(),
    })
}

/// `~/x/y` for a path under `home`, else the path; forward slashes.
fn display_name(path: &Path, home: Option<&Path>) -> String {
    let shown = match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    };
    shown.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// A tree like a real agent's: home/.agentmux/agents/CLAUDE.md (shared,
    /// hand-maintained), the agent's own foreign CLAUDE.md importing
    /// AGENTMUX_MEMORY.md, a config-dir CLAUDE.md, skills and .mcp.json.
    fn agent_tree() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let agents = home.join(".agentmux/agents");
        let cwd = agents.join("clamk-0612a");
        let config = home.join(".agentmux/channels/c/identities/x/claude");
        write(&agents.join("CLAUDE.md"), "# Jekt rules\n");
        write(&cwd.join("CLAUDE.md"), "# Available Skills\n\n@.claude/AGENTMUX_MEMORY.md\n");
        write(
            &cwd.join(".claude/AGENTMUX_MEMORY.md"),
            "# Memory\n# [AgentMux System] App API\n\n# Available Skills\n",
        );
        write(&config.join("CLAUDE.md"), "AgentMux: intentionally empty.\n");
        write(&cwd.join(".claude/commands/tdd.md"), "x");
        write(&cwd.join(".claude/commands/code-review.md"), "x");
        write(&cwd.join(".claude/skills/run/SKILL.md"), "x");
        write(&cwd.join(".claude/.agentmux-managed-skill-files.json"), "[]");
        write(
            &cwd.join(".mcp.json"),
            r#"{"mcpServers":{"agentmux":{"env":{"AGENTMUX_JEKT_KEY":"secret"}}}}"#,
        );
        (t, home, cwd, config)
    }

    #[test]
    fn lists_what_claude_loads_in_order_with_owners() {
        let (_t, home, cwd, config) = agent_tree();
        let files = claude_startup_files(&cwd, Some(&config), Some(&home));
        let got: Vec<(&str, &str, &str)> = files.iter().map(|f| (f.role, f.name.as_str(), f.owner.as_str())).collect();
        assert_eq!(
            got,
            [
                ("user_instructions", "~/.agentmux/channels/c/identities/x/claude/CLAUDE.md", "agentmux"),
                ("instructions", "~/.agentmux/agents/CLAUDE.md", "external"),
                ("instructions", "~/.agentmux/agents/clamk-0612a/CLAUDE.md", "user"),
                ("instructions_import", "~/.agentmux/agents/clamk-0612a/.claude/AGENTMUX_MEMORY.md", "agentmux"),
                ("skills", "Skills (3) in ~/.agentmux/agents/clamk-0612a/.claude", "agentmux"),
                ("mcp_servers", "MCP servers: agentmux (~/.agentmux/agents/clamk-0612a/.mcp.json)", "agentmux"),
            ]
        );
        let memory = &files[3];
        assert_eq!(memory.contains, ["global_memory", "skills_index"]);
        assert_eq!(files[2].contains, ["skills_index"]);
    }

    #[test]
    fn the_mcp_item_never_carries_the_file() {
        let (_t, home, cwd, config) = agent_tree();
        let files = claude_startup_files(&cwd, Some(&config), Some(&home));
        let mcp = files.iter().find(|f| f.role == "mcp_servers").unwrap();
        assert!(mcp.text.is_empty());
        assert_eq!(mcp.count, Some(1));
        assert!(!format!("{mcp:?}").contains("secret"));
    }

    #[test]
    fn imports_resolve_relative_and_home_skip_code_and_stop_at_cycles() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let cwd = t.path().join("ws");
        write(
            &cwd.join("CLAUDE.md"),
            "See @docs/a.md and @~/notes.md.\n`@docs/in-code.md`\n```\n@docs/fenced.md\n```\n@missing.md\n",
        );
        write(&cwd.join("docs/a.md"), "@../CLAUDE.md back again\n");
        write(&cwd.join("docs/in-code.md"), "x");
        write(&cwd.join("docs/fenced.md"), "x");
        write(&home.join("notes.md"), "notes");
        let files = claude_startup_files(&cwd, None, Some(&home));
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        let a = cwd.join("docs/a.md").display().to_string().replace('\\', "/");
        let root_md = cwd.join("CLAUDE.md").display().to_string().replace('\\', "/");
        assert_eq!(names, [root_md.as_str(), a.as_str(), "~/notes.md"]);
        assert!(files[1..].iter().all(|f| f.role == "instructions_import"));
    }

    #[test]
    fn nothing_there_lists_nothing() {
        let t = tempfile::tempdir().unwrap();
        assert!(claude_startup_files(&t.path().join("ws"), None, Some(&t.path().join("home"))).is_empty());
    }
}
