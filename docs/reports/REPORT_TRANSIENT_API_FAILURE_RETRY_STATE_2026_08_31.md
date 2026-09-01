# Transient API failure (429 / 529 / network) retry — where we are

**Date:** 2026-08-31
**Author:** AgentY
**Status:** historical — records the state of transient-failure retry as of
2026-08-31 and the ladder fix that shipped in PR #2870. One gap fixed in the same PR (§4.1); the rest recorded,
not fixed.
**Scope:** `agentmux-srv/src/agents/failure.rs`,
`frontend/app/view/agent/hooks/useAgentFailure.ts`,
`frontend/app/view/agent/failure/failure-accessory.ts`,
`frontend/app/view/agent/providers/claude-translator.ts`,
`frontend/app/store/agent-pane-state/reducer.ts`
**Related:** `SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md` (the design; still
marked Draft/proposed though largely implemented),
`SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20.md`,
`SPEC_PERSISTENT_CONTROLLER_FAILURE_CLASSIFICATION_2026_08_04.md`,
`docs/retro/retro-busy-animation-stuck-on-429-2026-06-24.md`

---

## 1. Short answer

Retry for transient provider failures **is implemented**, at two independent
layers, and both work. The weakness was **policy, not plumbing**: the auto-retry
budget was two attempts totalling ~15 seconds, which is shorter than a typical
Anthropic 429/529 episode — so a genuinely transient, genuinely retryable
failure routinely exhausted the budget and dropped to manual-only. That is
fixed (§4.1).

Three gaps remain open and are recorded in §5. The most significant: **auto-retry
only exists while an agent pane is mounted** — it is a frontend hook, so a turn
driven headlessly gets no retry at all.

## 2. Layer 1 — the CLI's own backoff (we observe, we don't drive)

When Claude CLI hits a 429 it runs its *own* retry loop and emits
`rate_limit_event` NDJSON lines carrying `retry_after_ms`.

- `claude-translator.ts:60-63` translates it to a `provider_waiting` event.
- `agent-pane-state/reducer.ts` records `retryAfterMs` on the turn phase and
  uses it to size the stuck-stream watchdog
  (`retryAfterMs + LIVENESS_RECOVERY_MS`, `reducer.ts:318-334`) so a legitimate
  backoff isn't misread as a stall.
- The pane shows a waiting state rather than a silently-spinning progress bar.

This is the fix from `retro-busy-animation-stuck-on-429-2026-06-24.md`, and it
shipped. **No AgentMux-side retry happens or should happen here** — the CLI is
still alive and retrying; our job is only to not misdiagnose it as a hang.

## 3. Layer 2 — AgentMux retry after the turn actually fails

When the CLI gives up and the process exits:

- `agents/failure.rs` classifies the exit. `RateLimited` (429, `failure.rs:26`),
  `Overloaded` (529, `failure.rs:28`) and `Network` are marked
  `retryable: true`.
- The classification reaches the pane as an `agentfailure` WPS event and is
  persisted to block meta (`agent:last_failure`), so the recovery banner
  survives tab switches and reloads.
- `failure-accessory.ts:73-75` — `isTransient()` gates auto-retry to exactly
  those three classes. Everything else (auth, context_exceeded, killed, …) gets
  a banner with class-appropriate manual actions but never auto-fires.
- `useAgentFailure.ts` arms a countdown and re-sends the last user message.

The budget is deliberately capped: an auto-retry stays inside the same
"episode", so a persistently-throttled turn cannot loop forever. The budget
resets only on a genuine turn success or a genuinely fresh user message.
That design is sound and is preserved.

## 3.1 This was already diagnosed, a month earlier

Both gaps fixed below were identified in
`REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md`'s "Gaps vs.
best practice" section on **2026-07-27**, verbatim:

> 1. **Not exponential.** 5s → 10s is a fixed doubling for exactly two steps,
>    not real exponential growth — a THIRD consecutive 429 has no automatic
>    retry left at all and drops straight to manual, even though 429 bursts
>    are frequently short and self-clearing over more than two attempts.
> 2. **No jitter.** `armAutoRetry` fires at exactly T+5s / T+10s,
>    deterministically. Multiple agent panes sharing one account/quota (a real
>    scenario in this app — several agents, one Anthropic subscription) would
>    all retry in lockstep and collide on the SAME rate limit again.

