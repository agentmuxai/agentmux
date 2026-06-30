# Agent Status Labels — Richer Working State UX

**Date:** 2026-06-27  
**Status:** Draft  
**Files touched:** `AgentFooter.tsx`, `agent-pane-state/types.ts`, `agent-pane-state/reducer.ts`, `agent-view.tsx`

---

## 1. Problem

`pickThinkingPhrase()` always returns `"Working"`. Every agent activity — user message, background shell output, another agent pinging back, rate-limit retry, tool use — shows the same label. This is low-information and creates a spurious UX problem:

- A background shell producing stdout re-invokes the agent. The user sees `"Working…"` appear unprompted, with no explanation, then disappear a second later.
- A long tool chain (`bash` → `read` → `edit`) shows `"Working…"` throughout with no indication of what step is running.
- Rate-limit back-off and a productive tool call look identical.

---

## 2. Goals

1. **Status label is always informative** — a user who glances at the footer can answer "what is Claude doing right now?"
2. **Background-triggered turns are distinguishable** from user-initiated ones — no more mystery `"Working…"` popups.
3. **Tool-level activity is surfaced** — reading, editing, running a terminal, searching.
4. **Zero regression** on the core state machine — all changes are additive, label derivation is pure.

---

## 3. Trigger Context (new field)

### 3.1 Why it's needed

The `TurnPhase` discriminated union currently has no record of *why* the turn started. We need to distinguish:

| Trigger | Current display | Desired display |
|---------|----------------|-----------------|
| User typed a message | `Working…` | `Thinking…` |
| Background shell output flushed | `Working…` | `Responding to shell` |
| Another agent sent a message | `Working…` | `Responding to agent` |
| Cron / scheduled run | `Working…` | `Running scheduled task` |
| MCP tool callback | `Working…` | `Responding to tool` |

### 3.2 Proposed type

Add `triggerContext` to the `Submitting` and `Streaming` phases:

```typescript
export type TurnTrigger =
    | { kind: "user" }                          // user sent a message
    | { kind: "shell"; shellId: string; title?: string }  // shell stdout flushed
    | { kind: "agent"; agentId: string; name?: string }   // another agent replied
    | { kind: "cron"; cronName: string }        // scheduled run
    | { kind: "tool"; toolName: string }        // MCP tool callback
    | { kind: "unknown" };                      // fallback / legacy
```

Add to `Submitting` and `Streaming` phases:

```typescript
| {
      kind: "Streaming";
      bufferSize: number;
      toolsActive: number;
      lastEventMs: number;
      trigger: TurnTrigger;          // ← new
      waitingReason?: "rate_limited";
      retryAfterMs?: number | null;
  }
```

### 3.3 Where trigger is set

- **`TurnStart` command** (user sends message) → `{ kind: "user" }`
- **`StreamFlushObserved` command** (shell stdout re-invokes agent) → `{ kind: "shell", shellId, title }` — shellId and title already available in the flush event payload
- **Agent-to-agent message event** → `{ kind: "agent", agentId, name }`
- All others → `{ kind: "unknown" }` initially, refined as new triggers are identified

---

## 4. Status Label Dictionary

### 4.1 Derivation function

Replace `pickThinkingPhrase()` with a pure function:

```typescript
export function deriveStatusLabel(
    phase: TurnPhase,
    currentTool: string | null,
    currentToolArg: string | null,
): string
```

### 4.2 Priority order

Labels are derived by checking from most-specific to least-specific:

```
1. Phase is Interrupting              → "Stopping…"
2. Phase is Submitting                → label from trigger (see §4.3)
3. Phase is Streaming + rate-limited  → "Waiting for rate limit…"
4. Phase is Streaming + currentTool   → label from tool map (see §4.4)
5. Phase is Streaming + trigger       → label from trigger context (see §4.3)
6. Phase is Streaming, no tool        → "Thinking…"
```

### 4.3 Trigger → label map

| `trigger.kind` | Label (no tool active) | Label (submitting) |
|---------------|------------------------|-------------------|
| `"user"`      | `"Thinking…"`          | `"Sending…"` |
| `"shell"`     | `"Responding to shell"` | `"Waking for shell…"` |
| `"agent"`     | `"Responding to agent"` | `"Waking for agent…"` |
| `"cron"`      | `"Running scheduled task"` | `"Starting scheduled run…"` |
| `"tool"`      | `"Responding to tool"` | `"Waking for tool…"` |
| `"unknown"`   | `"Working…"` | `"Working…"` |

