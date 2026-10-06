// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The runtime flags a pane's CLI is given, decided in srv.
//!
//! The runtime menu shows `agent:runtime` block meta; the process runs on
//! `cmd:args`. The frontend keeps the two in step (`buildPaneArgs`), but
//! `agent.open` — MCP `OpenAgent`, layouts, `agent.define` + open — builds a
//! pane without the frontend. It wrote `cmd:args` from the catalog alone and no
//! `agent:runtime`, so the pane's CLI ran on its own default model (Opus 5.5 /
//! medium on current CLIs) while the menu fell back to Sonnet / high. A pane
//! with a session on file spawns eagerly at open, before any message, so
//! nothing ever corrected it.
//! docs/retro/RETRO_RESUMED_AGENT_SPAWNS_WITHOUT_RUNTIME_FLAGS_2026_09_30.md,
//! docs/reports/REPORT_AGENT_RUNTIME_BINDINGS_2026_09_30.md (G3).
//!
//! Two entry points, one decision, for the two providers whose model the menu
//! wires (Claude, Codex):
//!
//! - [`seed_launch`] — a pane `agent.open` is creating.
//! - [`with_runtime_flags`] — a pane that already exists, at the moment it
//!   spawns. srv never reads `agent:runtime` otherwise, and a persistent
//!   process never re-reads `cmd:args`, so a pane saved before the fixes (or
//!   written by any path that forgets) would run on the CLI default for its
//!   whole life. This is the backstop that makes srv authoritative at the one
//!   moment it matters.
//!
//! The defaults are duplicated from the frontend by necessity and pinned to
//! its by `providers/runtime-defaults-consistency.test.ts`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::backend::obj::MetaMapType;
use crate::backend::providers::default_model_for;

/// `DEFAULT_RUNTIME_CONFIG.permissionMode` in `frontend/app/view/agent/types.ts`.
pub(crate) const DEFAULT_PERMISSION_MODE: &str = "bypass";
/// `DEFAULT_RUNTIME_CONFIG.effort` in `frontend/app/view/agent/types.ts`.
pub(crate) const DEFAULT_EFFORT: &str = "high";

/// Flags this module owns. Stripped from the catalog args so a base that ever
/// grows one cannot double it.
const OWNED_FLAGS: &[&str] = &["--model", "-m", "--effort"];

/// What an agent process was actually spawned with: the runtime flags in its
/// final argv. The Runtime menu shows what was REQUESTED (`agent:runtime`);
/// this is what the process was GIVEN, so the menu can say when they differ.
/// `None` for a flag the argv does not carry — the CLI then uses its own default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnRuntime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
}

/// The runtime flags in `args`. The LAST occurrence of a repeated flag wins,
/// which is how the CLI is expected to read them (unverified; it matters only
/// when `provider_flags` repeats a flag the runtime already set).
/// `--dangerously-skip-permissions` reads as `bypass`; `--permission-mode X`
/// as `X`.
pub fn spawn_runtime_from_args(args: &[String]) -> SpawnRuntime {
    let mut out = SpawnRuntime::default();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let next = || args.get(i + 1).cloned();
        match a {
            "--model" | "-m" => {
                if let Some(v) = next() {
                    out.model = Some(v);
                    i += 1;
                }
            }
            "--effort" => {
                if let Some(v) = next() {
                    out.effort = Some(v);
                    i += 1;
                }
            }
            "--permission-mode" => {
                if let Some(v) = next() {
                    out.permission_mode = Some(v);
                    i += 1;
                }
            }
            "--dangerously-skip-permissions" => out.permission_mode = Some("bypass".to_string()),
            _ => {
                if let Some(v) = a.strip_prefix("--model=") {
                    out.model = Some(v.to_string());
                } else if let Some(v) = a.strip_prefix("--effort=") {
                    out.effort = Some(v.to_string());
                } else if let Some(v) = a.strip_prefix("--permission-mode=") {
                    out.permission_mode = Some(v.to_string());
                }
            }
        }
        i += 1;
    }
    out
}

/// What the running CLI says it is ACTUALLY using, from its `get_settings`
/// answer (`applied`): the alias on the command line resolved to a concrete
/// model, and the effort in force. `None` for a field the CLI reports as null
/// (Haiku has no effort).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectiveRuntime {
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// Prefix of the `request_id` of our `get_settings` requests, so the answer can
/// be told from the answers to anything else.
pub const SETTINGS_REQUEST_PREFIX: &str = "agentmux-settings-";

/// The control request asking a running Claude CLI what it is using. Observed
/// on CLI 2.1.285 to be answered straight after spawn, with no account and no
/// message (docs/specs/SPEC_RUNTIME_MENU_REMAINING_GAPS_2026_10_01.md §7.3).
pub fn settings_request_line() -> String {
    json!({
        "type": "control_request",
        "request_id": format!("{SETTINGS_REQUEST_PREFIX}{}", uuid::Uuid::new_v4()),
        "request": { "subtype": "get_settings" },
    })
    .to_string()
}

