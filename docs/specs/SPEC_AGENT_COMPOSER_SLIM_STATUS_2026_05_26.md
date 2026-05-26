# SPEC: Slim composer status strip + expandable details panel

**Date:** 2026-05-26
**Status:** Draft — ready to implement
**Author:** AgentA
**Trigger:** User feedback — *"the progress bar is taking up too much space … we want a single line to take all our information there, so there's the bottom help text, the input box, and then a circular in-progress and a couple important stuff to the right, when clicked, you get a panel with all the options. That reduces the traffic near the input."*

---

## 1. Why

The agent composer region today stacks five separate UI surfaces between the activity log and the textarea:

```
[AgentStatusLine]   ← spinner + phrase / "Worked" summary / processBadge
[ActivityLogPanel]  ← variable height (multi-line)
[AgentControlBar]   ← perm / model / effort dropdowns + Archive / Export
[AgentFooter]       ← textarea
[hint text]         ← "Enter to send • Shift+Enter for newline • Esc"
```

In a narrow pane (≤500px) each of these consumes a row, pushing the input away from the conversation context and burning vertical real estate users would rather give to the document. The status line and control bar in particular are mostly idle — the user reads them rarely but they cost a row every render.

This redesign collapses status + controls into a **single 28–32px slim strip** directly above the textarea, with a chevron that **expands a detail panel** containing everything currently in `AgentControlBar` plus a full stats breakdown. Bottom hint text stays where it is.

---

## 2. Visual layout — before / after

### 2.1 Before (today)

```
┌─────────────────────────────────────────────────────────────┐
│ ▣ Working…  hammer   ↑2.1k ↓480 • 1m 12s         ⚙ 3       │  AgentStatusLine
├─────────────────────────────────────────────────────────────┤
│  [activity log lines]                                      │  ActivityLogPanel
├─────────────────────────────────────────────────────────────┤
│  ▼ Auto · sonnet · medium · 12 lines                       │  AgentControlBar (collapsed)
│  [Archive] [Export] [Restore]                              │
├─────────────────────────────────────────────────────────────┤
│ ┌─────────────────────────────────────────────────────────┐ │
│ │ Send message to Claude...                              │ │  AgentFooter textarea
│ │                                                        │ │
│ └─────────────────────────────────────────────────────────┘ │
│ Enter to send • Shift+Enter for newline • Esc to clear/stop│  hint text
└─────────────────────────────────────────────────────────────┘
```

Five horizontal bands above the document edge.

### 2.2 After (proposed)

```
┌─────────────────────────────────────────────────────────────┐
│ ◐ Editing src/auth.ts  ↑2.1k ↓480  1m 12s  ⚙ 3      ▾³   │  NEW: slim status strip
├─────────────────────────────────────────────────────────────┤
│ ┌─────────────────────────────────────────────────────────┐ │
│ │ Send message to Claude...                              │ │  AgentFooter textarea
│ │                                                        │ │
│ └─────────────────────────────────────────────────────────┘ │
│ Enter to send • Shift+Enter for newline • Esc to clear/stop│  hint text (unchanged)
└─────────────────────────────────────────────────────────────┘
```

**Two bands** above the textarea instead of five. The strip is a single row; AgentStatusLine + AgentControlBar **and the activity log** all collapse into it (or into the expanded panel below).

- The **strip's left segment** surfaces the **latest activity-log line** (truncated) while working — so users aren't blind to what the agent is currently doing. The cycling-phrase / current-tool text from today's AgentStatusLine is *replaced* by this — same visual slot, richer signal.
- The `▾` chevron has a small **superscript count** (`▾³`) when new activity-log entries arrived while the panel was collapsed — visual cue that there's something new to see. Counter resets on expand.
- The full activity log lives **inside the expanded details panel** as its top section (scrollable, see §4.0).

### 2.3 Strip — expanded

Clicking the `▾` chevron (or anywhere on the strip outside the inline buttons) slides up a **details panel** from below the strip:

```
┌─────────────────────────────────────────────────────────────┐
│ ◐ Editing src/auth.ts  ↑2.1k ↓480  1m 12s  ⚙ 3      ▴    │  slim strip (chevron flipped, counter cleared)
├─────────────────────────────────────────────────────────────┤
│ ─── Activity ────────────────────────────────────────────  │  details panel (top section)
│  ▸ Read src/auth.ts                                       │
│  ▸ Found existing OAuth flow                              │
│  ▸ Editing src/auth.ts (cursor: line 42)                  │
│  ▸ Wrote 28 lines, 412 bytes                              │
│  …                                                        │  (full log, scrollable max-height ~200px)
│ ─── Session ─────────────────────────────────────────────  │
│ 4 turns · $0.124 · 11.2k in / 3.8k out                    │
│ ─── Runtime ─────────────────────────────────────────────  │
│ Permission [Auto ▾]  Model [Sonnet ▾]  Effort [Medium ▾]   │
│ Tools so far: edit, bash, read, grep, hammer (×3)         │
│ [Archive session]  [Export]  [Restore]                    │
├─────────────────────────────────────────────────────────────┤
│ [textarea]                                                 │
│ hint text                                                  │
└─────────────────────────────────────────────────────────────┘
```

Panel is a single block that slides in/out with a 120ms ease. Outside-click or sending a message auto-collapses it.

---

## 3. Strip — content rules

Single horizontal row, vertically centered, padding `var(--space-1) var(--space-2)`. Height: 28px target (max 32px).

### 3.1 Left segment — "what's the agent doing right now"

Surfaces the **latest activity-log line** when there is one — this is the single-line live ticker that lets users track work without expanding the panel.

| Agent state | Strip-left content |
|---|---|
| Idle (no turn in flight, no log entries yet) | Nothing (left segment empty) |
| Working (no log line yet for this turn) | `◐` (animated circular spinner) `Working…` |
| Working, log line available | `◐` (animated) `<latest log line>` (truncated to fit, ellipsis) |
| Stopping (after Esc) | `◐` (decelerating) `Stopping…` |
| Turn just completed | `✓` (1.5s fade-out) `<final log line>` then truncates to last log line only |
| Idle, prior session log exists | (last log line, no spinner) — gives ambient memory of "where we left off" |

The circular spinner replaces today's loading-bar slab. SVG circle stroke-dasharray animation at 60rpm, 16px diameter.

**Truncation rule:** the latest log line is rendered as a single line with `text-overflow: ellipsis; overflow: hidden; white-space: nowrap;` so any length fits the strip width. Users see the full text in the activity log section of the expanded panel.

**Unread badge on chevron:** while the panel is collapsed, count new log entries since last collapse. Render the count as a superscript on the chevron glyph (`▾³`). Counter resets to 0 when the panel is expanded. Max display: `▾9+`.

### 3.2 Right segment — "the few numbers users glance at"

Right-aligned, comma-joined with `·` separators, font-size 11px:

| Condition | Right-segment items |
|---|---|
| Working | `↑<inputTokens> ↓<outputTokens>` `<elapsedSeconds>s` |
| Idle (after a turn) | `↑<sessionTotalIn> ↓<sessionTotalOut>` `<sessionTotalDuration>` |
| Process count > 0 | (always) `⚙ <count>` (clickable, opens swarm — unchanged from today) |
| Permission != Auto | (always, last item) `<permission-pill>` color-coded |

Rationale: tokens + elapsed are the two stats users actually read for live feedback. Cost and turn count move into the details panel — they're reference info, not live signal. The permission pill stays inline ONLY when it's *not* the default (Auto) — visible cue that "I'm in plan mode" / "I'm in bypass" without a click. Default mode = nothing visible = quiet UI.

### 3.3 Chevron — `▾` (collapsed) / `▴` (expanded)

- 16px font, far right of the strip, `var(--secondary-text-color)`.
- Affordance: cursor: pointer on the chevron AND the strip's empty-area (anywhere not a button/pill is clickable to expand).
- Hover state: chevron brightens to `--main-text-color`.
- Aria: `aria-expanded`, `aria-controls` linked to the panel's id.

---

## 4. Details panel — content rules

Renders below the strip when expanded. Single column, padded with `var(--space-2)`, separated rows by `1px solid var(--border-color)`:

### 4.0 Row 0 — activity log (NEW: moved here from main composer flow)

The full activity log lives here as the top section of the details panel. Scrollable, capped at `max-height: 200px` (about 8-10 lines at default density). Auto-scrolls to the latest entry on expand. Render is identical to today's `ActivityLogPanel` — same `logLines` source, same line styling, same truncation rules — only the parent container changes.

A subtle `─── Activity ───` divider sits above it so the section reads as a unit. When there are no log entries, the section is hidden entirely (no empty heading).

