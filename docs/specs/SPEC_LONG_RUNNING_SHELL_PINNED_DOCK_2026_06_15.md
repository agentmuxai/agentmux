# SPEC: Pinned Activity Dock — Unified Long-Running Activities

**(shell · cron · subagent · …)**

**Date:** 2026-06-15
**Status:** Draft
**Builds on:** `SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md` (Phases 1–2),
`SPEC_PERSISTENT_SHELL_PHASE3_STOP_2026_06_14.md` (stop / tree-kill, PR #1422),
MSYS cwd fix (#1415), and the existing subagent stack
(`subagent_watcher.rs`, `useSubagentEvents.ts`, swarm pane).

> Supersedes the shell-only framing of this file's first draft. The dock is
> **not** shell-specific — a shell, a cron, and a subagent are all *kinds* of the
> same thing: a long-running activity that should stay glanceable and
> expandable.

---

## 1. Problem

Long-running things an agent starts are **scattered and scroll away**:

- A **shell** (`task dev`, vite) is a row buried deep in the conversation.
- A **subagent** (Task tool) lives in the *Swarm* pane — a different surface
  entirely; you context-switch to check on it.
- A **cron / recurring** check has no first-class home at all.

There is no single, glanceable place that answers *"what's running for me right
now, and how is it doing?"* Each kind also re-implements its own status chrome.

---

## 2. Core idea

**One abstraction, one dock.** Introduce a **`PinnedActivity`** — anything
long-running the agent spawns — and render every instance as a uniform colored
row in a **dock pinned to the top of the agent pane**. Click a row → its
**live view expands**; click again → collapses. Kinds are pluggable:

| Kind | Today's surface | Live view (expand) | Stop action |
|------|-----------------|--------------------|-------------|
| `shell` | inline `PersistentShellBlock` | streaming log (`ToolOverlayLog`) | `ShellStop` (tree-kill) |
| `cron` | — (new) | iteration timeline | `ShellStop` (kill loop) |
| `subagent` | Swarm pane card | live transcript / current tool | cancel subagent (`agentstop`) |
| *(future)* `download`, `build`, `deploy`… | — | kind-specific | kind-specific |

The dock is the **live control surface**; each kind keeps its existing detailed
surface (inline block, swarm pane, subagent view) as the deep/permanent view.

```
┌─ AgentControlBar ───────────────────────────────────────────┐
├─ Activity dock (pinned, sticky) ────────────────────────────┤
│ ⟩ task dev          [run 4:12] ↳ vite ready in 312ms      ■ │  ← shell
│ ⟳ check disk q60s   [run 2:00] ↳ 41% used                 ■ │  ← cron
│ ◆ refactor auth     [run 1:08] ↳ editing auth/login.ts    ■ │  ← subagent
├─ Conversation document (scrolls under the dock) ────────────┤
```

---

## 3. The `PinnedActivity` abstraction

A presentational + behavioral contract every kind implements (frontend):

```ts
type ActivityKind = "shell" | "cron" | "subagent";
type ActivityStatus = "running" | "done" | "error" | "stopped";

interface PinnedActivity {
    id: string;
    kind: ActivityKind;
    title: string;            // command, cron label, or subagent task
    status: ActivityStatus;
    startedAt: number;        // Unix ms
    endedAt?: number;
    sigil: string;            // ⟩ / ⟳ / ◆  (per kind, colored by status)
    tailLine?: () => string;  // latest line / current step — the collapsed tail
    canStop: boolean;
    stop: () => void;         // ShellStop / kill loop / cancel subagent
    Expanded: Component;      // the kind-specific live view rendered on expand
}
```

**Adapters** map each existing source onto it — no new persistence, all derived:

- **Shell adapter** — `ShellNode`s from the agent-document store
  (`status === "running" || recentlyExited`). `stop` → `ShellStopCommand`.
  `Expanded` → the existing shell `ToolOverlayLog`.
- **Subagent adapter** — `SubagentInfo` from `useSubagentEvents` /
  `swarm-model.ts`. `SubagentStatus::Active → running`, `Completed → done`.
  `stop` → cancel (`AgentStopCommand` on the subagent block). `tailLine` →
  current tool / last message. `Expanded` → condensed `subagent-view`.
- **Cron adapter** — `CronActivity` (a `shell` running an interval loop, §6).
  `Expanded` → iteration timeline.

Status colors/sigils unify in `_shell-node.scss` → renamed/extended to
`_activity-dock.scss`; every kind reuses the same row chrome.

---

## 4. The dock (UI)

- **Sticky strip** between `AgentControlBar` and the conversation document in
  `agent-view.tsx`. The conversation scrolls *under* it.
- **One row per running activity.** Collapsed row = sigil + title + elapsed +
  tail + stop. **Click → `Expanded` view inline; click again → collapse.**
  Expand state stored in the existing **`pinnedNodes`** set keyed by activity
  `id`, so dock and the kind's own surface (inline block, swarm) stay in sync.
- **Ordering:** running-first, focus-first, interleaved across kinds — see **D3**.
- **Stack + summary:** ≤ 3 rows inline; overflow collapses to a truthful
  "▸ N more" chip with a scrollable full list — see **D6**.
- **Exit handling:** linger time depends on terminal status (done/stopped fade,
  error persists until acknowledged) — see **D4**. The detailed surface (inline
  block / Swarm card) always remains as the record.
- **Responsive:** reuse the container-query tiers from
  `SPEC_AGENT_PANE_RESPONSIVE_AUX_INFO_2026_06_09` (hide tail <400px, elapsed
  ≥600px, stop-without-hover ≥900px).

### Components / files
| File | Change |
|------|--------|
| `frontend/app/view/agent/components/ActivityDock.tsx` *(new)* | merges adapters → sorted activity list → rows |
| `frontend/app/view/agent/components/ActivityRow.tsx` *(new)* | uniform collapsed row + inline `Expanded` slot |
| `frontend/app/view/agent/activity/adapters.ts` *(new)* | shell / subagent / cron → `PinnedActivity` |
| `frontend/app/view/agent/agent-view.tsx` | mount `<ActivityDock>` |
| `frontend/app/view/agent/styles/_shell-node.scss` → `_activity-dock.scss` | shared row/sigil/status styles |
| `PersistentShellBlock.tsx`, `SubagentLinkBlock.tsx` | "pinned above" compact mode while running |

---

## 5. Part B — Auto-detect long-running shells

So the agent never has to *predict* duration (only relevant to the `shell`
kind; subagents/crons are explicit by construction):

- **Launch heuristic** — known shapes (`task dev`, `vite`, `*serve*`,
  `*--watch`, `nodemon`, `next dev`, `cargo watch`, `tail -f`, `ping -t`, …)
  route a `Bash` call to a ShellNode → dock, returning the `shell_id`.
- **Overrun promotion** (the catch-all; original spec §7 / Phase 4):
  `agentmux-bashwrap` already wraps every `Bash` call — if one is still running
  after a threshold (~30s, `agent:bashwrap_promote_secs`), **promote** it to a
  ShellNode, publish `shell_node_create` retroactively (→ dock), and return:
  *"Still running after 30s — promoted to background shell `<id>`; watch it in
  the dock, stop with `ShellStop(<id>)`."* → **no command can hang the agent.**

---

## 6. Part C — Cron / "stays active"

A recurring check is a `shell` whose work is periodic — pinned and observable
like any activity:

- Sugar tool `ShellEvery(cmd, interval_secs, title?)` → a ShellNode running the
  cross-platform equivalent of `while true; do <cmd>; sleep <n>; done`, each
  iteration delimited by a `system` chunk (`── 11:32:00 ──`) so the live view
  reads as a timeline. Stop = `ShellStop` (tree-kills the loop).

**Explicit non-goal — scheduling the *agent*.** "Every minute the *agent*
re-evaluates and acts" needs a scheduler that triggers **agent turns**
(`CronCreate` / `ScheduleWakeup` in the agent runtime) — a separate spec. This
dock makes the *process/subagent* side first-class and observable; the two
compose (a scheduled agent could keep a `ShellEvery` monitor pinned).

---

## 7. Architecture

```
            ┌─────────────── ActivityDock (derived signal) ───────────────┐
            │  merge + sort(running first, by startedAt)                   │
            └───▲────────────────────▲───────────────────────▲────────────┘
                │                     │                       │
        shell adapter          subagent adapter          cron adapter
                │                     │                       │
        agent-document          useSubagentEvents        ShellEvery /
        store (ShellNode)        / swarm-model            interval ShellNode
                │                     │                       │
        ShellStopCommand        AgentStopCommand          ShellStopCommand
        (tree-kill, #1422)      (cancel subagent)         (kill loop)
```

- **No new backend state for the dock** — it's a pure frontend merge of streams
  that already exist. Subagents already publish status (`subagent_watcher.rs`);
  shells already stream (`shell_chunk`); cron is a shell.
- **Extensibility:** a new kind = one adapter implementing `PinnedActivity` +
  an `Expanded` component. The dock, row chrome, stack/summary, and responsive
  behaviour are kind-agnostic.

---

## 8. Resolved design decisions

### D1 — Dock vs. Swarm: complementary, not duplicate
- **Dock = active + glanceable, scoped to *this* agent pane.** It shows only
  activities spawned by this pane's agent (block-scoped — see D5), in their
  running + briefly-terminal state. It is a "now active" mini-bar.
- **Swarm = full roster + management + history** across the whole swarm
  (all subagents incl. completed, detail, spawn). It is the "library."
- A running subagent appears in **both, intentionally** — like a now-playing bar
  vs. the full library. They are different scopes, not a render conflict.
- **Stop is shared + idempotent.** Dock-stop and Swarm-stop call the *same* RPC
  (`ShellStopCommand` for shells, `AgentStopCommand` on the subagent block for
  subagents). The Phase-3 shell registry is already idempotent (`stop()` on an
  unknown id is a no-op), and `AgentStopCommand` is too — so a double-stop from
  the two surfaces can never misbehave.

### D2 — Durability: session-scoped, replay-backed (no new persistence)
- The dock is a **live** surface. On a pane reload, a *running* activity
  reappears because **its source already replays**: shells via the
  `shell_chunk` persist:1024 ring on resubscribe (Phases 1–2), subagents via
  their own store (`subagent_watcher.rs`), cron because it *is* a shell.
- The dock therefore wires **no new FileStore persistence**. Terminal
  activities are not in the dock (they live in the conversation / Swarm).
- **Across a full app restart**, ShellNodes/cron do **not** survive (consistent
  with the original spec §10 Q1 — ShellNode is session-scoped). App-restart
  durability for cron is an explicit **future enhancement**, not v1.

### D3 — Ordering: running-first, focus-first, interleaved
- Sort key: **running before terminal**; within running, the **expanded**
  activity floats to the top (you're focused on it), then by `startedAt`
  **descending** (newest spawn on top).
- Terminal rows sink below running and auto-dismiss per D4.
- **Interleaved across kinds** — one unified "what's active" list; the per-kind
  sigil (`⟩` shell / `⟳` cron / `◆` subagent) distinguishes them. Grouping by
  kind appears **only** inside the overflow summary (D6).

### D4 — Exit retention: by terminal status
- `done` (clean exit / completed subagent) → linger ~8 s, then fade out.
- `stopped` (user-initiated) → linger ~3 s, then dismiss (you did it).
- `error` (non-zero exit / failed subagent) → **persist until acknowledged**
  (`×` dismiss) — failures never silently vanish.
- In every case the **permanent record** (inline block / Swarm card) remains;
  the dock only sheds the *live* row.

### D5 — Tear-off / moves: travels with the pane (confirmed)
The dock is **block-scoped** — `agent-view.tsx` renders per `model.blockId`
(`registerAgentPane(model.blockId)`), and the activity list is derived from that
block's shells/subagents. So it **travels with the agent pane** through tear-off,
floating-pane, and layout moves automatically — no window-scoped state. It also
renders in the chromeless floating-pane layout (it's core agent state, not tab
chrome).

### D6 — Cap & overflow: 3 inline, truthful summary, scroll (no silent drop)
- Show up to **3** rows inline (ordered per D3). A 4th+ collapses the overflow
  into a **"▸ N more (2 shells · 1 subagent)"** chip whose count is **always the
  true total** — click expands a **scrollable** list of *all* activities.
- The expanded activity (if any) always stays visible; only un-expanded rows
  past the cap move into the summary.
- **No silent truncation:** nothing is ever hidden without the count reflecting
  it; the full set is always one click + scroll away.

---

## 9. Relation to existing work

- **Phases 1–3** (ShellNode, streaming, stop/tree-kill) — the `shell` adapter's
  substrate.
- **Subagent stack** (`subagent_watcher.rs`, `useSubagentEvents.ts`,
  `SubagentLinkBlock`, swarm pane, `subagent-view`) — the `subagent` adapter's
  substrate; the dock is a new *live* view of it, not a replacement.
- **`SPEC_AGENT_PANE_RESPONSIVE_AUX_INFO_2026_06_09`** — the dock *is* aux info;
  reuse its tiers.
- **Original persistent-shell spec §7 + Phase 4** — concrete design for bashwrap
  auto-promotion, now with the UI to surface it.

## 10. Suggested phasing

1. **Dock + shell adapter** — highest value, reuses Phases 1–3; establishes the
   `PinnedActivity` contract and row chrome.
2. **Subagent adapter** — proves the generalization (a non-shell kind in the
   same dock); biggest "wow" since subagents currently live elsewhere.
3. **Overrun promotion** (§5) — kills the "agent picked Bash and it hung" class.
4. **Launch heuristic** (§5) + **cron sugar** (§6).
