// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A container turn's CLI arguments: the one-shot shape a `docker exec` turn runs with.

/// Heal a container agent's argv IN PLACE when — and only when — it still
/// carries the persistent controller's flags.
///
/// A container agent runs one `docker exec` per turn: `container_spawn.rs`
/// writes the raw message, closes stdin, and reads until EOF. There is no
/// long-lived stdin and no control channel. The PERSISTENT argv is wrong for
/// that in three separate ways, and it is what stale blocks carry:
///
///   * `--input-format stream-json` — makes the CLI parse every stdin line as
///     a JSON envelope, so it meets the startup markdown and dies with
///     `JSON Parse error: Unrecognized token '#'`. This is the crash that kept
///     container agents from EVER starting (verified live, 2026-08-31).
///   * missing `-p` — the one-shot print flag. Without it the CLI is not in
///     the mode this path drives at all.
///   * `--permission-prompt-tool stdio` (+ a non-bypass `--permission-mode`) —
///     routes tool permissions through the control protocol, which only the
///     persistent controller speaks. A container turn has nothing to answer
///     `can_use_tool`, so it rejects or hangs on the first permission check.
///
/// An earlier cut of this deleted only `--input-format` and left the other two
/// (codex P1 on PR #2867) — that unblocks the immediate crash while leaving the
/// pane in the wrong CLI mode, which is a worse failure because it looks like
/// it works.
///
/// The cut after THAT rebuilt the argv wholesale from `launch_args`, which was
/// worse again (reagent P1 on the same PR): this runs on EVERY container turn,
/// not once per stale block, so a rebuild also threw away everything
/// `agent-model.ts` legitimately appends after the base args —
/// `agent.provider_flags` (user-configurable) and `--fork-session` (gated on
/// `providerId === "claude"`, not on `agentMode`, so container agents reach it).
/// It would have permanently broken both for container agents, on blocks whose
/// argv this fix had already assembled correctly.
///
/// So: subtract, don't rebuild. Remove the flags the PERSISTENT argv carries
/// that the one-shot argv does not, restore any one-shot flag that's missing,
/// and leave every other token — the user's `--model`/`--effort`, their
/// `provider_flags`, `--fork-session` — exactly where it was. Which flags are
/// "persistent-only" is derived from the provider catalog rather than hardcoded,
/// so a future flag moving between the two lists doesn't silently strand this.
///
/// An argv carrying NO persistent-only flag is returned untouched: it was built
/// by the fixed `selectLaunchArgs` path and there is nothing to heal.
///
/// The root cause is fixed at the source in `launch-args.ts`; this is the
/// self-heal for blocks ALREADY persisted with the bad argv, since `cmd:args`
/// lives in block meta and would otherwise stay wrong forever. Applied at the
/// point of use so a block that never re-runs `resync_controller` can't bypass
/// it.
pub(crate) fn container_argv(argv: Vec<String>, provider_id: &str) -> Vec<String> {
    let Some(provider) = crate::backend::providers::get_provider(provider_id) else {
        // Unknown provider — no catalog to diff against, so remove the one flag
        // that is outright fatal rather than guessing at the rest.
        return strip_flag_with_value(argv, "--input-format");
    };
    let Some(persistent) = provider.persistent_launch_args else {
        // Subprocess-shaped provider: its only argv IS the one-shot argv, so a
        // container block could never have been given persistent flags.
        return argv;
    };

    let one_shot = flags_with_arity(provider.launch_args);
    let persistent_only: Vec<(&str, bool)> = flags_with_arity(persistent)
        .into_iter()
        .filter(|(flag, _)| !one_shot.iter().any(|(f, _)| f == flag))
        .collect();

    // Already one-shot-shaped — the frontend assembled this argv, hands off.
    if !argv
        .iter()
        .any(|a| persistent_only.iter().any(|(f, _)| f == a))
    {
        return argv;
    }

    let mut out = argv;
    for (flag, takes_value) in &persistent_only {
        out = if *takes_value {
            strip_flag_with_value(out, flag)
        } else {
            out.into_iter().filter(|a| a != flag).collect()
        };
    }

    // Restore the one-shot flags the persistent argv never had (`-p` above all).
    // Prepended so the provider's own baseline keeps its leading position.
    let mut restored: Vec<String> = Vec::new();
    for (flag, takes_value) in &one_shot {
        if out.iter().any(|a| a == flag) {
            continue;
        }
        restored.push(flag.to_string());
        if *takes_value {
            if let Some(pos) = provider.launch_args.iter().position(|a| a == flag) {
                if let Some(v) = provider.launch_args.get(pos + 1) {
                    restored.push(v.to_string());
                }
            }
        }
    }
    restored.extend(out);
    restored
}

/// Split an args list into `(flag, takes_a_value)` pairs.
///
/// Arity is read off the list itself — a flag whose next token is not another
/// flag takes a value. That's what lets `container_argv` strip
/// `--permission-prompt-tool stdio` (two tokens) and `--verbose` (one)
/// correctly without a hardcoded table of every provider's flag shapes.
fn flags_with_arity(args: &'static [&'static str]) -> Vec<(&'static str, bool)> {
    let mut out = Vec::new();
    for (i, tok) in args.iter().enumerate() {
        if !tok.starts_with('-') {
            continue;
        }
        let takes_value = args.get(i + 1).is_some_and(|next| !next.starts_with('-'));
        out.push((*tok, takes_value));
    }
    out
}

