# SPEC: Agent Pane — Status Zones Reorganization + Gradient Progress Bar

**Date:** 2026-06-14  
**Status:** Draft  
**Affects:** `frontend/app/view/agent/`

---

## Overview

Three UI improvements to the agent pane:

1. **Working indicator → conversation bottom** — move the spinner + "Working…" text + elapsed timer from the `AgentStatusLine` strip into the bottom of the conversation area (`AgentDocumentView`). Turn-complete summary ("✓ Worked · 42s") also appears there.
2. **Aux info strip** — repurpose the freed `AgentStatusLine` strip to show static runtime context: model, effort, permission mode.
3. **Gradient progress bar** — a thin (2px) animated gradient bar pinned to the very top of the agent pane as a secondary in-progress signal.

---

## 1. Current Architecture

**Files:**
- `frontend/app/view/agent/agent-view.tsx` — main layout
- `frontend/app/view/agent/components/AgentFooter.tsx` — `AgentStatusLine` + `AgentFooter`
- `frontend/app/view/agent/components/AgentControlBar.tsx` — collapsible model/effort/permission controls
- `frontend/app/view/agent/styles/_control-bar.scss` — spinner, status line, control bar styles

**Current layout order (agent-view.tsx lines 514–611):**

```
AgentDocumentView           ← scrollable conversation
agent-retry-bar             ← conditional
PendingMessagesPanel        ← message queue
AgentStatusLine             ← ⚠ CURRENT HOME of spinner + worked stats (to be split)
ActivityLogPanel            ← collapsible diagnostic log
agent-composer-region
  ├── AgentControlBar       ← collapsible: mode/model/effort + session controls
  └── AgentFooter           ← textarea input
```

**What `AgentStatusLine` currently does (AgentFooter.tsx:55–183):**

| State | Renders |
|-------|---------|
| `loading=true` | `● Working… · {elapsed}` (left) + `↑tokens ↓tokens · Ns` (right) |
| `loading=false, sessionStats` | `✓ Worked · Ns · ↑tokens` (left) + `$cost · N turns` (right) |
| idle (no stats) | empty `<span>` with optional process badge |

The "loading" state is what the user calls **"the spinner"**. The `AgentStatusLine` strip (between `PendingMessagesPanel` and `ActivityLogPanel`) is what they're calling the **"aux info container"**.

---

## 2. Change 1 — Working Indicator → Bottom of Conversation

### Goal
The spinner dot + "Working…" text + live elapsed timer should appear as the last visible item in the conversation area — visually at the bottom of the feed, like a chat UI's typing indicator. Same for the "Worked · 42s" completion line.

### Approach
Render a new **`AgentWorkingRow`** component as a direct sibling immediately below `AgentDocumentView`, before `PendingMessagesPanel`. Style it without a separating border so it reads as continuous with the conversation.

This is the simplest path — `AgentDocumentView` owns its own scroll container, so threading a sticky node into the document feed would require adding a new document node type. A sibling div with matching background is visually equivalent and avoids coupling the loading state into the document model.

### New component: `AgentWorkingRow`

**File:** `components/AgentFooter.tsx` (new export, or extract to `components/AgentWorkingRow.tsx`)

```tsx
interface AgentWorkingRowProps {
    loading: boolean;
    stopping: boolean;
    currentTool: string | null;
    sessionStats: SessionStats | null;
    turnTokens: TurnTokens | null;
}

export const AgentWorkingRow = (props: AgentWorkingRowProps): JSX.Element => {
    // Elapsed timer — same logic as current AgentStatusLine
    // Phrase cycling — same as current AgentStatusLine
    // Worked summary — same as current workedPrimary/workedSecondary memos
    //
    // Show=loading: spinner dot + "Working… · Ns · ↑tokens"
    // Show=!loading && sessionStats: "✓ Worked · 42s · ↑2.1k ↓890"  [fade after ~5s? TBD]
    // Show=neither: null (no empty placeholder)
}
```

**Placement in agent-view.tsx:**

```tsx
<AgentDocumentView ... />

{/* Working indicator — shows in conversation area as last visual item */}
<Show when={status.isLoading() || agentAtoms().turnActiveAtom[0]() || stoppingAtom[0]() || agentAtoms().sessionStatsAtom[0]()}>
    <AgentWorkingRow
        loading={status.isLoading() || agentAtoms().turnActiveAtom[0]() || stoppingAtom[0]()}
        stopping={stoppingAtom[0]()}
        currentTool={agentAtoms().currentToolAtom[0]()}
        sessionStats={agentAtoms().sessionStatsAtom[0]()}
        turnTokens={agentAtoms().turnTokensAtom[0]()}
    />
</Show>

<PendingMessagesPanel ... />
```

