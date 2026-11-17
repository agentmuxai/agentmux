// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `Agents update` with `model` or `effort` writes to a pane (the pick itself is `server::app_api::agent_runtime_pick`).

use serde_json::{json, Value};

use super::{flag_value, is_claude_model, without_flags, without_model_flags, EFFORT_LEVELS};
use crate::backend::obj::MetaMapType;

/// What `Agents update` with `model` or `effort` writes to a pane: the same two keys the Runtime menu writes
/// (`frontend/app/view/agent/runtime-apply.ts`).
pub struct RuntimeChoice {
    /// The value for `agent:runtime`: what the pane has, with the new model and effort laid over it.
    pub runtime: Value,
    /// The value for `cmd:args`: the stored argv without its own `--model` and `--effort`, so the next launch builds
    /// both from `runtime` ([`with_runtime_flags`]) and the two cannot disagree.
    pub cli_args: Option<Vec<String>>,
}

/// The pane's runtime once `model` and `effort` (each only when given) are changed. `keep_effort_flag` leaves a
/// `--effort` in the argv alone (the definition's own provider flags carry one).
pub fn runtime_choice(meta: &MetaMapType, model: Option<&str>, effort: Option<&str>, keep_effort_flag: bool) -> RuntimeChoice {
    let mut runtime = meta.get("agent:runtime").and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(model) = model {
        runtime.insert("model".to_string(), json!(model));
    }
    if let Some(effort) = effort {
        runtime.insert("effort".to_string(), json!(effort));
    }
    let cli_args = meta.get("cmd:args").and_then(Value::as_array).map(|a| {
        let args: Vec<String> = a.iter().filter_map(Value::as_str).map(str::to_string).collect();
        let args = without_model_flags(args);
        if keep_effort_flag { args } else { without_flags(args, &["--effort"]) }
    });
    RuntimeChoice { runtime: Value::Object(runtime), cli_args }
}

/// Whether the agent definition's own provider flags (`agent:provider_flags`) carry a model and an effort: that is
/// the operator's choice, which a runtime pick does not override (`with_runtime_flags`).
pub fn provider_flags_pin(meta: &MetaMapType) -> (bool, bool) {
    flags_pin(&crate::backend::obj::meta_get_string(meta, "agent:provider_flags", ""))
}

/// [`provider_flags_pin`] for the definition's flags as stored (`AgentDefinition::provider_flags`), which an agent with
/// no pane open has only here.
pub fn flags_pin(provider_flags: &str) -> (bool, bool) {
    let flags: Vec<String> = provider_flags.split_whitespace().map(str::to_string).collect();
    (flag_value(&flags, &["--model", "-m"]).is_some(), flag_value(&flags, &["--effort"]).is_some())
}

/// Whether `model` is one a Claude agent can be given: an alias (`opus`, `sonnet`, `haiku`) or a concrete id
/// (`claude-fable-5-2`), with no space or control character in it.
pub fn is_claude_model_name(model: &str) -> bool {
    is_claude_model(model) && !model.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The efforts a Claude agent takes.
pub fn is_claude_effort(effort: &str) -> bool {
    EFFORT_LEVELS.contains(&effort)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta_of(v: serde_json::Value) -> MetaMapType {
        v.as_object().unwrap().clone().into_iter().collect()
    }

    #[test]
    fn a_runtime_choice_lays_the_pick_over_the_runtime_and_takes_the_models_flags_out_of_the_argv() {
        let meta = meta_of(json!({
            "cmd:args": ["-p", "--model", "sonnet", "--effort=high", "--verbose", "-m", "x"],
            "agent:runtime": { "model": "sonnet", "effort": "high", "permissionMode": "plan" },
        }));
        let c = runtime_choice(&meta, Some("claude-fable-5-2"), None, false);
        assert_eq!(c.runtime, json!({ "model": "claude-fable-5-2", "effort": "high", "permissionMode": "plan" }));
        assert_eq!(c.cli_args.unwrap(), ["-p", "--verbose"]);
        let kept = runtime_choice(&meta, None, Some("max"), true).cli_args.unwrap();
        assert!(kept.contains(&"--effort=high".to_string()), "the definition's own effort flag stays: {kept:?}");
        let bare = runtime_choice(&meta_of(json!({})), Some("opus"), Some("low"), false);
        assert_eq!(bare.runtime, json!({ "model": "opus", "effort": "low" }));
        assert!(bare.cli_args.is_none(), "a pane with no stored argv is left without one");
    }

    #[test]
    fn the_definitions_own_model_and_effort_flags_are_seen_and_model_names_are_checked() {
        let pinned = meta_of(json!({ "agent:provider_flags": "--model opus --verbose" }));
        assert_eq!(provider_flags_pin(&pinned), (true, false));
        assert_eq!(provider_flags_pin(&meta_of(json!({ "agent:provider_flags": "--effort=low" }))), (false, true));
        assert_eq!(provider_flags_pin(&meta_of(json!({}))), (false, false));
        assert_eq!(flags_pin("--effort high -m opus"), (true, true));
        assert!(is_claude_model_name("claude-fable-5-2") && is_claude_model_name("sonnet"));
        assert!(!is_claude_model_name("gpt-5") && !is_claude_model_name("claude opus") && !is_claude_model_name(""));
        assert!(is_claude_effort("xhigh") && !is_claude_effort("ultra"));
    }
}
