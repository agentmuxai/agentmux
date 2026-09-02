# SPEC: "Working…" State — Liveness Model (rethink, not rewrite)

**Date:** 2026-06-29
**Status:** Design note — for review before any code
**Author:** AgentX
**Related:** `frontend/app/view/agent/useAgentStream.ts`, `frontend/app/store/agent-pane-state/reducer.ts`, `frontend/app/store/agent-pane-state/types.ts`, `frontend/app/view/agent/stream-parser.ts`

---

## 0. TL;DR

The "Working…" indicator hangs indefinitely for **persistent-mode agents** when a
turn's terminal `session_end` event is missed. Root cause is architectural, not a
single bug: completion is modeled as *"working until we are told we stopped,"* so
it depends on never losing one terminal event — and every way that event can go
missing has been patched with its own rescue timer. Three such timers exist; none
covers the persistent-idle case, and one is a diagnostic no-op.

This note proposes **one principled change** — derive "Working" from a *liveness
invariant* (working only while we keep observing activity) — that subsumes the
rescue timers, rather than adding an eighth patch. The discriminated-union state
machine is sound and is **kept**; only the completion-detection policy and the
redundant timers change.

## 1. Symptom

A persistent agent (Claude Code in persistent mode) showed "Working…" for
**1286 minutes** while idle between turns. Logs confirm: the srv streamed the
agent's stdout normally, but the frontend turn phase never left `Streaming`.

## 2. How "Working" is computed today

- `turnPhase` is a discriminated union: `Idle | Submitting | Streaming |
  Interrupting | Done | Disconnected` (reducer-owned, `reducer.ts`).
- `Working = workingFromPhase(turnPhase)` — true for `Submitting | Streaming |
  Interrupting`.
- Inputs that drive transitions (`useAgentStream.ts`): `TokensIn/Out`,
  `ToolStart/End`, and the **terminal** `session_end` → `finalizeTurn` →
  `TurnEnd` → `Done` (`useAgentStream.ts:841`, `:455`).

Completion is therefore **gated on a single terminal event.** Everything else is
recovery machinery for when that event does not arrive.

## 3. Root cause — three gaps that line up

For a persistent agent stuck mid-`Streaming`, every safety net is structurally
unable to fire:

1. **Process-exit grace timer can't fire** (`useAgentStream.ts:480-498`). It is
   armed only by `ControllerStatus: done`. Its own comment states the trap:
   *"Persistent mode: the process never exits between turns, so
   `ControllerStatus: done` only fires on crash or session teardown."* An idle
   persistent agent never emits it.

2. **Stuck-stream watchdog is diagnostic-only** (`reducer.ts:255-275`,
   `types.ts:357` — "No state mutation"). It detects the 45 s idle gap and emits a
   `stream-stuck` telemetry event, then returns `state` **unchanged**. It sees the
   hang and does nothing about the phase.

3. **Stop-fallback timer only covers user-Esc** (`useAgentStream.ts:548+`, gated
   on `turnPhase.kind === "Interrupting"`).

So the *only* path out of "Working" for a persistent agent is a live `session_end`.
If it is missed — a translator edge in persistent mode, a drop across a WS
reconnect/replay gap, or a turn that stalled after a 429 and never cleanly
completed — there is **no fallback**, and the phase sits in `Streaming` forever.

`bumpEvent` re-promotion compounds it (`reducer.ts:~820-850`): after `Done.completed`,
any late tool/token event re-enters `Streaming`, so even a near-recovery can be
undone by a stray event.

> Aside (separate, harmless bug): `stream-parser.ts:248-250` logs
> `Unknown event type: session_end` for every `session_end` line during history
> replay — log spam, not the cause. Worth silencing in the same pass.

## 4. Why this is the Nth patch, not a one-off

The same subsystem has been patched repeatedly, each time adding one transition or
one timer for one way completion detection failed:

| PR / change | What it added |
|---|---|
| #987 | TurnPhase discriminated union (dual-write) |
| #728 (gaps 1–3) | Submit-timeout, interrupt-timeout, stuck watchdog |
| #1752 | "persistent agents showing working when idle" fix |
| #1790 | Clear Working when subprocess crashes |
| #1826 | Keep Working live on 429 retry |
| #1757 | Emit `session_end` per turn in persistent mode |
| fix-working-stuck-rate-limit | Rate-limit stuck-Working fix |

Seven changes, each a targeted rescue for a missing/late/misordered terminal
signal. That accretion — not any single line — is the smell. Adding a
persistent-mode watchdog now would be patch #8 with the same shape.

## 5. The flaw, stated plainly