### Styling

```scss
.agent-working-row {
    // No border-top — continues visually from the document feed
    padding: var(--space-1) var(--space-3) var(--space-1-5);
    font-size: 11px;
    font-family: var(--fixed-font, monospace);
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
    color: var(--accent-color);
    // Light background blend to distinguish from document content
    background: color-mix(in srgb, var(--accent-color) 3%, transparent);

    &--stats {
        color: var(--secondary-text-color);
        opacity: 0.6;
        background: transparent;
    }

    .agent-working-row-left {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .agent-working-row-right {
        flex-shrink: 0;
        opacity: 0.7;
    }
}
```

The spinner dot and `agent-spotlight-sweep` animation (currently in `_control-bar.scss`) move to the working row's loading state.

**Spinner dot — theme-derived colors:**

The current `_control-bar.scss` definition has a hardcoded glow:
```scss
// ❌ current — hardcoded, breaks on Dracula/purple, Gruvbox/orange, etc.
box-shadow: 0 0 6px rgba(140, 200, 255, 0.55);
```

Replace with `color-mix()` against the theme accent so the glow matches every theme:
```scss
.agent-spinner-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent-color);
    flex-shrink: 0;
    animation: agent-pulse 1.2s ease-in-out infinite;
    // Glow derived from accent — works across all themes automatically
    box-shadow: 0 0 6px color-mix(in srgb, var(--accent-color) 60%, transparent);
}
```

The spotlight sweep overlay also uses a hardcoded `rgba(224, 170, 255, 0.30)` magenta. Replace with the accent:
```scss
// ❌ current — magenta hardcode looks wrong on blue/cyan/orange themes
background: linear-gradient(
    90deg,
    transparent 0%,
    rgba(224, 170, 255, 0.10) 35%,
    rgba(224, 170, 255, 0.30) 50%,
    rgba(224, 170, 255, 0.10) 65%,
    transparent 100%
);

// ✅ theme-derived sweep
background: linear-gradient(
    90deg,
    transparent                                                   0%,
    color-mix(in srgb, var(--accent-color) 12%, transparent)    35%,
    color-mix(in srgb, var(--accent-color) 28%, var(--main-text-color)) 50%,
    color-mix(in srgb, var(--accent-color) 12%, transparent)    65%,
    transparent                                                  100%
);
```

This makes the spotlight glint track the theme accent rather than always being magenta, while the `mix-blend-mode: screen` behavior is preserved.

### Worked / turn-complete behavior

**Decision:** No auto-dismiss. The "Worked · 42s" line stays visible until the user sends the next message. It acts as a visual delimiter between turns — the user can see exactly where one turn ended and the next began. `setSessionStats(null)` in `handleSendMessage` clears it naturally on send.

---

## 3. Change 2 — Aux Info Strip (Model / Effort / Permission)

### Goal
The `AgentStatusLine` strip between `PendingMessagesPanel` and `ActivityLogPanel` becomes a static "aux info" display showing the agent's current runtime config: permission mode, model, and effort level.

### New component: `AgentAuxInfoBar`

Replaces `<AgentStatusLine>` in agent-view.tsx.

```tsx
interface AgentAuxInfoBarProps {
    blockAtom: () => Block | undefined;
    providerId: string;
    processCount?: number;
    onProcessBadgeClick?: () => void;
}

export const AgentAuxInfoBar = (props: AgentAuxInfoBarProps): JSX.Element => {
    // Reads runtime config from blockAtom meta (same as AgentControlBar)
    // Renders: "{permMode} · {model} · Effort: {effort}"
    // Right side: process badge (kept here since it's always contextual)
    // Optional: click to toggle AgentControlBar expand (UX to decide at impl time)
}
```

**Placement in agent-view.tsx:** Unchanged position — replace `<AgentStatusLine>` with `<AgentAuxInfoBar>`.

**Content example:**
```
Default · Sonnet · Effort: High                                    ⚙ 2
```

**Styling:** Reuse the existing `> .agent-status-line` rules in `_control-bar.scss` — just change content. The element keeps its 10px font, monospace, secondary-text-color, opacity 0.6 look.