When `trigger.kind === "shell"` and `trigger.title` is set: `"Responding to shell: ${trigger.title}"` (truncated to ~30 chars).

When `trigger.kind === "agent"` and `trigger.name` is set: `"Responding to ${trigger.name}"`.

### 4.4 Tool → label map

When `currentTool` is set during `Streaming`, use this map. `currentToolArg` (file path or command) is appended when short (≤ 30 chars) and meaningful.

| Tool name (or prefix) | Label |
|----------------------|-------|
| `Bash` | `"Running terminal"` — with truncated command if short |
| `mcp__agentmux__Shell` | `"Starting shell"` |
| `mcp__agentmux__ShellStop` | `"Stopping shell"` |
| `Read` | `"Reading file"` — with basename of path |
| `Write` | `"Writing file"` — with basename |
| `Edit` | `"Editing file"` — with basename |
| `Glob` | `"Finding files"` |
| `Grep` | `"Searching code"` |
| `WebFetch` | `"Fetching URL"` |
| `WebSearch` | `"Searching the web"` |
| `Agent` | `"Spawning agent"` |
| `Workflow` | `"Running workflow"` |
| `mcp__agentmux__NewTab` | `"Creating tab"` |
| `mcp__agentmux__Layout` | `"Reading layout"` |
| `mcp__agentmux__SendMessage` | `"Sending message"` |
| `TaskCreate` | `"Creating task"` |
| `TaskUpdate` | `"Updating task"` |
| `AskUserQuestion` | `"Asking you a question"` |
| `NotebookEdit` | `"Editing notebook"` |
| `mcp__*` (generic MCP) | `"Using tool: ${toolName}"` (strip `mcp__` prefix) |
| *(anything else)* | `"Working…"` |

### 4.5 "Background" badge

When `trigger.kind !== "user"`, show a small badge or alternate color on the status row to signal that this turn was not user-initiated. Exact visual TBD in design pass. Suggested: dim the row or show a small icon (⚙ for shell, ↩ for agent).

---

## 5. Streaming phase sub-states

The existing `Streaming` phase already carries:

```typescript
toolsActive: number     // concurrent tools in flight
bufferSize: number      // queued output chunks
lastEventMs: number     // timestamp of last stream event
```

### 5.1 Stall detection

If `Date.now() - lastEventMs > STALL_THRESHOLD_MS` (suggested: 8000ms) while still `Streaming`:
- Label: `"Working… (no activity for Xs)"`
- This surfaces hung tool calls or silent network issues without a false alarm.

### 5.2 Multi-tool label

If `toolsActive > 1`: prefix label with `"[${toolsActive} tools] "`.  
E.g. `"[3 tools] Running terminal"`.

---

## 6. Done phase

Already shows `"✓ Worked · 42s"`. Extend with trigger context:

| Trigger | Done label |
|---------|-----------|
| `"user"` | `"✓ Done · 42s"` |
| `"shell"` | `"✓ Shell response · 1s"` |
| `"agent"` | `"✓ Agent response · 3s"` |
| `"cron"` | `"✓ Scheduled run · 2m 10s"` |
| `"unknown"` | `"✓ Done · 42s"` |

---

## 7. Implementation plan

### Phase 1 — Label derivation (no trigger context yet)
1. Add `TOOL_LABEL_MAP` constant to `AgentFooter.tsx`.
2. Replace `pickThinkingPhrase()` with `deriveStatusLabel(phase, currentTool, currentToolArg)`.
3. Pass `turnPhase` (already available at call site in `agent-view.tsx:1101`) into `AgentWorkingRow`.
4. **Tests:** pure-function unit tests for `deriveStatusLabel` covering all cases.

### Phase 2 — Trigger context
5. Add `TurnTrigger` type to `types.ts`.
6. Extend `Submitting`, `Streaming`, and `StreamFlushObserved` command shapes with `trigger: TurnTrigger`.
7. Populate trigger at dispatch sites:
   - `TurnStart` (user sends message) → `{ kind: "user" }`
   - `StreamFlushObserved` (shell stdout) → enrich payload in `useAgentStream.ts` with `shellId`/`shellTitle` from the document store *before* dispatch (see Q1 resolution); reducer constructs `{ kind: "shell", shellId, title }`.
   - Agent-to-agent turns → `{ kind: "agent", agentId }` (needs follow-up to identify the exact dispatch site — see Q2 resolution).