**"Working" is derived from the wrong source of truth.** The model is
*"we are working until we are told we stopped."* That is fragile by construction:
it can only be correct if the stop signal is never lost. The rescue timers each
exist to cover one loss path; new providers and modes keep inventing new loss
paths.

## 6. Proposal — a liveness invariant (rethink, not rewrite)

Flip the model to *"we are working only while we keep observing that we are."*

> **Working ⇔ (a turn is believed in-flight) AND (activity observed within the
> liveness window).**

Make it **self-healing by construction**: no observed activity for the liveness
window AND no in-flight request ⇒ not working, with **no terminal event required**.

This single rule subsumes all three rescue mechanisms:

- It covers the persistent-idle case (no `ControllerStatus: done` needed).
- It replaces the diagnostic-only watchdog with one that *acts*: the existing
  `StreamWatchdogTick` already computes `idleSinceMs` against `lastEventMs`
  (`reducer.ts:262`) — it simply needs to **transition** `Streaming/Submitting →
  Idle` past a (longer, conservative) liveness threshold instead of returning state
  unchanged.
- It still honors a clean `session_end` as the *fast path* to `Done` (zero-latency,
  carries stats); the liveness rule is only the floor that guarantees recovery.

### 6.1 What changes

1. **One watchdog that acts.** Make `StreamWatchdogTick` transition out of a
   working phase after a conservative liveness threshold (e.g. 120–180 s of zero
   stream activity with no active tool and no `provider_waiting`). Keep the 45 s
   `stream-stuck` telemetry as an earlier, non-mutating signal if useful.
2. **Liveness window, mode-aware inputs.** `provider_waiting` (429 retry) and live
   tool chunks already refresh `lastEventMs` — they keep the agent "alive" without
   a terminal event, which is exactly right. Tool execution that legitimately runs
   longer than the window must emit periodic liveness (it largely does via tool
   chunks); audit the few that don't.
3. **Delete the redundant nets.** Once the liveness rule covers crash, stall, 429,
   and persistent-idle uniformly, the process-exit grace timer and the
   stop-fallback timer collapse into the one rule (the AgentFailure banner remains
   driven independently by `ControllerStatus`/disconnect, not by turn phase).
4. **Tame `bumpEvent` re-promotion.** Re-promotion from `Done.completed` should
   require evidence of a *new* turn (e.g. `message_start` / `TurnStart`), not any
   stray late tool/token event, so a post-completion straggler can't revive
   "Working".
5. **Silence replay noise.** `stream-parser.ts` should treat `session_end` (and
   other non-node control events) as known-and-ignored during replay, not warn.

### 6.2 What does NOT change

- The `turnPhase` discriminated union and its invariants — it is a correct,
  well-tested state machine; the phases are right.
- The fast path: `session_end → finalizeTurn → TurnEnd → Done` stays as the
  primary, low-latency completion.
- Provider-quirk knowledge encoded in the existing transitions — it is preserved,
  not discarded. **This is why a ground-up rewrite is the wrong call**: it would
  regress that hard-won edge-case coverage to re-solve a problem that is really
  about completion policy, not the machine.

## 7. Why not a full rewrite

The bug was never in the state machine — it is in the **inputs** (a single
fallible terminal event) and the **recovery policy** (per-failure-mode timers). A
rewrite carries all the regression risk of re-deriving the union and its
transitions while fixing neither root issue. The liveness invariant is a localized
change to one reducer case + the removal of two timers — far lower risk and it
deletes the *reason* patches #2–#7 existed.

## 8. Risks & open questions

- **Threshold tuning.** Too short → a legitimately long, quiet tool call (or a slow
  first token) flips to Idle mid-turn. Mitigation: conservative window (≥120 s),
  and ensure long operations emit liveness. Open: enumerate tools/providers that
  can be silent > window and have them heartbeat.
- **`Done` vs `Idle` on watchdog recovery.** Recovering via liveness should land in
  `Idle` (turn abandoned, no stats) and must not fabricate a `Done.completed` that
  would mislead session digests. Open: confirm downstream consumers (digest, token
  meter) handle a turn that ends without `session_end`.
- **429 long-waits.** `provider_waiting` must keep refreshing liveness for the full
  retry backoff so a legitimate long rate-limit wait is not mistaken for a stall.

## 9. Suggested sequencing

1. Land the **acting watchdog** (the single highest-value change — it directly
   ends the 1286-minute class of hang) behind the conservative threshold.
2. Constrain `bumpEvent` re-promotion to genuine new-turn evidence.
3. Remove the now-redundant grace/stop timers once the watchdog is proven.
4. Silence the replay `session_end` warning.

Each step is independently shippable and reversible; together they replace seven
patches' worth of rescue machinery with one invariant.