**Relationship with `AgentControlBar`:** The `AgentControlBar` still exists — `AgentAuxInfoBar` is a read-only summary. The expand/collapse toggle stays on the `AgentControlBar`'s own header chevron. The two are visually distinct: `AgentAuxInfoBar` is between log and pending messages; `AgentControlBar` is between `AgentFooter` and the bottom of the pane.

### What to remove from `AgentStatusLine`

The old `AgentStatusLine` component is fully replaced. Its three rendering cases map to:

| Old case | New owner |
|----------|-----------|
| `loading=true` (spinner) | `AgentWorkingRow` |
| `loading=false, sessionStats` (Worked stats) | `AgentWorkingRow` |
| idle (process badge only) | `AgentAuxInfoBar` (right side) |

---

## 4. Change 3 — Gradient Progress Bar (Top of Pane)

### Inspiration

**Ghostty 1.2.0** (released Oct 2025, also in iTerm2 3.6.6 via OSC 9;4):  
The first macOS terminal to render a progress bar as a thin bar at the very top of the terminal pane — separate from text content. Blue by default, red on error, animated left-to-right bounce for indeterminate state. Per-split, not global.

**NProgress** (used by YouTube, GitHub, Medium):  
The canonical web "top loading bar" — 3px, `position: fixed; top: 0`, animated trickle that never reaches 100% until `.done()`. Color `#2299DD`, glow shadow.

**Linear app:**  
2px purple-to-teal gradient, `background-position` animated at 2s linear infinite. Colors: `#5E6AD2 → #26BFBF`.

### CSS technique: background-position animation

The canonical method — more performant than animating `width` or `left`:

```css
@keyframes agent-gradient-sweep {
    from { background-position: 200% 0; }
    to   { background-position: -200% 0; }
}
```

The gradient is made 200–300% wider than the element. Animating `background-position` slides the "window" across it — creates flowing light with zero layout cost. Runs on compositor thread only.

### Spec

**New component: `AgentPaneProgressBar`** (inline JSX in agent-view.tsx, or tiny component)

```tsx
// In agent-view.tsx, first child inside .agent-view:
<div
    class="agent-pane-progress-bar"
    classList={{
        "agent-pane-progress-bar--active": isLoading(),
        "agent-pane-progress-bar--stopping": stoppingAtom[0](),
    }}
    role="progressbar"
    aria-label="Agent working"
    aria-valuemin={0}
    aria-valuemax={100}
    // omit aria-valuenow for indeterminate
/>
```

Where `isLoading()` = `status.isLoading() || agentAtoms().turnActiveAtom[0]() || stoppingAtom[0]()`.

**Styles** (add to `_control-bar.scss` or new `_progress-bar.scss`):

```scss
// Scoped inside .agent-view (which already has position: relative)
.agent-pane-progress-bar {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 2px;
    z-index: 10;
    pointer-events: none; // never intercepts clicks
    opacity: 0;
    transition: opacity 200ms ease;

    /*
     * Color strategy: no hardcoded hex — derive entirely from theme tokens.
     *
     * Every theme overrides --accent-color and --main-text-color.
     * color-mix(in srgb, accent 55%, main-text) produces the brightest
     * in-theme point: near-white glint on dark themes, near-black dip on
     * light themes. This maximises contrast within the gradient automatically
     * across all 8 shipped themes without any per-theme override.
     *
     * Gradient shape (200% wide, swept left→right by background-position):
     *   0%   fade-in edge  (accent at 35% opacity → transparent)
     *  25%   full accent
     *  50%   accent + 55% main-text = brightest "glint" peak
     *  75%   full accent
     * 100%   fade-out edge (accent at 35% opacity → transparent)
     */
    background: linear-gradient(
        90deg,
        color-mix(in srgb, var(--accent-color) 35%, transparent)            0%,
        var(--accent-color)                                                  25%,
        color-mix(in srgb, var(--accent-color) 55%, var(--main-text-color)) 50%,
        var(--accent-color)                                                  75%,
        color-mix(in srgb, var(--accent-color) 35%, transparent)           100%
    );
    background-size: 200% 100%;

    &--active {
        opacity: 1;
        animation: agent-gradient-sweep 2s linear infinite;
    }

    // Stopping: keep bar visible, dim slightly to signal "winding down"
    &--stopping {
        opacity: 0.55;
    }
}

@keyframes agent-gradient-sweep {
    from { background-position: 200% 0; }
    to   { background-position: -200% 0; }
}

// Accessibility: no motion — static two-stop gradient still signals "active"
@media (prefers-reduced-motion: reduce) {
    .agent-pane-progress-bar--active {
        animation: none;
        background: linear-gradient(
            90deg,
            color-mix(in srgb, var(--accent-color) 30%, transparent) 0%,
            var(--accent-color)                                       50%,
            color-mix(in srgb, var(--accent-color) 30%, transparent) 100%
        );
        background-size: 100% 100%;
    }
}
```

