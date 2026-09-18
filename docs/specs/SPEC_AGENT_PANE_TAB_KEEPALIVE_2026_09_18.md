# SPEC: Keep-alive for agent tabs in the pane tab strip

**Date:** 2026-09-18
**Status:** implemented — PR #3391. `"agent"` added to
`pane-leaf-chrome.tsx`'s `KEEP_ALIVE_TYPES`, alongside the two
dormancy-gated timer fixes this audit found were needed first
(`AgentQuestionPanel`'s auto-timeout, `useAgentFailure`'s auto-retry).
**Related:** `docs/specs/SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`
(shipped the keep-alive machinery this reuses, scoped to `"term"` only, and
is the source of the "not yet audited for keep-alive safety" warning this
spec resolves), `docs/analysis/ANALYSIS_CHROME_PARITY_TAB_SWITCH_LATENCY_2026_09_15.md`
§6 (flagged in-pane agent-tab-switch latency as unmeasured and out of scope),
`docs/specs/SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` (a different,
unrelated tab surface — the top-level window tab bar — despite the similar
name; its findings don't apply here, see §1).

---

## 0. The ask

Switching between two agents sharing one pane's tab strip was slow —
multi-hundred-ms, noticeably worse than switching terminal tabs in the same
strip. Fix it, optimizing for the most robust result rather than the
smallest diff.

---

## 1. Root cause

`pane-leaf-chrome.tsx`'s `KEEP_ALIVE_TYPES` already lets `"term"` stack
members stay mounted permanently, toggling only CSS visibility on switch —
cheap, no remount. `"agent"` was deliberately left out, with a comment citing
unaudited entangled per-tab state (quick-fork, launch-in-place) as the
reason. Every agent-tab switch therefore ran the *other*, non-keep-alive
branch: the old member's `<Block>` (and its `AgentViewModel`) is fully
unmounted and a fresh one mounted for the new member. That remount pays for:

- `useHistoryPagination.ts`'s cold-start sequence — up to 3 sequential RPCs
  to `agentmux-srv` (session read → line-count → up to `RESTORE_WINDOW_LINES`
  = 5,000 lines of transcript) plus re-parsing all of it, on *every* switch,
  not just the first visit;
- a full `AgentDocumentVirtualList` rebuild over whatever that restore just
  produced.

This is the dominant cost. (`docs/specs/SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md`
profiled a different tab surface — the top-level window tab bar, which stays
mounted via `display:none` and never remounts — and attributed ITS cost to
browser layout/paint, not JS. It doesn't apply to this bug at all; the two
surfaces just share a similar name.)

---

## 2. The audit

Before widening `KEEP_ALIVE_TYPES`, the exact entangled state the existing
comment warned about was read in full: `quick-fork.ts`, `flows/launch-flow.ts`,
`flows/run-provider-login.ts`, `agent-model.ts` (including
`useSubagentBackfillGate`), `useAgentStream.ts`, `useHistoryPagination.ts`,
plus a cross-cutting grep for `visibilityState`/`IntersectionObserver`/
singleton module state anywhere in the agent view stack.

**Verdict: safe, with two real gaps.**

- **Quick-fork** (`quick-fork.ts`) has no mount-time assumptions at all —
  every read is a point-in-time, blockId-scoped store read, and the leaf/stack
  primitives it drives (`getNodeByBlockId`, the leaf reveal gate,
  `pushBlockOntoStack`) already handle a non-active stack member correctly.
- **Launch-in-place** does not rely on the Block remounting to reset state —
  it never did. The reset boundary is `<Show when={agentId()}>` *inside* one
  Block (`agent-view.tsx`'s `AgentBlockContent`): launching flips block meta,
  the picker cross-fades out, a fresh `AgentPresentationView` mounts — all
  inside the same, never-remounted `<Block>`. Keep-alive doesn't touch this
  boundary.
- **`AgentViewModel`** (`agent-model.ts`): `dispose()` is a confirmed no-op;
  every piece of constructor-time state (the activity-flash effect,
  `useSubagentBackfillGate`) is either per-instance-closure or
  blockId-keyed, with no mount-count assumptions. Cost, not correctness:
  N stack members now hold N live `useAgentDefinitions()` subscriptions
  instead of 1-recreated-per-switch.
- **`useAgentStream`/`useHistoryPagination`**: no visibility gating anywhere.
  A backgrounded-but-mounted agent's live stream keeps appending to its
  document — this is the term-PTY analogue and is the behavior keep-alive is
  *for*. `useHistoryPagination`'s cold-start chain becomes once-per-block
  instead of once-per-switch, which is the actual win. Trade: every kept-alive
  member's full parsed document + virtualizer stays resident for the pane's
  lifetime, same class of memory trade term keep-alive already made.
- **Two real gaps**, both because an unmount used to be an implicit "pause"
  that keep-alive removes:
  - `AgentQuestionPanel`'s 30s auto-timeout (auto-answers with recommended
    defaults) used to stop counting down the moment a tab was switched away
    from (unmounted) and restart fresh on return. Under keep-alive it would
    keep counting down invisibly and auto-answer a question the user was
    never shown.
  - `useAgentFailure`'s auto-retry backoff countdown, armed only from a live
    failure event, would likewise keep ticking in the background and could
    fire `doRetry()` — re-sending a turn to the CLI — from a tab nobody is
    looking at.

Both are fixed here (§3) before flipping the flag, rather than shipping the
speed win with a new correctness gap.

One item was found but deliberately **not** touched: `useAgentControllerStatus.ts`'s
`onCleanup` calls `getApi().cancelCliLogin()` unconditionally, against a
host-side login slot that is a single global, not one per caller
(`run-provider-login.ts`'s own doc comment). Closing/backgrounding one tab
mid-login can already kill a *different* tab's in-flight login today, across
panes — keep-alive makes the same pre-existing hazard reachable one level
narrower (same pane, two tabs) instead of introducing it. Fixing the
underlying global-slot ownership model is a real but separate piece of work
in an already fragile, heavily-scarred auth surface (see that file's own
history of reagent/codex findings) — out of scope here. Tracked as a known
follow-up, not silently ignored.

---

## 3. What changed

- `pane-leaf-chrome.tsx`: `KEEP_ALIVE_TYPES` now includes `"agent"`.
- `block-component-registry.ts`: added `isBlockDormant(blockId): Accessor<boolean>`,
  a reactive per-blockId mirror of the existing (plain-Set, snapshot-only)
  dormancy tracking, so a component can *subscribe* to "is my own tab
  currently backgrounded" instead of only being queried for it.
- `AgentQuestionPanel`: new optional `isDormant` prop, OR'd into the same
  gate the existing hover-pause uses — pauses the countdown for exactly as
  long as the tab stays backgrounded, re-arms a fresh window on reveal (same
  "fresh retrigger, not resumed from wherever it was paused" semantics the
  hover-pause already documents).
- `useAgentFailure`: new optional `isDormant` option — the countdown simply
  stops decrementing while dormant and resumes exactly where it left off on
  reveal (a fixed backoff budget, unlike the question timeout, so there's no
  reason to restart it).

Both new props default to "never dormant" and are optional, so every
pre-existing call site and test is unaffected.
