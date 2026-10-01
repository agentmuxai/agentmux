// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The runtime a pane opened by `agent.open` starts with.
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
//! This is the same decision the frontend makes at launch, for the two
//! providers whose model the menu wires (Claude, Codex). The defaults are
//! duplicated by necessity and pinned to the frontend's by
//! `providers/runtime-defaults-consistency.test.ts`.

use serde_json::{json, Value};

use crate::backend::obj::MetaMapType;
use crate::backend::providers::default_model_for;

/// `DEFAULT_RUNTIME_CONFIG.permissionMode` in `frontend/app/view/agent/types.ts`.
pub(super) const DEFAULT_PERMISSION_MODE: &str = "bypass";
/// `DEFAULT_RUNTIME_CONFIG.effort` in `frontend/app/view/agent/types.ts`.
pub(super) const DEFAULT_EFFORT: &str = "high";

/// Flags this module owns. Stripped from the catalog args so a base that ever
/// grows one cannot double it.
const OWNED_FLAGS: &[&str] = &["--model", "-m", "--effort"];

pub(super) struct SeededLaunch {
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
pub(super) fn seed_launch(
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

/// Write the three keys that must travel together into a new pane's meta:
/// `cmd:args` (what runs), `agent:runtime` (what the menu shows) and
/// `agent:provider_flags` (what the frontend's per-send rebuild reapplies — it
/// starts from the catalog and would drop the definition's flags on the first
/// send otherwise, #2872). One call, so no caller can write one without the
/// others.
pub(super) fn apply_to_meta(meta: &mut MetaMapType, seeded: SeededLaunch, provider_flags: &str) {
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
}