/// Remove `flag` and the value token following it, everywhere it appears.
/// Tolerates a trailing `flag` with no value rather than panicking.
fn strip_flag_with_value(args: Vec<String>, flag: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == flag {
            let _ = it.next();
            continue;
        }
        out.push(a);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{container_argv, flags_with_arity, strip_flag_with_value};

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    /// The exact argv a container agent carried before the fix — the persistent
    /// controller's args, copied verbatim from the live broken block (Moras,
    /// 2026-08-31).
    fn stale_persistent_argv() -> Vec<String> {
        v(&[
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
            "--permission-mode",
            "default",
            "--model",
            "sonnet",
            "--effort",
            "high",
        ])
    }

    /// All three persistent-only defects must go, not just the fatal one
    /// (codex P1 on PR #2867): the parse crash, the missing one-shot `-p`, and
    /// the control-protocol permission flags a container turn cannot answer.
    #[test]
    fn rebuilds_a_stale_persistent_argv_into_the_one_shot_form() {
        let got = container_argv(stale_persistent_argv(), "claude");

        assert!(
            !got.iter().any(|a| a == "--input-format"),
            "the fatal parse flag must go"
        );
        assert!(
            got.iter().any(|a| a == "-p"),
            "one-shot print mode must be present"
        );
        assert!(
            !got.iter().any(|a| a == "--permission-prompt-tool"),
            "the container turn has no control channel to answer can_use_tool",
        );
        assert!(
            !got.iter().any(|a| a == "stdio"),
            "its value token must go with it"
        );
        assert!(
            !got.iter().any(|a| a == "--permission-mode"),
            "non-bypass permission mode needs the control protocol",
        );
        assert!(
            got.iter().any(|a| a == "--output-format"),
            "output-format stream-json is how the pane parses the turn at all",
        );
    }

    /// A user's model/effort selections must survive the heal — silently
    /// resetting someone's model on every stale pane would be its own bug.
    #[test]
    fn preserves_the_users_model_and_effort_choices() {
        let got = container_argv(stale_persistent_argv(), "claude");
        let pos = |f: &str| got.iter().position(|a| a == f);
        assert_eq!(got[pos("--model").expect("--model kept") + 1], "sonnet");
        assert_eq!(got[pos("--effort").expect("--effort kept") + 1], "high");
    }

    /// reagent P1 on PR #2867. This runs on EVERY container turn, not once per
    /// stale block, so it must not touch an argv the frontend already built
    /// correctly — a rebuild-from-baseline silently deleted `provider_flags`
    /// and `--fork-session` on every single turn, forever.
    #[test]
    fn leaves_an_already_correct_one_shot_argv_completely_untouched() {
        let correct = v(&[
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--dangerously-skip-permissions",
            "--exclude-dynamic-system-prompt-sections",
            "--model",
            "opus",
            "--my-custom-provider-flag",
            "42",             // agent.provider_flags
            "--fork-session", // resolveForkSessionArgs
        ]);
        assert_eq!(container_argv(correct.clone(), "claude"), correct);
    }

    /// …and when it DOES heal a stale argv, those same user-owned tokens still
    /// have to come through. Healing is subtraction, not reconstruction.
    #[test]
    fn a_heal_preserves_provider_flags_and_fork_session_too() {
        let mut stale = stale_persistent_argv();
        stale.extend(v(&["--my-custom-provider-flag", "42", "--fork-session"]));

        let got = container_argv(stale, "claude");

        assert!(
            got.iter().any(|a| a == "--my-custom-provider-flag"),
            "provider_flags survive"
        );
        assert_eq!(
            got[got
                .iter()
                .position(|a| a == "--my-custom-provider-flag")
                .unwrap()
                + 1],
            "42",
            "…with its value",
        );
        assert!(
            got.iter().any(|a| a == "--fork-session"),
            "fork-session survives"
        );
        assert!(
            !got.iter().any(|a| a == "--input-format"),
            "while still being healed"
        );
        assert!(got.iter().any(|a| a == "-p"));
    }

    /// An unknown provider has no baseline to diff against, so fall back to
    /// removing only the outright-fatal flag rather than guessing.
    #[test]
    fn an_unknown_provider_falls_back_to_removing_only_the_fatal_flag() {
        let got = container_argv(
            v(&["--input-format", "stream-json", "--custom"]),
            "no-such-provider",
        );
        assert_eq!(got, v(&["--custom"]));
    }

    /// A subprocess-shaped provider has no persistent argv, so nothing it was
    /// ever launched with could need healing.
    #[test]
    fn a_provider_with_no_persistent_variant_is_passed_through() {
        let argv = v(&["--json", "--whatever"]);
        assert_eq!(container_argv(argv.clone(), "codex"), argv);
    }

    /// Arity comes off the catalog, not a hardcoded table — this is what lets
    /// a valued flag drop its value and a bare flag not eat the next one.
    #[test]
    fn flags_with_arity_reads_valued_and_bare_flags_off_the_list() {
        static ARGS: &[&str] = &["-p", "--output-format", "stream-json", "--verbose"];
        assert_eq!(
            flags_with_arity(ARGS),
            vec![
                ("-p", false),
                ("--output-format", true),
                ("--verbose", false)
            ],
        );
    }

    #[test]
    fn strip_flag_with_value_removes_every_occurrence_and_tolerates_a_missing_value() {
        assert_eq!(
            strip_flag_with_value(v(&["--x", "1", "--keep", "--x", "2"]), "--x"),
            v(&["--keep"]),
        );
        assert_eq!(
            strip_flag_with_value(v(&["--keep", "--x"]), "--x"),
            v(&["--keep"])
        );
        assert_eq!(strip_flag_with_value(vec![], "--x"), Vec::<String>::new());
    }
}