/// The effective runtime in a `get_settings` answer, or `None` for any other
/// frame, an error answer (a CLI that has no `get_settings`) or one without
/// `applied`. The shape is `{type: control_response, response: {subtype:
/// success, request_id, response: {applied: {model, effort, ...}}}}`.
pub fn effective_from_control_response(frame: &Value) -> Option<EffectiveRuntime> {
    if frame.get("type").and_then(Value::as_str) != Some("control_response") {
        return None;
    }
    let resp = frame.get("response")?;
    if resp.get("subtype").and_then(Value::as_str) != Some("success") {
        return None;
    }
    if !resp
        .get("request_id")
        .and_then(Value::as_str)
        .is_some_and(|id| id.starts_with(SETTINGS_REQUEST_PREFIX))
    {
        return None;
    }
    let applied = resp.get("response")?.get("applied")?.as_object()?;
    let field = |k: &str| {
        applied
            .get(k)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    Some(EffectiveRuntime { model: field("model"), effort: field("effort") })
}

/// The `agentruntime` event for one pane (persisted, so a menu that mounts late
/// still gets the latest). `running` is `None` when no process is alive — then
/// there is nothing for the menu to compare against. `effective` is what the
/// CLI reported using, when it has (see [`EffectiveRuntime`]).
pub fn agent_runtime_event(
    block_id: &str,
    running: Option<&SpawnRuntime>,
    restart_pending: bool,
    effective: Option<&EffectiveRuntime>,
) -> Value {
    let mut v = json!({
        "blockid": block_id,
        "running": running.is_some(),
        "restart_pending": restart_pending,
    });
    if let (Some(rt), Some(obj)) = (running, v.as_object_mut()) {
        if let Some(m) = &rt.model {
            obj.insert("model".to_string(), json!(m));
        }
        if let Some(e) = &rt.effort {
            obj.insert("effort".to_string(), json!(e));
        }
        if let Some(p) = &rt.permission_mode {
            obj.insert("permission_mode".to_string(), json!(p));
        }
    }
    if let (Some(eff), Some(obj)) = (effective.filter(|_| running.is_some()), v.as_object_mut()) {
        if let Some(m) = &eff.model {
            obj.insert("effective_model".to_string(), json!(m));
        }
        if let Some(e) = &eff.effort {
            obj.insert("effective_effort".to_string(), json!(e));
        }
    }
    v
}

pub fn publish_agent_runtime(
    broker: &crate::backend::mps::Broker,
    block_id: &str,
    running: Option<&SpawnRuntime>,
    restart_pending: bool,
    effective: Option<&EffectiveRuntime>,
) {
    use crate::backend::mps::{MuxEvent, EVENT_AGENT_RUNTIME};
    broker.publish(MuxEvent {
        event: EVENT_AGENT_RUNTIME.to_string(),
        scopes: vec![format!("block:{block_id}")],
        sender: String::new(),
        persist: 1,
        data: Some(agent_runtime_event(block_id, running, restart_pending, effective)),
    });
}

pub(crate) struct SeededLaunch {
    /// The argv to store as `cmd:args`.
    pub cli_args: Vec<String>,
    /// The value for `agent:runtime`, or `None` for a provider whose model the
    /// menu does not wire (the pane keeps whatever the menu falls back to).
    pub runtime: Option<Value>,
}

/// The value of `--flag value` or `--flag=value` in `flags`, if present; the last one when it repeats.
fn flag_value<'a>(flags: &'a [String], names: &[&str]) -> Option<&'a str> {
    // The LAST occurrence wins, as the CLI reads a repeated flag (observed on CLI
    // 2.1.285: `--model opus --model haiku` sends Haiku, and the reverse sends Opus;
    // the frontend's `modelFromFlags` agrees). Returning the first decided on Opus
    // and handed Haiku an `--effort` it does not take.
    let mut found = None;
    let mut it = flags.iter();
    while let Some(f) = it.next() {
        for n in names {
            if f == n {
                if let Some(v) = it.next() {
                    found = Some(v.as_str());
                }
                break;
            }
            if let Some(v) = f.strip_prefix(&format!("{n}=")) {
                found = Some(v);
                break;
            }
        }
    }
    found
}

/// `base` without any of [`OWNED_FLAGS`] (and their values).
fn without_owned_flags(base: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(base.len());
    let mut it = base.into_iter();
    while let Some(a) = it.next() {
        if OWNED_FLAGS.contains(&a.as_str()) {
            it.next();
            continue;
        }
        if OWNED_FLAGS.iter().any(|f| a.starts_with(&format!("{f}="))) {
            continue;
        }
        out.push(a);
    }
    out
}

/// What the user last picked in the Runtime menu for an agent (`last_runtime`
/// on `db_agents`), already validated. Each setting is optional: only what was
/// picked is here, and the rest keeps coming from the definition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Remembered {
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

const PERMISSION_MODES: &[&str] = &["bypass", "auto", "acceptEdits", "plan", "default"];
const EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
/// The flag a control-protocol (persistent) agent is launched with; its
/// presence in the catalog args is what makes `bypass` become `default`.
const CONTROL_PROTOCOL_FLAG: &str = "--permission-prompt-tool";

impl Remembered {
    /// Parse the stored JSON for `provider_id`. Only Claude is covered: its
    /// model namespace is known here (`opus` / `sonnet` / `haiku` or a concrete
    /// `claude-…` id), and anything else, or anything malformed, is dropped
    /// setting by setting.
    pub(crate) fn parse(provider_id: &str, raw: &str) -> Remembered {
        if provider_id != "claude" || raw.is_empty() {
            return Remembered::default();
        }
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(raw) else {
            return Remembered::default();
        };
        let get = |k: &str| obj.get(k).and_then(Value::as_str).filter(|v| !v.is_empty());
        Remembered {
            permission_mode: get("permissionMode")
                .filter(|m| PERMISSION_MODES.contains(m))
                .map(str::to_string),
            effort: get("effort").filter(|e| EFFORT_LEVELS.contains(e)).map(str::to_string),
            model: get("model")
                .filter(|m| matches!(*m, "opus" | "sonnet" | "haiku") || m.starts_with("claude-"))
                .map(str::to_string),
        }
    }

    fn is_empty(&self) -> bool {
        self.permission_mode.is_none() && self.model.is_none() && self.effort.is_none()
    }
}

/// `args` without the permission flags (`--permission-mode X`,
/// `--dangerously-skip-permissions`).
fn without_permission_flags(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--permission-mode" {
            it.next();
            continue;
        }
        if a == "--dangerously-skip-permissions" || a.starts_with("--permission-mode=") {
            continue;
        }
        out.push(a);
    }
    out
}

/// `flags` without the `names` flags (and their values), `--flag=value` too.
fn without_flags(flags: Vec<String>, names: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(flags.len());
    let mut it = flags.into_iter();
    while let Some(f) = it.next() {
        if names.contains(&f.as_str()) {
            it.next();
            continue;
        }
        if names.iter().any(|n| f.starts_with(&format!("{n}="))) {
            continue;
        }
        out.push(f);
    }
    out
}

