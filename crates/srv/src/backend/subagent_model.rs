// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which model a Claude agent's sub-agents run on
//! (`docs/specs/SPEC_SUBAGENT_MODELS_2026_10_25.md`).
//!
//! Sub-agents were about a quarter of the fleet's spend and almost all of
//! their calls ran on the biggest model, because a sub-agent inherits its
//! parent's model. Measured on Claude Code 2.1.288:
//!
//! - `CLAUDE_CODE_SUBAGENT_MODEL` moves **general-purpose** sub-agents (and
//!   custom ones with no model of their own). It is set at spawn from
//!   [`SETTING_SUBAGENT`], or the pane's own [`META_SUBAGENT`]. Without
//!   `..._FORCE`, an agent's own `model:` and the `model` its parent passes
//!   per call still win, so a leader can send a hard question to Opus.
//! - **Explore** and **Plan** ignore it and stay on the parent's model.
//!   Explore is moved by a user-level agent definition named `Explore` in the
//!   Claude config dir, which beats the built-in: AgentMux writes one
//!   ([`seed_explore_agent`]) from [`SETTING_EXPLORE`] and removes it again
//!   when the setting says `inherit`. One a person wrote is left alone.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::backend::obj::{meta_get_string, MetaMapType};

/// `agent:subagentmodel`: the model for general-purpose sub-agents.
pub const SETTING_SUBAGENT: &str = "agent:subagentmodel";
pub const DEFAULT_SUBAGENT_MODEL: &str = "sonnet";
/// `agent:exploremodel`: the model for Explore sub-agents.
pub const SETTING_EXPLORE: &str = "agent:exploremodel";
pub const DEFAULT_EXPLORE_MODEL: &str = "haiku";
/// Block meta that overrides [`SETTING_SUBAGENT`] for one agent.
pub const META_SUBAGENT: &str = "agent:subagentmodel";
const ENV_KEY: &str = "CLAUDE_CODE_SUBAGENT_MODEL";
/// First line of the body of the Explore definition AgentMux wrote.
const MANAGED_MARK: &str = "<!-- agentmux-managed: replaced from the agent:exploremodel setting; set it to inherit to remove -->";

/// The model a value names: an alias (`sonnet`, `haiku`, `opus`) or a model
/// ID. `None` for an empty value, for `inherit` (and its synonyms, which mean
/// "don't set one"), and for text that couldn't be a model name.
fn named(value: &str) -> Option<Option<String>> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if matches!(v.to_ascii_lowercase().as_str(), "inherit" | "default" | "none" | "off") {
        return Some(None);
    }
    let ok = v.len() <= 64
        && v.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '[' | ']'));
    ok.then(|| Some(v.to_string()))
}

/// The model to set, the first level that says something: the pane's own,
/// then the setting, then the default. `None` when it says `inherit`.
pub fn resolve(per_agent: &str, setting: &str, default: &str) -> Option<String> {
    named(per_agent).or_else(|| named(setting)).or_else(|| named(default)).flatten()
}

/// Add `CLAUDE_CODE_SUBAGENT_MODEL` to a Claude agent's spawn environment,
/// unless its `cmd:env` already sets one. Not for other providers.
pub fn carry(env_vars: &mut HashMap<String, String>, block_meta: &MetaMapType) {
    let provider = meta_get_string(block_meta, "agentProvider", "claude");
    let is_claude = provider.trim().is_empty() || crate::backend::providers::resolve_provider_alias(&provider) == "claude";
    if !is_claude || env_vars.contains_key(ENV_KEY) {
        return;
    }
    let setting = crate::backend::providers::live_setting_str(SETTING_SUBAGENT).unwrap_or_default();
    if let Some(model) = resolve(&meta_get_string(block_meta, META_SUBAGENT, ""), &setting, DEFAULT_SUBAGENT_MODEL) {
        env_vars.insert(ENV_KEY.to_string(), model);
    }
}

/// [`SETTING_EXPLORE`] now, resolved: the model to write, or `None` for none.
pub fn explore_model() -> Option<String> {
    resolve("", &crate::backend::providers::live_setting_str(SETTING_EXPLORE).unwrap_or_default(), DEFAULT_EXPLORE_MODEL)
}

fn explore_definition(model: &str) -> String {
    format!(
        "---\nname: Explore\ndescription: Fast, read-only agent for searching and reading files and code. Use it to find where something is or how it works; it can't edit anything. Say how thorough to be: quick, medium or very thorough.\nmodel: {model}\ntools: Read, Glob, Grep, Bash\n---\n{MANAGED_MARK}\n\
         You are a search specialist. You only read: never create, edit, move or delete files, and run only commands that read (ls, cat, git log, git diff, git show, grep). \
         Find what you were asked for, start broad and narrow down, and look in more than one place when the first doesn't settle it. \
         Report what you found with absolute file paths and line numbers, briefly, and say what you did not find.\n"
    )
}

