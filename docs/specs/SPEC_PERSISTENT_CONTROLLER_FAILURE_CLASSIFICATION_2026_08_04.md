# SPEC — Wire failure classification + auto-retry into the persistent controller (Claude 429/overloaded)

**Date:** 2026-08-04
**Type:** Bug fix (regression, not a new feature)
**Status:** Proposed
**Owner:** Agent3
**Scope:** `agentmux-srv/src/backend/blockcontroller/persistent.rs`
**Related:** `SPEC_AGENT_FAILURE_DIAGNOSTICS_2026_06_11.md` (the classifier),
`SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md` (the recovery banner + auto-retry
policy this spec wires into — nothing in that spec's design changes here),
`SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20.md` (the durability layer this reuses).

## Problem

When Claude's API returns `429`/rate-limited or `529`/overloaded and the
`claude` CLI surfaces that error, the agent pane does **not** show the
recovery banner or auto-retry — the turn just ends and gets treated as a
normal completion. This reproduces the exact symptom reported: "sometimes it
returns too busy [and] currently it just quits."

**"Quits" here means the turn silently completes, not that the app or the CLI
process crashes.** `frontend/app/view/agent/hooks/useTurnLifecycle.ts:113`
dispatches `TurnEnd` unconditionally whenever a `session_end` frame arrives,
and the Claude translator always emits `session_end` alongside an
`error_result` event (`claude-translator.ts`'s "Case 5a") — so the pane lands
in phase `Done, outcome:"completed"` regardless of whether the turn actually
succeeded. The only visible trace is one inline red `error_result` line in
the transcript; nothing offers a retry.

## Root cause: this is a regression, not a missing feature

The failure-recovery framework already exists, is already fully built, and
already works correctly — **for every provider except Claude**:

- `agentmux-srv/src/agents/failure.rs::classify()` — a pure function that
  takes exit code / signal / stderr tail / an optional terminal `result`
  frame and returns a classified `AgentFailure { code, title, detail,
  retryable, .. }`. Already matches `"rate limited"` / `"429"` →
  `FailureClass::RateLimited` and `"overloaded"` / `"529"` →
  `FailureClass::Overloaded`, both `retryable: true`
  (`failure.rs:161-186`). Its own doc comment already anticipates the
  mid-stream case this spec needs: *"the CLI sometimes reports an error
  [in the result frame] while still exiting 0, so it is folded into the
  evidence."*
- `agentmux-srv/src/backend/blockcontroller/core.rs::persist_last_failure()`
  — writes the classified failure into block meta (survives tab
  switches/reloads) and is the gate before the WPS publish below.
- `wps::EVENT_AGENT_FAILURE` — the per-block event the frontend subscribes
  to.
- `frontend/app/view/agent/hooks/useAgentFailure.ts` — subscribes to that
  event, and for `isTransient(code)` classes (`rate_limited` / `overloaded`
  / `network`, `failure-accessory.ts:79-81`) already arms a 5s → 10s
  auto-retry countdown, capped at 2 attempts
  (`AUTO_RETRY_BACKOFF_S = [5, 10]`, `useAgentFailure.ts:44`), plus a
  manual Retry action beyond that. This is genuinely "best practice" retry
  UX (bounded, exponential-ish, user-visible, cancelable) — nothing about
  the *policy* needs to change.

All four pieces are wired together correctly today in
`agentmux-srv/src/backend/blockcontroller/subprocess/host_spawn.rs`
(`SubprocessController`, used by Codex and other non-Claude providers):
`host_spawn.rs:603-687` classifies on exit (covering both a genuine non-zero
exit *and* an error `result` frame arriving with exit 0), persists, and
publishes — every step this spec needs, already correct, already reviewed,
already shipped.

**The gap:** Claude Code is hard-wired to a different controller,
`ControllerType::Persistent` (`providers.rs:518-522`, chosen specifically so
`AskUserQuestion` can block mid-turn without the process exiting between
turns). `persistent.rs` — the only controller Claude actually runs on — never
calls `classify()`, never calls `persist_last_failure()`, and never
publishes `EVENT_AGENT_FAILURE`, anywhere. `SPEC_AGENT_FAILURE_RECOVERY_UI`
was designed and implemented against `subprocess.rs` (this file's earlier
name, before the persistent/subprocess split) — the persistent controller
either didn't exist yet or wasn't the assumed target, and the wiring was
never carried over when Claude moved onto it. Every other provider still on
`SubprocessController` already gets the recovery banner and auto-retry;
Claude is the one silent gap, and Claude is the highest-traffic provider in
the app.

## Design

Two insertion points in `persistent.rs`, mirroring `host_spawn.rs`'s
existing pattern exactly — no new policy, no new event, no frontend
changes. Both must compose correctly with `persistent.rs`'s own **existing**
stale-`--resume` auto-retry machinery (`persistent_resume`), which is a
different, unrelated retry mechanism (recovers from a stale `--resume
<session_id>` by respawning fresh) that must keep working exactly as it
does today — the new classify-and-publish calls fire only on errors that
mechanism has already decided are *not* being silently retried.

### 1. Mid-stream: an error `result` frame while the process stays alive

`persistent.rs:2267-2421` already computes `is_error_result` (a terminal
`result` frame with `is_error:true`) and, via
`apply_resume_event(ErrorResultLine{..})`, already computes
`hold_back_for_resume_retry` — `true` exactly when this same error is still
a live candidate for the stale-`--resume` auto-retry (in which case it must
not be surfaced to the user at all; see `PersistImmediately`/`FlushErrorLine`
vs. that state machine's other paths). Add, right after that block resolves
(after `persistent.rs:2421`, still with `parsed` in scope):

```rust
if is_error_result && !hold_back_for_resume_retry {
    let failure = crate::agents::failure::classify(None, None, "", Some(&parsed));
    core::persist_last_failure(&block_id_read, Some(&failure), &wstore_read, &event_bus_read);
    if let Some(ref broker) = broker_read {
        broker.publish(wps::WaveEvent {
            event: wps::EVENT_AGENT_FAILURE.to_string(),
            scopes: vec![format!("block:{}", block_id_read)],
            sender: String::new(),
            persist: 1,
            data: serde_json::to_value(&failure).ok(),
        });
    }
}
```

`classify()` is called with `exit_code: None, signal: None, stderr: ""` —
the process hasn't exited, so there is no exit evidence — and
`result_frame: Some(&parsed)`. This is exactly the shape the function's own
doc comment describes handling; `frame_error_text()` extracts the matchable
text from the frame itself, so an empty stderr string doesn't lose
anything. `is_error_result` is a `bool` computed at `persistent.rs:2267-2268`
inside the same `if let Ok(parsed) = ...` block this new code lives in — no
new scoping needed, unlike `hold_back_for_resume_retry` which is already
hoisted (`let mut hold_back_for_resume_retry = false;` at `:2201`, outside
the block) for the same reason.

### 2. Process exit: the resume-retry machinery has already decided this exit is final

`persistent.rs:2653-2654` calls
`apply_resume_event(ProcessExited{generation})` and iterates the resulting
`effects: Vec<ResumeEffect>` (`:2700-2770`). Three variants exist:

| Variant | Meaning | Classify here? |
|---|---|---|
| `PersistImmediately(line)` / `FlushErrorLine(line)` | This error line is final — not being retried internally. | **Yes** |
| `FireRetry{retry, held_error_line}` | A stale `--resume` caused this exit; the persistent-resume machinery is about to respawn fresh automatically. The user must never see this as a failure (`persistent.rs:2716-2719`'s own comment: *"the doomed attempt's own terminal error result must never reach the user"*). | No |
| `PublishDone` | Clean exit, no error line. | No |

Add the classify+persist+publish call inside the `PersistImmediately(line) |
FlushErrorLine(line)` arm (`persistent.rs:2702-2714`), after the existing
`handle_append_block_file` call, using `line` as the evidence — parse it as
JSON first (it's the same `result`-frame-shaped text the mid-stream path
already handles) and fall back to treating it as raw stderr text if it
doesn't parse:

```rust
persistent_resume::ResumeEffect::PersistImmediately(line)
| persistent_resume::ResumeEffect::FlushErrorLine(line) => {
    if let Some(ref broker) = broker_wait {
        super::shell::handle_append_block_file(/* unchanged, existing call */);
    }
    let parsed_line: Option<serde_json::Value> = serde_json::from_str(&line).ok();
    let stderr_text = if parsed_line.is_none() { line.as_str() } else { "" };
    let failure = crate::agents::failure::classify(
        Some(exit_code), None, stderr_text, parsed_line.as_ref(),
    );
    if failure.retryable || failure.code != crate::agents::failure::FailureClass::UnknownNonZero {
        core::persist_last_failure(&block_id_wait, Some(&failure), &wstore_wait, &event_bus_wait);
        if let Some(ref broker) = broker_wait {
            broker.publish(wps::WaveEvent {
                event: wps::EVENT_AGENT_FAILURE.to_string(),
                scopes: vec![format!("block:{}", block_id_wait)],
                sender: String::new(),
                persist: 1,
                data: serde_json::to_value(&failure).ok(),
            });
        }
    }
}
```

The `failure.retryable || code != UnknownNonZero` guard is deliberately
conservative: this exit arm's `line` isn't always a clean `result` frame the
way the mid-stream path's `parsed` always is (`FlushErrorLine`'s line can
originate from an earlier held-back turn — see `persistent.rs:2317-2349`'s
comment on flushed lines), so avoid publishing a low-confidence
`UnknownNonZero` classification from noisy text; still publish every
recognized class (including non-retryable ones like `Auth` — the recovery
banner has value for those too, just without auto-retry, per
`SPEC_AGENT_FAILURE_RECOVERY_UI`'s existing per-class action matrix).

### 3. No frontend changes

`useAgentFailure.ts` already subscribes to `EVENT_AGENT_FAILURE` per block
and already has the full auto-retry countdown/banner built — this is purely
a backend wiring fix. The existing `isTransient()` gate
(`rate_limited`/`overloaded`/`network`) already decides which classes
auto-retry vs. show a manual-only banner; `RateLimited`/`Overloaded` are
already in that set.

## Non-goals

- **No change to the auto-retry policy itself** (5s/10s countdown, 2-attempt
  cap) — that's `SPEC_AGENT_FAILURE_RECOVERY_UI`'s design, already shipped,
  already correct.
- **No change to `persistent_resume`'s stale-`--resume` retry mechanism** —
  this spec only adds classification on the paths that mechanism has
  already decided are NOT being retried; the mechanism itself is untouched.
- **No change to `classify()`'s keyword taxonomy** — `RateLimited`/
  `Overloaded` detection already exists and is already unit-tested against
  real Anthropic error phrasings.
- **Not extending this to `SubprocessController`** — it already has this
  wiring (`host_spawn.rs:603-687`); this spec closes the one remaining gap.

## Files changed

| File | Change |
|---|---|
| `agentmux-srv/src/backend/blockcontroller/persistent.rs` | Two new call sites (mid-stream error-result branch, exit-arm `PersistImmediately`/`FlushErrorLine` match arm) calling `classify()` + `core::persist_last_failure()` + publishing `wps::EVENT_AGENT_FAILURE`, mirroring `subprocess/host_spawn.rs:603-687`. |

## Testing plan

- Unit test the mid-stream path: feed a synthetic `{"type":"result",
  "is_error":true, "result":"...overloaded..."}` line through the stdout
  handling path with `hold_back_for_resume_retry` forced `false`, assert
  `classify()` returns `FailureClass::Overloaded` and the WPS publish
  fires with `persist:1`. Repeat with `hold_back_for_resume_retry` forced
  `true` (a live stale-`--resume` retry candidate) and assert **no**
  publish — this is the regression-guard for not breaking the existing
  resume-retry silence-the-doomed-attempt behavior.
- Unit test the exit-arm path for each of the three `ResumeEffect`
  variants: `PersistImmediately`/`FlushErrorLine` with an
  `is_error:true` line → publish fires; `FireRetry` → no publish (the
  existing "never show this as a failure" comment's invariant, now
  covered by a test instead of just a comment); `PublishDone` → no
  publish.
- Manual, in `task dev`: the most direct repro is hard to force on-demand
  (would need a real 429/529 from Anthropic), so this is the weakest link
  in verification — flag rather than skip: if there's a way to inject a
  synthetic `is_error:true` result frame through the same code path
  `task dev` uses (e.g. a debug/test hook, or temporarily wrapping `claude`
  with a script that emits one canned error line), use it to confirm the
  recovery banner + auto-retry countdown actually render end-to-end, not
  just that the backend event fires.

## Open questions

- Should the exit-arm's `stderr_text` fallback (when `line` doesn't parse as
  JSON) also try stripping any leading/trailing framing the append-format
  might add? Not fully verified against `handle_append_block_file`'s exact
  line format — worth a quick check during implementation rather than
  assuming `line` is always bare JSON.
- `FailureClass::UnknownNonZero` is deliberately excluded from the exit-arm
  publish (see the guard in §2) to avoid noisy false-positive banners from
  unrelated flushed lines. Worth confirming this doesn't also suppress a
  genuine-but-unrecognized error class that should have shown *something* —
  low risk (the mid-stream path already covers the common
  cleanly-`is_error_result`-tagged case), but not proven either way.