**Height:** 2px. Peripheral signal — should be noticed in peripheral vision, not command attention.

### `.agent-view` already has `position: relative`

Confirmed in `agent-view.scss:44`. The bar's `position: absolute; top: 0` will work without modifications to the root element.

---

## 5. Layout After Changes

```
.agent-view  (position: relative)
│
├── AgentPaneProgressBar      ← NEW: position: absolute; top: 0; height: 2px
│                                    gradient shimmer while loading, hidden at rest
│
├── BookmarksPanel            (conditional, unchanged)
├── AgentSearchBar            (unchanged)
├── SessionDigestBanner       (conditional, unchanged)
├── AgentFocusedPanel         (overlay, unchanged)
│
├── AgentDocumentView         ← conversation (unchanged internally)
│
├── AgentWorkingRow           ← NEW: "● Working… · 12s" or "✓ Worked · 42s"
│                                    shown only when loading || sessionStats
│                                    styled to read as part of conversation
│
├── agent-retry-bar           (conditional, unchanged)
├── PendingMessagesPanel      (unchanged)
│
├── AgentAuxInfoBar           ← REPURPOSED (was AgentStatusLine):
│                                    "Default · Sonnet · Effort: High  ⚙ 2"
│                                    static, always visible when provider=claude
│
├── ActivityLogPanel          (unchanged)
│
└── agent-composer-region
    ├── SlashHelpPanel        (conditional, unchanged)
    ├── SlashCommandPicker    (conditional, unchanged)
    ├── AgentControlBar       ← still exists; expand/collapse for editing settings
    └── AgentFooter           ← textarea (unchanged)
```

---

## 6. File Change Summary

| File | Change |
|------|--------|
| `agent-view.tsx` | Add `<AgentPaneProgressBar>` as first child of `.agent-view--presentation`; add `<AgentWorkingRow>` below `<AgentDocumentView>`; replace `<AgentStatusLine>` with `<AgentAuxInfoBar>` |
| `components/AgentFooter.tsx` | Extract `AgentWorkingRow` (loading + worked states); extract or rename `AgentStatusLine` to `AgentAuxInfoBar` (model/effort/permission display); keep `AgentFooter` unchanged |
| `styles/_control-bar.scss` | Add `.agent-pane-progress-bar` + `@keyframes agent-gradient-sweep`; update `> .agent-status-line` rules for aux-info role; move spinner/spotlight styles to `.agent-working-row` |

Optional new file: `components/AgentWorkingRow.tsx` if extracted from `AgentFooter.tsx` for clarity.

---

## 7. Open Questions

1. ~~**Worked line auto-dismiss?**~~ **Decided:** no auto-dismiss — the line stays as a turn delimiter until the user sends the next message.
2. **Aux info bar click target?** Should clicking the `AgentAuxInfoBar` expand/collapse `AgentControlBar`? Would make the bar feel interactive and discoverable as a settings toggle.
3. **Progress bar colors?** accent→violet→sky gradient, or a simpler solid accent with opacity pulse? Or provider-branded colors?
4. **Stopping state bar behavior?** Keep full animation, reduce opacity to ~0.6, or switch to a slow solid fade?
5. **Provider scope?** `AgentControlBar` currently only renders for `providerId === "claude"`. Should `AgentAuxInfoBar` follow the same rule or always show?

---

## 8. Reference

- Ghostty 1.2.0 progress bar: https://ghostty.org/docs/install/release-notes/1-2-0
- Martin Emde on Ghostty bars: https://martinemde.com/blog/ghostty-progress-bars
- iTerm2 3.6.6 OSC 9;4: https://iterm2.com/documentation-escape-codes.html
- NProgress: https://github.com/rstacruz/nprogress
- Current spotlight sweep animation: `_control-bar.scss:121–128`
- Current status line position decision: `_control-bar.scss:61–71` (comment explains prior move)
- Prior zone order spec: `docs/specs/SPEC_AGENT_PANE_ZONE_ORDER_WORKED_FOOTER_2026_04_24.md`