/// Lay what the user last picked over the definition, the way the frontend's
/// launch does (`resolveLaunchRuntime`): the remembered setting beats the
/// definition's flag for it, and that flag is taken out of the pane's copy.
///
/// Returns `(base, pane_flags, permission_mode)`: the catalog args (their
/// permission flags replaced when a mode is remembered), the flags to hand to
/// [`seed_launch`] and to store as `agent:provider_flags`, and the mode the
/// menu must show (`None` = leave what `seed_launch` says).
pub(crate) fn apply_remembered(
    provider_id: &str,
    base: Vec<String>,
    provider_flags: &str,
    remembered: &Remembered,
) -> (Vec<String>, String, Option<String>) {
    if provider_id != "claude" || remembered.is_empty() {
        return (base, provider_flags.to_string(), None);
    }
    let mut flags: Vec<String> = provider_flags.split_whitespace().map(str::to_string).collect();
    let mut base = base;
    let mut front: Vec<String> = Vec::new();

    // The model that will run: remembered, else the definition's, else the default.
    let model = remembered
        .model
        .clone()
        .or_else(|| flag_value(&flags, &["--model", "-m"]).map(str::to_string));
    if let Some(m) = &remembered.model {
        flags = without_flags(flags, &["--model", "-m"]);
        front.extend(["--model".to_string(), m.clone()]);
    }
    if let Some(e) = &remembered.effort {
        flags = without_flags(flags, &["--effort"]);
        if model.as_deref().map_or(true, model_takes_effort) {
            front.extend(["--effort".to_string(), e.clone()]);
        }
    }
    // A model that takes no `--effort` must not be left with the definition's.
    if model.as_deref().is_some_and(|m| !model_takes_effort(m)) {
        flags = without_flags(flags, &["--effort"]);
    }

    let mut mode_for_menu = None;
    if let Some(mode) = &remembered.permission_mode {
        let control = base.iter().any(|a| a == CONTROL_PROTOCOL_FLAG);
        base = without_permission_flags(base);
        flags = without_permission_flags(flags);
        let wire = if control && mode == "bypass" { "default" } else { mode.as_str() };
        if wire == "bypass" {
            front.push("--dangerously-skip-permissions".to_string());
        } else {
            front.extend(["--permission-mode".to_string(), wire.to_string()]);
        }
        mode_for_menu = Some(mode.clone());
    }

    front.extend(flags);
    (base, front.join(" "), mode_for_menu)
}

/// Build `cmd:args` and `agent:runtime` for a freshly opened pane.
///
/// `base` is the catalog argv for the pane's controller. `provider_flags` is
/// the agent definition's own flags: they are appended last, exactly as the
/// frontend does, and a `--model` / `--effort` among them is the agent's own
/// choice — it becomes the runtime the menu shows and no second flag is added,
/// so the two cannot disagree.
pub(crate) fn seed_launch(
    provider_id: &str,
    base: Vec<String>,
    provider_flags: &str,
) -> SeededLaunch {
    let flags: Vec<String> = provider_flags
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let Some(default_model) = default_model_for(provider_id) else {
        let mut cli_args = base;
        cli_args.extend(flags);
        return SeededLaunch {
            cli_args,
            runtime: None,
        };
    };

    let flag_model = flag_value(&flags, &["--model", "-m"]);
    let flag_effort = flag_value(&flags, &["--effort"]);
    let model = flag_model.unwrap_or(default_model).to_string();
    let effort = flag_effort.unwrap_or(DEFAULT_EFFORT).to_string();

    let mut cli_args = without_owned_flags(base);
    // Codex reads its prompt from stdin via a trailing `-`, so it must stay last
    // however many flags land after the catalog args: held back here, put back
    // after the definition's own flags below. (The spawn path re-normalizes it
    // too, `build_codex_argv`, but the stored `cmd:args` should be right as is.)
    let marker = if provider_id == "codex" && cli_args.last().map(String::as_str) == Some("-") {
        cli_args.pop()
    } else {
        None
    };
    match provider_id {
        "claude" => {
            if flag_model.is_none() {
                cli_args.extend(["--model".to_string(), model.clone()]);
            }
            // Haiku takes no `--effort` (the pinned CLI drops it; older CLIs forwarded
            // it and the API answered 400); the frontend skips it the same way.
            if flag_effort.is_none() && model_takes_effort(&model) {
                cli_args.extend(["--effort".to_string(), effort.clone()]);
            }
        }
        "codex" | "antigravity" => {
            if flag_model.is_none() {
                cli_args.extend(["--model".to_string(), model.clone()]);
            }
        }
        _ => {}
    }
    cli_args.extend(flags);
    cli_args.extend(marker);

    SeededLaunch {
        cli_args,
        runtime: Some(json!({
            "permissionMode": DEFAULT_PERMISSION_MODE,
            "model": model,
            "effort": effort,
        })),
    }
}

/// Whether `--effort` is passed for `model`. Haiku takes none (the pinned CLI drops
/// it; older CLIs forwarded it and the API answered HTTP 400 on
/// every turn). Matches the model id as well as the alias, case-insensitively —
/// this was `model == "haiku"`, so a concrete Haiku id still got the flag.
/// Mirrors `modelTakesEffort` in frontend/app/view/agent/runtime-capabilities.ts.
pub(crate) fn model_takes_effort(model: &str) -> bool {
    !model.to_ascii_lowercase().contains("haiku")
}

/// Claude model names: the aliases and concrete ids. A pane carried over from
/// before per-provider models can have one stored for a Codex agent; it must
/// never reach Codex, which rejects it (frontend `buildRuntimeArgs.ts` guards the
/// same way).
fn is_claude_model(m: &str) -> bool {
    matches!(m, "opus" | "sonnet" | "haiku") || m.starts_with("claude-")
}

