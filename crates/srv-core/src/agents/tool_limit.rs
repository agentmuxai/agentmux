// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a Drone Agent block's one-shot `claude` may use (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §5).
//!
//! The block's `allow_commands` is the whole answer, and the runner passes it to the CLI as flags:
//!   - nothing, `false` or an empty list: the tools that read (`Read`, `Grep`, `Glob`) and no MCP server;
//!   - a command line, or a list of them: those and `Bash` for exactly those command lines, and no MCP server;
//!   - `true`: no limit from AgentMux (the check that turns a job on asks the person about it).
//!
//! A tool the CLI is not given is not there for the model to call, and a command line that is not listed is not run
//! (`--permission-mode dontAsk` answers no to anything that is not allowed, where `--print` has nobody to ask).

use serde_json::Value;

/// The tools a native Agent block may use.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolLimit {
    /// The reading tools, and `Bash` for exactly these command lines (none: no `Bash`).
    Limited { commands: Vec<String> },
    /// The CLI's own permission settings are the only limit.
    Unlimited,
}

impl Default for ToolLimit {
    /// A task that says nothing about its tools is limited to reading.
    fn default() -> Self {
        ToolLimit::Limited { commands: Vec::new() }
    }
}

/// The tools that read and change nothing.
const READING: &str = "Read,Grep,Glob";

impl ToolLimit {
    /// From a block's `allow_commands`: absent or `false`, a command line, a list of them, or `true`.
    pub fn from_allow_commands(value: Option<&Value>) -> Self {
        let line = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
        match value {
            Some(Value::Bool(true)) => ToolLimit::Unlimited,
            Some(Value::String(s)) => ToolLimit::Limited { commands: line(s).into_iter().collect() },
            Some(Value::Array(list)) => ToolLimit::Limited { commands: list.iter().filter_map(Value::as_str).filter_map(line).collect() },
            _ => ToolLimit::default(),
        }
    }

    /// The flags that carry the limit to the CLI.
    pub fn args(&self) -> Vec<String> {
        let ToolLimit::Limited { commands } = self else { return Vec::new() };
        let mut args = vec!["--strict-mcp-config".to_string(), "--permission-mode".to_string(), "dontAsk".to_string(), "--tools".to_string()];
        if commands.is_empty() {
            args.push(READING.to_string());
            return args;
        }
        args.push(format!("{READING},Bash"));
        args.push("--allowedTools".to_string());
        args.extend(commands.iter().map(|c| format!("Bash({})", escaped(c))));
        args
    }
}

/// A command line as the CLI's permission rule reads it exactly: a `(`, `)`, `*` or `\` in it is itself.
fn escaped(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    for c in command.chars() {
        if matches!(c, '\\' | '(' | ')' | '*') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn of(v: Value) -> ToolLimit {
        ToolLimit::from_allow_commands(Some(&v))
    }

    #[test]
    fn a_block_with_no_commands_is_limited_to_reading_and_has_no_mcp_server() {
        for v in [None, Some(json!(false)), Some(json!([])), Some(json!("  ")), Some(Value::Null)] {
            let args = ToolLimit::from_allow_commands(v.as_ref()).args();
            assert_eq!(args, ["--strict-mcp-config", "--permission-mode", "dontAsk", "--tools", "Read,Grep,Glob"], "{v:?}");
        }
    }

    #[test]
    fn listed_command_lines_are_the_only_commands() {
        let args = of(json!(["git status", "cargo test (fast)", 7])).args();
        let tools = args.iter().position(|a| a == "--tools").unwrap();
        assert_eq!(args[tools + 1], "Read,Grep,Glob,Bash");
        let allowed = args.iter().position(|a| a == "--allowedTools").unwrap();
        assert_eq!(args[allowed + 1..], ["Bash(git status)", r"Bash(cargo test \(fast\))"], "a value that is not a line is left out");
        assert!(args.iter().any(|a| a == "--strict-mcp-config"));
        assert_eq!(of(json!("make check")).args().last().unwrap(), "Bash(make check)", "a single line is a list of one");
    }

    #[test]
    fn a_star_in_a_command_line_is_not_a_pattern() {
        assert_eq!(escaped(r"rm -rf *"), r"rm -rf \*");
        assert_eq!(escaped(r"a\b"), r"a\\b");
    }

    #[test]
    fn only_true_lifts_the_limit_and_it_adds_no_flag() {
        assert_eq!(of(json!(true)), ToolLimit::Unlimited);
        assert!(ToolLimit::Unlimited.args().is_empty());
        assert_eq!(ToolLimit::default(), ToolLimit::from_allow_commands(None), "a task that says nothing is limited");
        assert_eq!(of(json!(1)), ToolLimit::default(), "a value of another kind is not a way out");
    }
}