8. Agent display name resolved at render time from agent registry atom, not stored in trigger.
9. Thread trigger through to `AgentWorkingRow` props and `deriveStatusLabel`.
10. Label text alone signals background turns (no badge for Phase 2 — see Q4 resolution).

### Phase 3 — Done phase and stall detection
10. Extend done-label with trigger context.
11. Add stall detection timer to `AgentWorkingRow` using `useTick()` (already present).

---

## 8. Non-goals

- Not a full activity log replacement (that's `ActivityDock`/`AgentComposerStrip`).
- Not changing the state machine itself — the discriminated union is already correct.
- Not showing shell stdout in the status label.
- No changes to the `TurnOutcome` or `Done` state lifecycle.

---

## 9. Open questions — RESOLVED

### Q1 ✅ Shell title availability
**Answer:** The title is **not in the `StreamFlushObserved` payload** — it only carries `addedCount: number` and `at: number`. The shell title lives in `ShellNode` in the document/block store.

**Resolution:** At the point where `StreamFlushObserved` is dispatched (in `useAgentStream.ts`), the document store *is* accessible. Enrich the payload there before dispatch:

```typescript
// useAgentStream.ts — when emitting StreamFlushObserved
const activeShell = docNodes.findLast(n => n.type === "shell" && n.status === "running");
dispatch({
    type: "StreamFlushObserved",
    addedCount,
    at: Date.now(),
    shellId: activeShell?.id ?? null,      // ← new
    shellTitle: activeShell?.title ?? null, // ← new
});
```

Update `StreamFlushObserved` command type in `types.ts` accordingly. The reducer uses these to construct `trigger: { kind: "shell", shellId, title }`. No block-store lookup needed downstream.

---

### Q2 ✅ Agent name at TurnStart
**Answer:** `AgentMessageEvent.from` carries only the sender's **agent ID**, not a display name. Additionally, agent-to-agent messages currently don't route through `TurnStart` — that command is user-initiated only.

**Resolution:**
- Store the sender `agentId` in the trigger: `{ kind: "agent", agentId: from }`.
- Resolve the display name **at render time** (not in the reducer) by looking up the agent registry atom: `agentRegistry[agentId]?.name ?? agentId`. This keeps the reducer pure and avoids stale-name bugs.
- A separate investigation is needed to confirm which event causes the receiving agent's turn to start when an agent-to-agent message arrives (not `TurnStart` — likely a `StreamFlushObserved` or a new command). Track under a follow-up task.

---

### Q3 ✅ `pickThinkingPhrase` rotation
**Answer:** Rotation was **intentionally removed**. The function previously cycled a 100+ phrase array (`"Accomplishing"`, `"Baking"`, `"Muxing"`, `"Swarmifying"`, …) every 30 seconds, with `_exclude` preventing back-to-back repeats. It was simplified to `return "Working"` in a deliberate refactor. The `_exclude` and `_phrase` underscore params are now dead signatures.

**Resolution for Phase 1:**
- Delete `pickThinkingPhrase` and `ingToEd` entirely.
- Replace with `deriveStatusLabel(phase, currentTool, currentToolArg, trigger?)`.
- No rotation fallback — the new label map covers all known states; `"unknown"` trigger falls back to `"Working…"` which is no worse than today.
- Remove `_exclude` call sites in `AgentWorkingRow` (the interval-based rotation can be removed or simplified to a static label).

---

### Q4 🎨 Visual design for background badge
**Status: Needs design decision.**

Options:
- **A) Muted italic label** — `"Responding to shell"` rendered in a lighter/italic style, no icon. Minimal change, purely typographic.
- **B) Small icon prefix** — ⚙ for shell, ↩ for agent, 🕐 for cron. High information density, may feel noisy.
- **C) Color shift** — Use a distinct CSS variable (e.g. `--status-bg-shell`) for the working row background on non-user turns. Subtle, doesn't change label layout.
- **D) No badge, label only** — The label text itself (`"Responding to shell"`) is sufficient; no extra visual treatment.

**Recommendation:** Option D for Phase 2 launch — the label change alone is the signal. Badge/color can be a follow-up after user testing.
