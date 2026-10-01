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

pub(crate) struct SeededLaunch {
    /// The argv to store as `cmd:args`.
    pub cli_args: Vec<String>,
    /// The value for `agent:runtime`, or `None` for a provider whose model the
    /// menu does not wire (the pane keeps whatever the menu falls back to).
    pub runtime: Option<Value>,
}

/// The value of `--flag value` or `--flag=value` in `flags`, if present.
fn flag_value<'a>(flags: &'a [String], names: &[&str]) -> Option<&'a str> {
    let mut it = flags.iter();
    while let Some(f) = it.next() {
        for n in names {
            if f == n {
                return it.next().map(String::as_str);
            }
            if let Some(v) = f.strip_prefix(&format!("{n}=")) {
                return Some(v);
            }
        }
    }
    None
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
            // `--effort` 400s on Haiku 4.5; the frontend skips it the same way.
            if flag_effort.is_none() && model != "haiku" {
                cli_args.extend(["--effort".to_string(), effort.clone()]);
            }
        }
        "codex" => {
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
            .filter(|m| provider_id != "codex" || !is_claude_model(m))
            .unwrap_or_else(|| default_model.to_string())
    });

    let mut out = args;
    match provider_id {
        "claude" => {
            if have_model.is_none() {
                out.extend(["--model".to_string(), model.clone()]);
            }
            if !have_effort && model != "haiku" {
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

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    fn after(args: &[String], flag: &str) -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
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
            "antigravity",
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

    #[test]
    fn it_leaves_alone_what_it_cannot_judge() {
        // not wired, unknown, no provider recorded, and an empty argv
        for provider in ["gemini", "kimi", "antigravity", "mystery", ""] {
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