The analysis was correct and nothing acted on it for five weeks. Worth noting
as a process observation, not just a code one: the finding was filed inside a
report about a *different* headline bug (login persistence + stuck working
state), so it never became a tracked item of its own. A correct diagnosis
buried in a larger document is, in practice, close to no diagnosis at all.

## 4. What changed in this PR

### 4.1 The ladder was too short (fixed)

`AUTO_RETRY_BACKOFF_S` was `[5, 10]` — two attempts, ~15s of total coverage,
then manual-only. Anthropic overload episodes commonly outlast that, so the
common case was: classifier correctly says "retryable", UI correctly offers
auto-retry, budget correctly exhausts — and the user is left clicking *Retry
now* by hand against a failure the system already knew was transient.

Now `[5, 15, 30, 60, 120]` — five attempts, ~3.9 minutes of coverage.

Still bounded, and deliberately so: a turn still failing after ~4 minutes is
more likely a sustained outage or an account-level limit than a blip, and at
that point a human should decide rather than have the app hammer the API
indefinitely.

### 4.2 Retries were unjittered (fixed)

Every rung fired on an exact second boundary. AgentMux drives many agents
concurrently — Fleet broadcast, cron sweeps, loops — so a provider-wide 529
fails them together and they would all retry on the *same* second, plausibly
re-triggering the overload they were backing off from.

Each rung now carries ±20% jitter (`jitteredBackoffSeconds`, floored at 1s so
the visible countdown never starts at zero). Pure and injectable, so the ladder
stays deterministic under test.

## 5. Gaps NOT addressed here

### 5.1 Auto-retry is pane-scoped — headless turns get none (most significant)

`useAgentFailure` is a SolidJS hook, mounted in exactly one place:
`agent-view.tsx:1815`. **If no agent pane is mounted, nothing retries.**

The classification, the WPS event and the persisted block meta are all
server-side and work regardless — it is only the *acting on them* that lives in
the view layer. So a turn driven by cron (`CronCreate`), a loop (`Loop`), a
Fleet broadcast, or an MCP `SendMessage` to an agent whose pane isn't rendered
will classify the 429 correctly, persist it correctly, surface it correctly the
next time a human opens the pane — and never retry.

Note this is *not* simply "the tab is in the background": inactive workspace
tabs stay mounted (`workspace.tsx` renders all tabs and hides them with
`display:none`), so those panes still retry. The exposure is panes that are
genuinely not rendered — a closed pane, or a non-active block in a pane's
block stack.

Fixing this properly means moving the retry decision behind the pane, into the
persistent controller (`backend/blockcontroller/persistent.rs`), which already
has the classification (`classify_exit_line`) and today explicitly does nothing
with it — its own doc comment says the banner is surfaced *"just without
auto-retry."* That is a real design change (who owns retry, how it interacts
with the pane's budget, what a headless agent does when the budget caps) and
wants its own spec.

### 5.2 The backoff ignores server-provided timing

`retryAfterMs` is plumbed for Layer 1 (the CLI's own backoff, used by the stall
watchdog) but Layer 2's ladder is fixed. If a failing turn's evidence carries a
`Retry-After` or an equivalent hint, honouring it would beat any hardcoded
ladder. Requires plumbing the hint through `AgentFailure`, which today has no
field for it.

### 5.3 The spec is still marked Draft

`SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md` reads `Status: Draft / proposed`
while most of its §3 action matrix, §5 actions and §6 budget are implemented and
under test. Same stale-status class as the docs cleanup in PR #2866. Not updated
here because a careful pass would need to check each row of the matrix against
the code, which is a separate piece of work from this one.

## 6. Verification

`useAgentFailure.test.ts` — 8 passing, covering the full ladder, the cap, the
jitter band, and all three budget-reset paths. Ladder expectations derive from a
single `LADDER_MS` constant so a future policy change updates one place.
Full `frontend/app/view/agent` suite: 1572 passing across 120 files.
`tsc --noEmit` clean.

Not verified by observation: I did not reproduce a live 429/529 against the
provider. The change is a policy constant plus a pure helper, both unit-tested,
but the end-to-end behaviour under a real sustained overload is unobserved.