### 4.1 Row 1 — full session summary

```
Session: <num_turns> turns · $<cost_usd> · <total_in>k in / <total_out>k out
```

Shows even when idle. Hides individual zero values.

### 4.2 Row 2 — runtime config (today's AgentControlBar)

Three labeled dropdowns: Permission, Model, Effort. Same `PERMISSION_LABELS` / `MODEL_LABELS` / `EFFORT_LABELS` maps the AgentControlBar uses today, same change-on-next-turn semantics. The dropdowns dispatch the same RPCs.

### 4.3 Row 3 — tools used this session (optional, render only if any)

```
Tools so far: edit, bash, read, grep, hammer (×3)
```

De-duplicated, sorted by first use, with `(×N)` suffix for N > 1. Click a tool name → filter activity log to that tool (optional v2 feature; for v1 just display).

### 4.4 Row 4 — session management buttons

`[Archive session]` `[Export]` `[Restore]` — same buttons as today's AgentControlBar.

### 4.5 Animation

`max-height: 0 → max-height: 240px` transition over 120ms cubic-bezier(0.2, 1, 0.3, 1). Reverse on collapse. `overflow: hidden` clips content during the slide.

### 4.6 Auto-collapse triggers

- User clicks outside the panel (but inside the agent pane)
- User sends a message (focus returns to textarea, agent goes to work, panel hides — collapsing on send keeps users from sending-blind)
- User presses Esc while the textarea is focused AND empty (already a SIGINT signal — panel collapses as part of the same gesture)
- ESC while panel is focused but textarea is empty: collapse (same)
- Pressing the chevron again toggles

---

## 5. Component restructure

### 5.1 New components

- `frontend/app/view/agent/components/AgentComposerStrip.tsx` — the slim status row. Replaces `AgentStatusLine`. Receives the latest log line via a new prop `latestLogLine?: string`.
- `frontend/app/view/agent/components/AgentComposerDetails.tsx` — the expandable details panel. Absorbs `AgentControlBar` AND the full `ActivityLogPanel` content.

### 5.2 Deleted / replaced

- `AgentStatusLine` (in `AgentFooter.tsx`) — folded into `AgentComposerStrip`.
- `AgentControlBar.tsx` — folded into `AgentComposerDetails`. The component file can be deleted once the details panel ships.
- `ActivityLogPanel` — content moves into the details panel as its top section (§4.0). The current `ActivityLogPanel.tsx` component can be **kept** and reused inside `AgentComposerDetails`, just rendered in a different parent — no need to rewrite the log-line rendering itself.

### 5.3 Modified

- `frontend/app/view/agent/agent-view.tsx` (lines ~824–891 — the composer region JSX) — replace the stacked AgentStatusLine + ActivityLogPanel + AgentControlBar + AgentFooter pattern with:

```tsx
<div class="agent-composer-region">
    <AgentComposerStrip
        // existing AgentStatusLine props:
        loading={...} stopping={...} currentTool={...}
        sessionStats={...} turnTokens={...}
        processCount={...} onProcessBadgeClick={...}
        // new:
        latestLogLine={logLines().at(-1)?.text}          // for the live ticker
        unreadCount={unreadLogCount()}                    // for the chevron badge
        expanded={detailsOpen()}
        onToggleExpanded={() => setDetailsOpen(v => !v)}
        permissionMode={...}                              // for the inline non-default pill
    />
    <Show when={detailsOpen()}>
        <AgentComposerDetails
            blockId={model.blockId}
            blockAtom={block}
            providerId={provider()?.id ?? ""}
            sessionStats={...}
            logEntries={logLines()}                        // full log as top section
            onClose={() => setDetailsOpen(false)}
        />
    </Show>
    <AgentFooter ... />
</div>
```

### 5.4 State — reducer-integrated (Phase B pattern)

**Composer state lives in the existing `agent-pane-state` reducer slice** — not in component-local Solid signals. This matches the PR G architectural contract: the per-pane reducer is the single source of truth for everything-that-lives-as-long-as-the-pane (streaming, sessionStats, currentTool, turnTokens, pending, initPhase, turnPhase). Adding `detailsOpen` and `composerUnreadCount` to that slice keeps the contract intact and lets sagas/tests reason about the composer the same way they reason about the turn machine.

#### 5.4.1 New fields on `AgentPaneState`

`frontend/app/store/agent-pane-state/types.ts`:

```ts
export interface AgentPaneState {
    // …existing fields unchanged…

    /**
     * Whether the composer details panel is expanded. Default `false`.
     * Persists across renders within a pane lifetime; resets to `false`
     * on pane unmount (because a new pane gets a fresh state via
     * `initialState()`). Not persisted to backend (per-session
     * ephemeral preference — matches today's AgentControlBar behavior).
     */
    detailsOpen: boolean;

    /**
     * Number of activity-log entries that arrived while
     * `detailsOpen === false`. Resets to 0 on every transition
     * `detailsOpen: false → true`. Drives the chevron's unread badge
     * (`▾³`). The activity-log slice itself owns the entries; this
     * counter is a lightweight projection so the view doesn't have to
     * subscribe to both slices to render the badge.
     */
    composerUnreadCount: number;
}
```

Update `initialState(agentId)` to set both: `detailsOpen: false, composerUnreadCount: 0`.

#### 5.4.2 New commands on `AgentPaneCommand`

```ts
export type AgentPaneCommand =
    // …existing variants unchanged…

    // ── Composer details panel ─────────────────────────────────────
    /** User clicked the chevron / strip — toggle the details panel. */
    | { type: "DetailsToggle" }
    /** Explicit expand (e.g. keyboard shortcut, programmatic open). */
    | { type: "DetailsExpand" }
    /**
     * Explicit collapse. Fired by the outside-click handler in the
     * view, and by the `TurnStart` saga (send-message auto-collapses).
     */
    | { type: "DetailsCollapse" }
    /**
     * A new activity-log entry just landed. The activity-log hook
     * dispatches this AFTER its own slice's append, so the reducer
     * here can safely treat the count as already-incremented from the
     * activity slice's perspective. No-op when `detailsOpen === true`
     * (the user can see it; no need to flag).
     */
    | { type: "LogEntryArrived" };
```

#### 5.4.3 Reducer arms

In `frontend/app/store/agent-pane-state/reducer.ts`:

| Command | Transition | Side-effect events |
|---|---|---|
| `DetailsToggle` | `detailsOpen` flips; on flip-to-true, `composerUnreadCount → 0` | none |
| `DetailsExpand` | `detailsOpen → true`, `composerUnreadCount → 0` (idempotent if already true) | none |
| `DetailsCollapse` | `detailsOpen → false` (`composerUnreadCount` unchanged so unread accumulates fresh from now) | none |
| `LogEntryArrived` | if `!detailsOpen`: `composerUnreadCount++`; else: no-op | none |
| `TurnStart` (existing arm, extended) | also: `detailsOpen → false` (auto-collapse on send — same UX rule as today's "send-clears-focus") | existing events unchanged |

`TurnStart` extension is the only existing-arm change. It's a pure additional write to the same return value — no impact on existing turn-machine invariants.

#### 5.4.4 Tests

`reducer.test.ts` additions (one test per arm + the TurnStart cross-slice case). Patterns to follow:

```ts
it("DetailsToggle: closed → open resets unread counter", () => {
    let s = initialState("agent-1");
    s = { ...s, composerUnreadCount: 5 };
    const r = update(s, { type: "DetailsToggle" });
    expect(r.state.detailsOpen).toBe(true);
    expect(r.state.composerUnreadCount).toBe(0);
});

it("LogEntryArrived: increments only while collapsed", () => {
    let s = initialState("agent-1");
    s = update(s, { type: "LogEntryArrived" }).state;
    s = update(s, { type: "LogEntryArrived" }).state;
    expect(s.composerUnreadCount).toBe(2);
    s = update(s, { type: "DetailsExpand" }).state;
    s = update(s, { type: "LogEntryArrived" }).state;
    expect(s.composerUnreadCount).toBe(0);  // panel open → no-op
});

it("TurnStart auto-collapses the details panel", () => {
    let s = initialState("agent-1");
    s = update(s, { type: "DetailsExpand" }).state;
    s = update(s, { type: "InitReady", at: 100 }).state;
    s = update(s, { type: "StreamSubscribe", at: 100 }).state;
    const r = update(s, { type: "TurnStart", at: 200 });
    expect(r.state.detailsOpen).toBe(false);
});
```

#### 5.4.5 Atom projection

`frontend/app/view/agent/state.ts` (where `createAgentAtoms` lives) gets two new atoms projected from the slice — same pattern as the existing `turnPhaseAtom`, `currentToolAtom`, `sessionStatsAtom`:

```ts
detailsOpenAtom: atomFromSelector((s) => s.detailsOpen),
composerUnreadCountAtom: atomFromSelector((s) => s.composerUnreadCount),
```

#### 5.4.6 View wiring (replaces §5.3's local signal block)

In `agent-view.tsx`:

```tsx
<AgentComposerStrip
    // …turn/stats/tools props unchanged…
    latestLogLine={logLines().at(-1)?.text}
    unreadCount={agentAtoms().composerUnreadCountAtom[0]()}
    expanded={agentAtoms().detailsOpenAtom[0]()}
    onToggleExpanded={() => dispatchPane(model.blockId, { type: "DetailsToggle" }, "user")}
    permissionMode={...}
/>
<Show when={agentAtoms().detailsOpenAtom[0]()}>
    <AgentComposerDetails
        blockId={model.blockId}
        blockAtom={block}
        providerId={provider()?.id ?? ""}
        sessionStats={...}
        logEntries={logLines()}
        onClose={() => dispatchPane(model.blockId, { type: "DetailsCollapse" }, "user")}
    />
</Show>
```

The `useActivityLog` hook (which today owns `logLines` via the `agentActivity` slice) gets one new line: dispatch `LogEntryArrived` to the pane reducer after each append. That's the only cross-slice wiring needed — it's a fire-and-forget projection signal.

#### 5.4.7 Why reducer-owned, not Solid-local

| Concern | Local-signal approach | Reducer approach |
|---|---|---|
| Single source of truth | New tier outside the contract | Same contract as everything else |
| Test replay-ability | Solid-only; component must mount | Pure-function tests; same harness as 60+ existing reducer tests |
| Cross-slice coordination (e.g. `TurnStart` auto-collapse) | Each consumer wires its own effect | One reducer arm |
| Saga integration (future "collapse if idle 10min") | Side effect outside reducer | Cleanly an event from the reducer |
| HMR / hot-reload survival | Signals reset on remount | Reducer state survives if its dispatcher does (same as existing pane slots) |

Mirrors how `turnPhase` displaced the legacy `turnActive` / `stopping` / `streaming.active` booleans in PR G — every new "this state lives at the pane level" addition goes the same way.

### 5.5 SCSS

- `frontend/app/view/agent/styles/_composer-strip.scss` — new file for the strip layout (flex row, spinner SVG keyframes, chevron rotation, button reset).
- `frontend/app/view/agent/styles/_composer-details.scss` — new file for the details panel (slide-in animation, dropdown styling that matches the old control-bar, button row).
- `_control-bar.scss` — keep for now as the source of dropdown chrome (referenced from `_composer-details.scss`), delete in a follow-up once the details panel is verified visually.

---

## 6. Edge cases

| Case | Behaviour |
|---|---|
| Pane is very narrow (<240px) | Strip content right-truncates with `text-overflow: ellipsis`; the right segment hides items low-to-high priority: drop `↑↓ tokens` first, then `Ns`, then process badge stays last. Chevron always visible. |
| Agent has no `sessionStats` yet (fresh pane) | Strip-right shows nothing except process badge (if any) + chevron. Panel still expands but shows only the runtime-config row and the buttons (greyed Archive/Export until line_count > 0). |
| Disconnected | `AgentDisconnectedBanner` stays as-is (renders above the activity log, separate concern). Strip shows last-known state but greyed out; details panel disabled until reconnect. |
| User types into textarea while panel is open | Panel stays open; sending the message collapses it. |
| Voice input writing into textarea via mic | Same as keyboard — no special behaviour. |
| Slash autocomplete dropdown | Renders above the strip as today (anchored to the textarea, not the strip). No conflict. |
| `--prefers-reduced-motion` | All animations (spinner rotation, panel slide, chevron flip, "✓" fade) disabled per `mixins.respect-reduced-motion` — strip just toggles instantly, no rotation on the spinner. |

---

## 7. Acceptance criteria

1. **Composer region is at most 2 bands tall when panel is closed** (slim strip + textarea-with-hint).
2. **Strip height ≤ 32px** in default theme.
3. **Idle pane shows an essentially empty strip** — just the chevron (and process badge if any). Last log line lingers if a prior session ran in this pane.
4. **Working state** shows a small animated spinner + the **latest activity-log line** (truncated) + at-most-3 stats on the right + chevron.
5. **Chevron carries an unread count** (`▾³`) when activity-log entries arrive while the panel is collapsed; resets to 0 on expand.
6. **Click anywhere on the strip outside an inline button** opens the details panel.
7. **Details panel top section is the full activity log** (scrollable, capped ~200px), followed by session/runtime/tools/buttons sections.
8. **Details panel shows every option that was in `AgentControlBar`** — permission, model, effort, archive/export/restore.
9. **Outside-click and send-message both collapse the panel.**
10. **All ARIA attributes wired** — `aria-expanded`, `aria-controls`, `aria-label` on the strip's expand trigger; the activity log section inside the panel reads as a labeled region.
11. **Reduced-motion is honoured** — no spinning, no slide animation, instant toggle.
12. **No regressions in narrow-pane modal compact behaviour** (`MODAL_COMPACT_VARIANT_ARCHITECTURE_2026_05_26.md` Phases 1+2 already merged — this work is composer-only).

---

## 8. Out of scope (deferred follow-ups)

- **Tool-name → activity-log filter** (mentioned in §4.3) — v2.
- **Persisting the expanded/collapsed preference across reloads** — keep ephemeral for v1; revisit if users ask for it.
- **Drag-to-resize for the details panel** — fixed `max-height: 240px` for v1, animate to `auto` if real content exceeds.
- **Renaming the agent inline from within the details panel** — separate spec.
- **Strip stickiness during scroll** — the strip is already at the bottom of the composer region (always pinned). No change.

---

## 9. Migration / rollout

Single PR ships:
1. New `AgentComposerStrip.tsx` + `_composer-strip.scss`.
2. New `AgentComposerDetails.tsx` + `_composer-details.scss` (carries over AgentControlBar logic).
3. Modified `agent-view.tsx` composer-region JSX.
4. **Deletes `AgentStatusLine` from `AgentFooter.tsx`** (no remaining caller after the JSX swap).
5. **Deletes `AgentControlBar.tsx`** (no remaining caller).
6. **Deletes `_control-bar.scss`** (or migrates its dropdown chrome into `_composer-details.scss`).

Estimated diff: ~250 lines added (new components + styles), ~180 lines deleted (old components + styles). Net positive ~70 lines, but visually the surface is smaller.

No backend changes, no IPC changes, no spec impact outside this file.

---

## 10. Open questions

1. **Should the spinner show the current tool's icon** instead of a generic circle? Tools have distinct identity (bash, edit, grep); a tiny icon would convey more. Cost: maintaining the icon set. *Recommendation: defer to v2; ship circle first.*
2. **Should the strip persist a "click to see what changed since you last looked" affordance** when new tools fire? E.g. a small unread-dot on the chevron. *Recommendation: defer; if requested, add a `data-unread` state to the chevron.*
3. **Should the details panel render above the strip** (sliding down from below the activity log) or **below the strip** (sliding up between strip and textarea)? Below = doesn't shift the textarea (more stable). Above = visually closer to the activity log it summarizes. *Recommendation: BELOW the strip, ABOVE the textarea (as in §2.3 mockup) — keeps the textarea at a predictable screen position so users don't lose their typing focus.*

---

## 11. Files this redesign touches

```
# Reducer slice (Phase B integration)
frontend/app/store/agent-pane-state/types.ts                    +2 fields, +4 commands
frontend/app/store/agent-pane-state/reducer.ts                  +4 arms, extend TurnStart
frontend/app/store/agent-pane-state/reducer.test.ts             +tests for new arms

# Atom projection
frontend/app/view/agent/state.ts                                +2 projected atoms

# Cross-slice wiring
frontend/app/view/agent/hooks/useActivityLog.ts                 dispatch LogEntryArrived

# New view components
frontend/app/view/agent/components/AgentComposerStrip.tsx       NEW
frontend/app/view/agent/components/AgentComposerDetails.tsx     NEW
frontend/app/view/agent/styles/_composer-strip.scss             NEW
frontend/app/view/agent/styles/_composer-details.scss           NEW

# Replacements / deletions
frontend/app/view/agent/components/AgentFooter.tsx              DELETE AgentStatusLine export
frontend/app/view/agent/components/AgentControlBar.tsx          DELETE
frontend/app/view/agent/styles/_control-bar.scss                DELETE (or absorb)
frontend/app/view/agent/agent-view.tsx                          REPLACE composer-region JSX
```

---

*End of spec. Ready for review + go/no-go decision.*