/// Write, or remove, the Explore definition in `config_dir`'s `agents/`
/// folder for `model` (`None` removes AgentMux's own). Leaves one a person
/// wrote. Returns whether the file changed.
pub fn seed_explore_agent(config_dir: &str, model: Option<&str>) -> io::Result<bool> {
    let path = Path::new(config_dir).join("agents").join("Explore.md");
    let existing = std::fs::read_to_string(&path).ok();
    if existing.as_deref().is_some_and(|t| !t.contains(MANAGED_MARK)) {
        return Ok(false);
    }
    match model {
        None => {
            if existing.is_none() {
                return Ok(false);
            }
            std::fs::remove_file(&path)?;
            Ok(true)
        }
        Some(model) => {
            let wanted = explore_definition(model);
            if existing.as_deref() == Some(wanted.as_str()) {
                return Ok(false);
            }
            std::fs::create_dir_all(path.parent().expect("a file path has a parent"))?;
            let tmp = path.with_extension("md.tmp");
            std::fs::write(&tmp, wanted)?;
            std::fs::rename(&tmp, &path)?;
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_level_that_says_something_wins() {
        assert_eq!(resolve("", "", "sonnet").as_deref(), Some("sonnet"), "the default");
        assert_eq!(resolve("", "opus", "sonnet").as_deref(), Some("opus"), "the setting over the default");
        assert_eq!(resolve("haiku", "opus", "sonnet").as_deref(), Some("haiku"), "the pane over the setting");
        assert_eq!(resolve("", "inherit", "sonnet"), None, "inherit sets nothing");
        assert_eq!(resolve("Off", "opus", "sonnet"), None, "a pane can turn it off");
        assert_eq!(resolve("", "claude-opus-5-5", "sonnet").as_deref(), Some("claude-opus-5-5"), "a model ID");
        assert_eq!(resolve("", "opus; rm -rf", "sonnet").as_deref(), Some("sonnet"), "not a model name: skipped");
    }

    fn meta(provider: Option<&str>, model: Option<&str>) -> MetaMapType {
        let mut m = MetaMapType::new();
        if let Some(p) = provider {
            m.insert("agentProvider".into(), serde_json::json!(p));
        }
        if let Some(v) = model {
            m.insert(META_SUBAGENT.into(), serde_json::json!(v));
        }
        m
    }

    #[test]
    fn a_claude_agent_gets_the_model_and_nothing_else_does() {
        let mut env = HashMap::new();
        carry(&mut env, &meta(Some("claude"), None));
        assert_eq!(env.get(ENV_KEY).map(String::as_str), Some("sonnet"));
        let mut env = HashMap::new();
        carry(&mut env, &meta(None, Some("opus")));
        assert_eq!(env.get(ENV_KEY).map(String::as_str), Some("opus"), "an unset provider is Claude; the pane's value wins");
        let mut env = HashMap::new();
        carry(&mut env, &meta(Some("codex"), None));
        assert!(env.is_empty(), "other providers are left alone");
        let mut env = HashMap::from([(ENV_KEY.to_string(), "haiku".to_string())]);
        carry(&mut env, &meta(Some("claude"), Some("opus")));
        assert_eq!(env.get(ENV_KEY).map(String::as_str), Some("haiku"), "an explicit cmd:env wins");
        let mut env = HashMap::new();
        carry(&mut env, &meta(Some("claude"), Some("inherit")));
        assert!(env.is_empty(), "inherit sets none");
    }

    #[test]
    fn the_explore_definition_follows_the_setting_and_never_replaces_a_persons() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().to_str().unwrap();
        let file = dir.path().join("agents").join("Explore.md");

        assert!(seed_explore_agent(cfg, Some("haiku")).unwrap());
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("name: Explore") && text.contains("model: haiku") && text.contains(MANAGED_MARK));
        assert!(!seed_explore_agent(cfg, Some("haiku")).unwrap(), "unchanged: no write");
        assert!(seed_explore_agent(cfg, Some("sonnet")).unwrap());
        assert!(std::fs::read_to_string(&file).unwrap().contains("model: sonnet"), "follows the setting");
        assert!(seed_explore_agent(cfg, None).unwrap(), "inherit removes AgentMux's own");
        assert!(!file.exists());
        assert!(!seed_explore_agent(cfg, None).unwrap(), "nothing to remove");

        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "---\nname: Explore\nmodel: opus\n---\nmine\n").unwrap();
        assert!(!seed_explore_agent(cfg, Some("haiku")).unwrap());
        assert!(!seed_explore_agent(cfg, None).unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "---\nname: Explore\nmodel: opus\n---\nmine\n", "a person's is untouched");
    }
}