/// `args` as stored in `cmd:args`, plus any runtime flag it lacks, taken from
/// the pane's `agent:runtime` (or the defaults when it has none).
///
/// Never overrides a flag already present: the frontend's rebuild and
/// [`seed_launch`] put them there, and a definition's own `--model` is the
/// agent's choice. Leaves alone a provider the menu does not wire, an unknown
/// provider, and an empty argv (nothing to extend). Idempotent.
pub(crate) fn with_runtime_flags(meta: &MetaMapType, args: Vec<String>) -> Vec<String> {
    if args.is_empty() {
        return args;
    }
    // `agentProvider` is written from the definition's `provider` verbatim, and
    // that may be a legacy alias ("claude-code", "codex-cli"). Resolve it, or a
    // pane on an alias — exactly the old, pre-existing kind this backstop exists
    // for — would silently get nothing (ReAgent P1 on #4116). Unknown → "".
    let raw_provider = crate::backend::obj::meta_get_string(meta, "agentProvider", "");
    let provider_id = crate::backend::providers::resolve_provider_alias(&raw_provider);
    let Some(default_model) = default_model_for(provider_id) else {
        return args;
    };
    let runtime = |key: &str| {
        meta.get("agent:runtime")
            .and_then(|r| r.get(key))
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };

    let have_model = flag_value(&args, &["--model", "-m"]).map(str::to_string);
    let have_effort = flag_value(&args, &["--effort"]).is_some();
    let model = have_model.clone().unwrap_or_else(|| {
        runtime("model")
            .filter(|m| match provider_id {
                "codex" => !is_claude_model(m),
                // agy's own ids include `claude-sonnet-4-6`; only the bare
                // Claude aliases, carried over from a Claude pane, are foreign.
                "antigravity" => !matches!(m.as_str(), "opus" | "sonnet" | "haiku"),
                _ => true,
            })
            .unwrap_or_else(|| default_model.to_string())
    });

    let mut out = args;
    match provider_id {
        "claude" => {
            if have_model.is_none() {
                out.extend(["--model".to_string(), model.clone()]);
            }
            if !have_effort && model_takes_effort(&model) {
                let effort = runtime("effort").unwrap_or_else(|| DEFAULT_EFFORT.to_string());
                out.extend(["--effort".to_string(), effort]);
            }
        }
        "codex" => {
            if have_model.is_none() {
                // The stdin marker `-` must end up last even when the stored
                // argv has flags after it (provider_flags were appended past it).
                let marker = out.iter().rposition(|a| a == "-").map(|i| out.remove(i));
                out.extend(["--model".to_string(), model]);
                out.extend(marker);
            }
        }
        "antigravity" => {
            if have_model.is_none() {
                out.extend(["--model".to_string(), model]);
            }
        }
        _ => {}
    }
    out
}

/// Write the three keys that must travel together into a new pane's meta:
/// `cmd:args` (what runs), `agent:runtime` (what the menu shows) and
/// `agent:provider_flags` (what the frontend's per-send rebuild reapplies — it
/// starts from the catalog and would drop the definition's flags on the first
/// send otherwise, #2872). One call, so no caller can write one without the
/// others.
pub(crate) fn apply_to_meta(meta: &mut MetaMapType, seeded: SeededLaunch, provider_flags: &str) {
    meta.insert("cmd:args".to_string(), json!(seeded.cli_args));
    if let Some(runtime) = seeded.runtime {
        meta.insert("agent:runtime".to_string(), runtime);
    }
    meta.insert("agent:provider_flags".to_string(), json!(provider_flags));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ss(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn the_spawn_runtime_is_read_from_the_final_argv() {
        let rt = spawn_runtime_from_args(&ss(&[
            "--input-format",
            "stream-json",
            "--permission-mode",
            "default",
            "--model",
            "opus",
            "--effort",
            "max",
        ]));
        assert_eq!(rt.model.as_deref(), Some("opus"));
        assert_eq!(rt.effort.as_deref(), Some("max"));
        assert_eq!(rt.permission_mode.as_deref(), Some("default"));
    }

    #[test]
    fn a_flag_the_argv_lacks_is_none_which_means_the_cli_default() {
        let rt = spawn_runtime_from_args(&ss(&["--permission-mode", "default", "--resume", "sid"]));
        assert_eq!(rt.model, None);
        assert_eq!(rt.effort, None);
    }

    #[test]
    fn every_spelling_of_a_flag_is_read_and_the_last_one_wins() {
        assert_eq!(
            spawn_runtime_from_args(&ss(&["--model=opus"]))
                .model
                .as_deref(),
            Some("opus")
        );
        assert_eq!(
            spawn_runtime_from_args(&ss(&["-m", "gpt-5.5"]))
                .model
                .as_deref(),
            Some("gpt-5.5")
        );
        assert_eq!(
            spawn_runtime_from_args(&ss(&["--effort=low"]))
                .effort
                .as_deref(),
            Some("low")
        );
        assert_eq!(
            spawn_runtime_from_args(&ss(&["--model", "sonnet", "--model", "haiku"]))
                .model
                .as_deref(),
            Some("haiku")
        );
        assert_eq!(
            spawn_runtime_from_args(&ss(&["--dangerously-skip-permissions"]))
                .permission_mode
                .as_deref(),
            Some("bypass")
        );
        // a flag with no value is not a value
        assert_eq!(spawn_runtime_from_args(&ss(&["--model"])).model, None);
    }

    #[test]
    fn the_event_says_what_runs_and_whether_a_restart_is_coming() {
        let rt = SpawnRuntime {
            model: Some("sonnet".into()),
            effort: None,
            permission_mode: Some("default".into()),
        };
        let v = agent_runtime_event("b1", Some(&rt), true, None);
        assert_eq!(v["blockid"], "b1");
        assert_eq!(v["running"], true);
        assert_eq!(v["model"], "sonnet");
        assert_eq!(v["permission_mode"], "default");
        assert_eq!(v["restart_pending"], true);
        assert!(
            v.get("effort").is_none(),
            "an absent flag is omitted, not null"
        );

        let none = agent_runtime_event("b1", None, false, None);
        assert_eq!(none["running"], false);
        assert!(none.get("model").is_none());
        assert!(v.get("effective_model").is_none(), "nothing reported yet");
    }

    #[test]
    fn the_event_carries_what_the_cli_reported_only_while_a_process_runs() {
        let rt = SpawnRuntime { model: Some("sonnet".into()), ..Default::default() };
        let eff = EffectiveRuntime { model: Some("claude-sonnet-5-5".into()), effort: None };
        let v = agent_runtime_event("b1", Some(&rt), false, Some(&eff));
        assert_eq!(v["effective_model"], "claude-sonnet-5-5");
        assert!(v.get("effective_effort").is_none(), "a null effort (Haiku) is omitted");
        // a stale report must not outlive its process
        let gone = agent_runtime_event("b1", None, false, Some(&eff));
        assert!(gone.get("effective_model").is_none());
    }

    #[test]
    fn a_get_settings_answer_is_read_and_nothing_else_is() {
        let ok = json!({"type":"control_response","response":{
            "subtype":"success","request_id":"agentmux-settings-1",
            "response":{"applied":{"model":"claude-opus-5-5","effort":"medium","advisor":null}}}});
        assert_eq!(
            effective_from_control_response(&ok),
            Some(EffectiveRuntime { model: Some("claude-opus-5-5".into()), effort: Some("medium".into()) })
        );
        let haiku = json!({"type":"control_response","response":{
            "subtype":"success","request_id":"agentmux-settings-2",
            "response":{"applied":{"model":"claude-haiku-4-5-20251001","effort":null}}}});
        assert_eq!(effective_from_control_response(&haiku).unwrap().effort, None);

        // someone else's request id, an error answer, no `applied`, other frames
        for bad in [
            json!({"type":"control_response","response":{"subtype":"success","request_id":"other-1","response":{"applied":{"model":"x"}}}}),
            json!({"type":"control_response","response":{"subtype":"error","request_id":"agentmux-settings-3","error":"Unsupported"}}),
            json!({"type":"control_response","response":{"subtype":"success","request_id":"agentmux-settings-4","response":{}}}),
            json!({"type":"control_request","request_id":"agentmux-settings-5","request":{"subtype":"get_settings"}}),
            json!({"type":"result"}),
        ] {
            assert_eq!(effective_from_control_response(&bad), None, "{bad}");
        }
    }

    #[test]
    fn the_settings_request_is_a_get_settings_control_request() {
        let v: Value = serde_json::from_str(&settings_request_line()).unwrap();
        assert_eq!(v["type"], "control_request");
        assert_eq!(v["request"]["subtype"], "get_settings");
        assert!(v["request_id"].as_str().unwrap().starts_with(SETTINGS_REQUEST_PREFIX));
        assert_ne!(settings_request_line(), settings_request_line(), "each request has its own id");
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    fn after(args: &[String], flag: &str) -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    }

    // ---- what the user last picked ----

    fn remembered(json: &str) -> Remembered {
        Remembered::parse("claude", json)
    }
    fn persistent_base() -> Vec<String> {
        s(&["-p", "--input-format", "stream-json", "--permission-prompt-tool", "stdio", "--permission-mode", "default"])
    }

    #[test]
    fn remembered_is_parsed_setting_by_setting_and_only_for_claude() {
        let r = remembered(r#"{"model":"opus","effort":"xhigh","permissionMode":"plan"}"#);
        assert_eq!(r.model.as_deref(), Some("opus"));
        assert_eq!(r.effort.as_deref(), Some("xhigh"));
        assert_eq!(r.permission_mode.as_deref(), Some("plan"));
        assert_eq!(remembered(r#"{"model":"claude-fable-5-1"}"#).model.as_deref(), Some("claude-fable-5-1"));
        // invalid settings are dropped on their own, the rest survive
        let r = remembered(r#"{"model":"gpt-5","effort":"ludicrous","permissionMode":"plan"}"#);
        assert_eq!((r.model, r.effort), (None, None));
        assert_eq!(r.permission_mode.as_deref(), Some("plan"));
        for bad in ["", "not json", "[]", "5", "null"] {
            assert_eq!(remembered(bad), Remembered::default(), "{bad:?}");
        }
        assert_eq!(Remembered::parse("codex", r#"{"model":"opus"}"#), Remembered::default());
    }

    #[test]
    fn nothing_remembered_changes_nothing() {
        let (base, flags, mode) = apply_remembered("claude", persistent_base(), "--model opus --add-dir /x", &Remembered::default());
        assert_eq!(base, persistent_base());
        assert_eq!(flags, "--model opus --add-dir /x");
        assert_eq!(mode, None);
    }

    #[test]
    fn a_remembered_model_and_effort_beat_the_definitions_flags() {
        let r = remembered(r#"{"model":"sonnet","effort":"low"}"#);
        let (base, flags, mode) = apply_remembered("claude", persistent_base(), "--model opus --effort max --add-dir /x", &r);
        assert_eq!(flags, "--model sonnet --effort low --add-dir /x");
        assert_eq!(mode, None);
        // and seed_launch then shows exactly that, with no second flag
        let out = seed_launch("claude", base, &flags);
        let rt = out.runtime.unwrap();
        assert_eq!((rt["model"].as_str(), rt["effort"].as_str()), (Some("sonnet"), Some("low")));
        assert_eq!(out.cli_args.iter().filter(|a| *a == "--model").count(), 1);
        assert_eq!(out.cli_args.iter().filter(|a| *a == "--effort").count(), 1);
    }

    #[test]
    fn a_setting_not_picked_keeps_the_definitions_flag() {
        let r = remembered(r#"{"model":"sonnet"}"#);
        let (_, flags, _) = apply_remembered("claude", persistent_base(), "--model opus --effort max", &r);
        assert_eq!(flags, "--model sonnet --effort max");
    }

    #[test]
    fn haiku_is_never_left_with_an_effort() {
        let r = remembered(r#"{"model":"haiku","effort":"high"}"#);
        let (_, flags, _) = apply_remembered("claude", persistent_base(), "--effort max", &r);
        assert_eq!(flags, "--model haiku");
        // remembered Haiku, definition's effort only
        let r = remembered(r#"{"model":"haiku"}"#);
        let (_, flags, _) = apply_remembered("claude", persistent_base(), "--model opus --effort max", &r);
        assert_eq!(flags, "--model haiku");
    }

    #[test]
    fn a_remembered_mode_replaces_the_catalog_and_definition_mode_flags() {
        let r = remembered(r#"{"permissionMode":"plan"}"#);
        let (base, flags, mode) = apply_remembered("claude", persistent_base(), "--dangerously-skip-permissions --add-dir /x", &r);
        assert!(!base.iter().any(|a| a == "--permission-mode"), "{base:?}");
        assert_eq!(flags, "--permission-mode plan --add-dir /x");
        assert_eq!(mode.as_deref(), Some("plan"));
        let out = seed_launch("claude", base, &flags);
        assert_eq!(after(&out.cli_args, "--permission-mode").as_deref(), Some("plan"));
        assert_eq!(out.cli_args.iter().filter(|a| *a == "--permission-mode").count(), 1);
        assert!(!out.cli_args.iter().any(|a| a == "--dangerously-skip-permissions"));
    }

    #[test]
    fn bypass_on_a_control_protocol_agent_is_default_on_the_wire_but_bypass_in_the_menu() {
        let r = remembered(r#"{"permissionMode":"bypass"}"#);
        let (_, flags, mode) = apply_remembered("claude", persistent_base(), "", &r);
        assert_eq!(flags, "--permission-mode default");
        assert_eq!(mode.as_deref(), Some("bypass"));
        // a one-shot agent (no control protocol) gets the real bypass flag
        let one_shot = s(&["-p", "--output-format", "stream-json"]);
        let (_, flags, _) = apply_remembered("claude", one_shot, "", &r);
        assert_eq!(flags, "--dangerously-skip-permissions");
    }

    #[test]
    fn other_providers_are_left_alone() {
        let r = Remembered { model: Some("opus".into()), ..Default::default() };
        let (base, flags, mode) = apply_remembered("codex", s(&["exec", "-"]), "--x", &r);
        assert_eq!((base, flags.as_str(), mode), (s(&["exec", "-"]), "--x", None));
    }

    #[test]
    fn claude_gets_the_models_and_effort_its_menu_will_show() {
        let out = seed_launch("claude", s(&["--permission-mode", "default"]), "");
        assert_eq!(after(&out.cli_args, "--model").as_deref(), Some("sonnet"));
        assert_eq!(after(&out.cli_args, "--effort").as_deref(), Some("high"));
        let rt = out.runtime.unwrap();
        assert_eq!(rt["model"], "sonnet");
        assert_eq!(rt["effort"], "high");
        assert_eq!(rt["permissionMode"], "bypass");
    }

    #[test]
    fn the_args_and_the_runtime_always_agree() {
        for (provider, flags) in [
            ("claude", ""),
            ("claude", "--model opus --effort max"),
            ("claude", "--model=claude-fable-5-1"),
            ("claude", "--model haiku"),
            ("codex", ""),
            ("codex", "--model gpt-5.4"),
        ] {
            let base = if provider == "codex" {
                s(&["exec", "--json", "-"])
            } else {
                s(&["--permission-mode", "default"])
            };
            let out = seed_launch(provider, base, flags);
            let rt = out.runtime.unwrap();
            let in_args = out
                .cli_args
                .iter()
                .enumerate()
                .find_map(|(i, a)| match a.as_str() {
                    "--model" => out.cli_args.get(i + 1).cloned(),
                    _ => a.strip_prefix("--model=").map(str::to_string),
                })
                .unwrap();
            assert_eq!(rt["model"], in_args, "{provider} {flags:?}");
        }
    }

    // ReAgent P2 on #4152. A repeated flag: the LAST wins, as the CLI reads it
    // (observed on 2.1.285) and as the frontend's modelFromFlags does. flag_value
    // used to return the first, so `--model opus --model haiku` was decided as Opus
    // (and given --effort) while the CLI ran Haiku, which takes no effort.
    #[test]
    fn a_repeated_flag_is_decided_on_the_last_one_as_the_cli_reads_it() {
        let out = seed_launch("claude", s(&[]), "--model opus --model haiku");
        assert!(
            !out.cli_args.iter().any(|a| a == "--effort"),
            "{:?}",
            out.cli_args
        );
        assert_eq!(out.runtime.unwrap()["model"], "haiku");

        let meta = meta_with("claude", None);
        let filled = with_runtime_flags(&meta, s(&["--model", "opus", "--model", "haiku"]));
        assert!(!filled.iter().any(|a| a == "--effort"), "{filled:?}");

        // and the other way round: the last model takes effort
        let out = seed_launch("claude", s(&[]), "--model haiku --model opus");
        assert_eq!(after(&out.cli_args, "--effort").as_deref(), Some("high"));

        // a repeated --effort: the last one is the definition's choice
        let out = seed_launch("claude", s(&[]), "--effort low --effort max");
        assert_eq!(out.runtime.unwrap()["effort"], "max");
    }

    #[test]
    fn effort_is_for_models_that_take_it_whatever_the_haiku_id_looks_like() {
        for haiku in [
            "haiku",
            "Haiku",
            "claude-haiku-4-5",
            "claude-haiku-4-5-20251001",
        ] {
            assert!(!model_takes_effort(haiku), "{haiku}");
            // seeded
            let seeded = seed_launch("claude", s(&[]), &format!("--model {haiku}"));
            assert!(
                !seeded.cli_args.iter().any(|a| a == "--effort"),
                "{haiku}: {:?}",
                seeded.cli_args
            );
            // filled into a stored pane
            let meta = meta_with("claude", Some(json!({"model": haiku})));
            let filled = with_runtime_flags(&meta, s(&["-p"]));
            assert!(
                !filled.iter().any(|a| a == "--effort"),
                "{haiku}: {filled:?}"
            );
        }
        for other in [
            "sonnet",
            "opus",
            "claude-sonnet-5-5",
            "claude-opus-5-5",
            "claude-fable-5-1",
        ] {
            assert!(model_takes_effort(other), "{other}");
        }
    }

    #[test]
    fn haiku_gets_no_effort_flag() {
        let out = seed_launch("claude", s(&[]), "--model haiku");
        assert!(
            !out.cli_args.iter().any(|a| a == "--effort"),
            "{:?}",
            out.cli_args
        );
    }

    #[test]
    fn a_definitions_own_model_is_the_runtime_and_is_not_doubled() {
        let out = seed_launch("claude", s(&[]), "--model opus --effort max");
        assert_eq!(
            out.cli_args.iter().filter(|a| *a == "--model").count(),
            1,
            "{:?}",
            out.cli_args
        );
        assert_eq!(
            out.cli_args.iter().filter(|a| *a == "--effort").count(),
            1,
            "{:?}",
            out.cli_args
        );
        let rt = out.runtime.unwrap();
        assert_eq!(rt["model"], "opus");
        assert_eq!(rt["effort"], "max");
    }

    #[test]
    fn provider_flags_stay_last_as_in_the_frontend() {
        let out = seed_launch(
            "claude",
            s(&["--permission-mode", "default"]),
            "--add-dir /tmp",
        );
        assert_eq!(
            out.cli_args[out.cli_args.len() - 2..],
            s(&["--add-dir", "/tmp"])
        );
    }

    #[test]
    fn codex_keeps_its_stdin_marker_last() {
        let out = seed_launch("codex", s(&["exec", "--json", "-"]), "");
        assert_eq!(
            out.cli_args,
            s(&["exec", "--json", "--model", "gpt-5.5", "-"])
        );
    }

    #[test]
    fn codex_keeps_its_stdin_marker_last_whatever_the_definition_adds() {
        // ReAgent P1 on #4114: a definition's own --model (flag_model is Some)
        // skipped the old marker handling and left `-` mid-argv.
        for flags in [
            "--model gpt-5.4",
            "-m gpt-5.4",
            "--model=gpt-5.4",
            "--add-dir /tmp",
            "--model gpt-5.4 --add-dir /tmp",
            "",
        ] {
            let out = seed_launch("codex", s(&["exec", "--json", "-"]), flags);
            assert_eq!(
                out.cli_args.last().map(String::as_str),
                Some("-"),
                "{flags:?}: {:?}",
                out.cli_args
            );
            assert_eq!(
                out.cli_args.iter().filter(|a| *a == "-").count(),
                1,
                "{flags:?}: {:?}",
                out.cli_args
            );
        }
    }

    #[test]
    fn a_codex_definitions_model_is_the_runtime_with_the_marker_last() {
        let out = seed_launch("codex", s(&["exec", "--json", "-"]), "--model gpt-5.4");
        assert_eq!(
            out.cli_args,
            s(&["exec", "--json", "--model", "gpt-5.4", "-"])
        );
        assert_eq!(out.runtime.unwrap()["model"], "gpt-5.4");
    }

    #[test]
    fn a_catalog_base_that_grows_a_model_flag_cannot_double_it() {
        let out = seed_launch(
            "claude",
            s(&["--model", "opus", "--permission-mode", "default"]),
            "",
        );
        assert_eq!(after(&out.cli_args, "--model").as_deref(), Some("sonnet"));
        assert_eq!(out.cli_args.iter().filter(|a| *a == "--model").count(), 1);
    }

    #[test]
    fn a_provider_the_menu_does_not_wire_is_left_alone() {
        for id in [
            "gemini",
            "kimi",
            "qwen",
            "openclaw",
            "copilot",
            "pi",
            "muxcode",
        ] {
            let out = seed_launch(id, s(&["--yolo"]), "--x y");
            assert!(out.runtime.is_none(), "{id}");
            assert_eq!(out.cli_args, s(&["--yolo", "--x", "y"]), "{id}");
        }
    }

    #[test]
    fn the_meta_a_pane_opens_with_has_all_three_keys_in_agreement() {
        let mut meta = MetaMapType::new();
        let seeded = seed_launch(
            "claude",
            s(&["--permission-mode", "default"]),
            "--model opus",
        );
        apply_to_meta(&mut meta, seeded, "--model opus");
        let args: Vec<String> = serde_json::from_value(meta["cmd:args"].clone()).unwrap();
        assert_eq!(after(&args, "--model").as_deref(), Some("opus"));
        assert_eq!(meta["agent:runtime"]["model"], "opus");
        assert_eq!(meta["agent:provider_flags"], "--model opus");
    }

    #[test]
    fn a_provider_without_a_runtime_still_records_its_flags() {
        let mut meta = MetaMapType::new();
        apply_to_meta(
            &mut meta,
            seed_launch("gemini", s(&["--yolo"]), "--x y"),
            "--x y",
        );
        assert!(!meta.contains_key("agent:runtime"));
        assert_eq!(meta["agent:provider_flags"], "--x y");
    }

    fn meta_with(provider: &str, runtime: Option<Value>) -> MetaMapType {
        let mut m = MetaMapType::new();
        m.insert("agentProvider".to_string(), json!(provider));
        if let Some(r) = runtime {
            m.insert("agent:runtime".to_string(), r);
        }
        m
    }

    #[test]
    fn a_stored_pane_with_no_flags_gets_what_its_menu_shows() {
        let meta = meta_with(
            "claude",
            Some(json!({"model": "opus", "effort": "max", "permissionMode": "bypass"})),
        );
        let out = with_runtime_flags(&meta, s(&["--permission-mode", "default"]));
        assert_eq!(after(&out, "--model").as_deref(), Some("opus"));
        assert_eq!(after(&out, "--effort").as_deref(), Some("max"));
    }

    #[test]
    fn a_pane_with_no_runtime_at_all_gets_the_defaults() {
        let out = with_runtime_flags(
            &meta_with("claude", None),
            s(&["--permission-mode", "default"]),
        );
        assert_eq!(after(&out, "--model").as_deref(), Some("sonnet"));
        assert_eq!(after(&out, "--effort").as_deref(), Some("high"));
    }

    #[test]
    fn flags_already_present_are_never_overridden_or_doubled() {
        let meta = meta_with("claude", Some(json!({"model": "sonnet", "effort": "low"})));
        let args = s(&["--model", "opus", "--effort", "max"]);
        assert_eq!(with_runtime_flags(&meta, args.clone()), args);
    }

    #[test]
    fn a_model_flag_without_an_effort_still_gets_the_runtimes_effort() {
        let meta = meta_with("claude", Some(json!({"model": "sonnet", "effort": "low"})));
        let out = with_runtime_flags(&meta, s(&["--model", "opus"]));
        assert_eq!(after(&out, "--model").as_deref(), Some("opus"));
        assert_eq!(after(&out, "--effort").as_deref(), Some("low"));
    }

    #[test]
    fn haiku_gets_no_effort_whether_it_comes_from_the_args_or_the_runtime() {
        let from_args = with_runtime_flags(&meta_with("claude", None), s(&["--model", "haiku"]));
        assert!(!from_args.iter().any(|a| a == "--effort"), "{from_args:?}");
        let from_runtime = with_runtime_flags(
            &meta_with("claude", Some(json!({"model": "haiku"}))),
            s(&["-p"]),
        );
        assert!(
            !from_runtime.iter().any(|a| a == "--effort"),
            "{from_runtime:?}"
        );
        assert_eq!(after(&from_runtime, "--model").as_deref(), Some("haiku"));
    }

    #[test]
    fn it_is_idempotent() {
        let meta = meta_with("claude", Some(json!({"model": "opus", "effort": "max"})));
        let once = with_runtime_flags(&meta, s(&["--permission-mode", "default"]));
        assert_eq!(with_runtime_flags(&meta, once.clone()), once);
    }

    #[test]
    fn codex_gets_its_model_before_the_stdin_marker() {
        let meta = meta_with("codex", Some(json!({"model": "gpt-5.4"})));
        let out = with_runtime_flags(&meta, s(&["exec", "--json", "-"]));
        assert_eq!(out, s(&["exec", "--json", "--model", "gpt-5.4", "-"]));
    }

    #[test]
    fn codex_keeps_its_stdin_marker_last_even_when_flags_sit_after_it() {
        let meta = meta_with("codex", Some(json!({"model": "gpt-5.4"})));
        let out = with_runtime_flags(&meta, s(&["exec", "--json", "-", "--add-dir", "/tmp"]));
        assert_eq!(
            out,
            s(&[
                "exec",
                "--json",
                "--add-dir",
                "/tmp",
                "--model",
                "gpt-5.4",
                "-"
            ])
        );
    }

    #[test]
    fn a_claude_model_stored_on_a_codex_pane_never_reaches_codex() {
        for stale in ["opus", "sonnet", "haiku", "claude-sonnet-5-5"] {
            let meta = meta_with("codex", Some(json!({ "model": stale })));
            let out = with_runtime_flags(&meta, s(&["exec", "-"]));
            assert_eq!(
                after(&out, "--model").as_deref(),
                Some("gpt-5.5"),
                "{stale}"
            );
        }
    }

    #[test]
    fn a_pane_recorded_under_a_legacy_provider_alias_is_still_filled() {
        // ReAgent P1 on #4116: `agentProvider` is the definition's provider as typed.
        for alias in ["claude-code", "claude_code"] {
            let meta = meta_with(alias, Some(json!({"model": "opus", "effort": "max"})));
            let out = with_runtime_flags(&meta, s(&["--permission-mode", "default"]));
            assert_eq!(after(&out, "--model").as_deref(), Some("opus"), "{alias}");
            assert_eq!(after(&out, "--effort").as_deref(), Some("max"), "{alias}");
        }
        let meta = meta_with("codex-cli", Some(json!({"model": "gpt-5.4"})));
        assert_eq!(
            with_runtime_flags(&meta, s(&["exec", "--json", "-"])),
            s(&["exec", "--json", "--model", "gpt-5.4", "-"])
        );
    }

    /// agy takes `--model` (SPEC_ANTIGRAVITY_HARNESS_REAL_CLI_2026_10_06.md).
    /// A bare Claude alias left in a pane's runtime is not an agy model; agy's
    /// own `claude-…` ids are.
    #[test]
    fn antigravity_gets_its_model_and_never_a_bare_claude_alias() {
        let seeded = seed_launch("antigravity", s(&["--output-format", "stream-json"]), "");
        assert_eq!(after(&seeded.cli_args, "--model").as_deref(), Some("gemini-3.8-flash-medium"));
        assert_eq!(seeded.runtime.unwrap()["model"], "gemini-3.8-flash-medium");

        let base = s(&["--output-format", "stream-json"]);
        let model = |m: &str| {
            let out = with_runtime_flags(&meta_with("antigravity", Some(json!({ "model": m }))), base.clone());
            after(&out, "--model")
        };
        assert_eq!(model("gemini-3.1-pro-high").as_deref(), Some("gemini-3.1-pro-high"));
        assert_eq!(model("claude-sonnet-4-6").as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(model("sonnet").as_deref(), Some("gemini-3.8-flash-medium"));
        // No --effort: agy has one (low/medium/high), but the runtime menu does
        // not wire it for Antigravity yet.
        let out = with_runtime_flags(&meta_with("antigravity", None), base.clone());
        assert!(!out.iter().any(|a| a == "--effort"), "{out:?}");
    }

    #[test]
    fn it_leaves_alone_what_it_cannot_judge() {
        // not wired, unknown, no provider recorded, and an empty argv
        for provider in ["gemini", "kimi", "mystery", ""] {
            let args = s(&["--yolo"]);
            assert_eq!(
                with_runtime_flags(&meta_with(provider, None), args.clone()),
                args,
                "{provider:?}"
            );
        }
        assert!(with_runtime_flags(&meta_with("claude", None), vec![]).is_empty());
    }
}
