# SPEC: Agent Composer Strip Redesign — Model/Effort Dropdowns + Shell History + Context Text

**Date:** 2026-06-23  
**Status:** Draft  
**Scope:** `AgentComposerStrip`, `ContextWindowBar`, `AgentControlBar`, `AgentFooter`  
**Trigger:** UX feedback — the strip is opaque; the context bar is unreadable; the
model/effort selectors are hidden behind a chevron expand; shell history has no dedicated
surface.

---

## 0. Problem statement

The current `AgentComposerStrip` (28–32px status row above the textarea) packs everything
into a single clickable bar:

```
[⟳ bash]                ↑2.1k ↓0.4k  30s  ⚙3  Bypass  [■■■░░░░░ 12k / 64k]  ▾
```

Four usability problems:

1. **Context bar is unreadable.** The filled-track graphic conveys fill% but the exact
   token counts require a tooltip hover. Users want to glance at "used / total" as plain
   text.

2. **Model/effort are buried.** Changing model or effort requires clicking the chevron,
   waiting for the details panel to expand, then finding the right `<select>`. These are
   high-frequency actions — users switch effort before long tasks and switch model to
   Opus/Haiku as needed.

3. **No shell history surface.** The sent-message history (ArrowUp/ArrowDown recall in
   `AgentFooter`) exists but is invisible. Power users want a quick-access panel showing
   recent messages they sent, so they can re-send or review without navigating the
   conversation.

4. **Three redundant loading indicators.** The pane shows: (1) animated ants in the tab
   bar, (2) `AgentWorkingRow` just above the composer showing "Working…" or the tool
   name, and (3) the strip's left zone showing `⟳ bash`. The strip zone duplicates #2
   and adds noise without adding information. Remove the strip left zone; enrich #2
   instead.

---

## 1. Design

### 1.1 New strip layout

The strip becomes a responsive row of **three inline controls** + the right-side status:

```
[Model ▾]  [Effort ▾]  [Shell]    ↑2.1k ↓0.4k  30s  12.1k / 64k  ▾
```

The left-zone spinner + tool name (`⟳ bash`) is **removed** — see §1.2. Left to right:

| Zone | Content |
|---|---|
| **Controls** | `[Model ▾]` dropdown · `[Effort ▾]` dropdown · `[Shell]` toggle button |
| **Right** | Token stats · elapsed · process badge · permission pill · **context text** · chevron |

The three controls are always visible in the strip — no expand required to change model
or effort.

### 1.2 AgentWorkingRow enrichment

`AgentWorkingRow` (`AgentFooter.tsx:48`) is rendered directly below the conversation
document, immediately above the composer strip. It is already the canonical loading
indicator — removing the strip's left zone makes it the sole in-flight status display
for what the agent is doing.

**Current output** (loading state, `AgentFooter.tsx:124–133`):

```
⊙  bash                               ↑2.1k ↓0.4k  ·  30s
   OR
⊙  Working…                           ↑2.1k ↓0.4k  ·  30s
```

**Enriched output:**

```
⊙  read  ·  providers/claude.ts       ↑2.1k ↓0.4k  ·  30s
   OR
⊙  bash  ·  cargo test --lib          ↑2.1k ↓0.4k  ·  30s
   OR
⊙  Working…                           ↑2.1k ↓0.4k  ·  30s
```

- Add a `currentToolArg?: string | null` prop to `AgentWorkingRowProps` alongside the
  existing `currentTool`. The caller (`agent-view.tsx`) populates this from the most
  recent tool-call event's first significant argument (file path for `read`/`write`,
  command string for `bash`, query for `search`, etc.), already available on the stream
  event that sets `currentTool`.
- Render: `{currentTool}  ·  {abbreviate(currentToolArg, 40)}` in `agent-working-row-left`
  when both are set; fall back to `{currentTool}` or `phrase()` as today.
- `abbreviate(s, 40)`: truncate to 40 chars, ellipsis on overflow. For file paths, prefer
  truncating from the left (show the filename, not the root): `…/providers/claude.ts`.
- When `props.stopping` remains highest priority: shows "Stopping…" regardless.

**Files changed:** `AgentFooter.tsx` (new prop + render logic), `AgentFooter.scss` (no
new rules needed — existing `.agent-working-row-left` handles truncation).

---

### 1.3 Model dropdown

- A `<select>` (or custom styled button+popover) directly in the strip showing the
  current model's short label: `Opus`, `Sonnet`, `Haiku`.
- Width: fixed at ~80px, enough for the longest label.
- On change: calls existing `applyRuntimeChange()` — same as current `AgentControlBar`.
- Only rendered for providers that support model switching (currently `providerId ===
  "claude"`, matching the existing Phase 1 gate).
- When no provider is set: hidden (same as today).

### 1.4 Effort dropdown

- A `<select>` showing the current effort label: `Low`, `Med`, `High`, `X-High`, `Max`.
- Width: fixed at ~72px.
- On change: calls `applyRuntimeChange()` — same as current.
- Always visible when the model dropdown is visible.

### 1.5 Shell history toggle

- A button labelled **`Shell`** (text label, no icon) immediately after the effort
  dropdown.
- Toggles a **shell history panel** that expands above the strip (same expand direction
  as the existing details panel).
- The shell history panel and the existing details panel (activity log + control bar)
  are **mutually exclusive**: opening one closes the other. Only one panel can be open
  at a time.
- The button gets a subtle active/highlighted state when the panel is open.
- State: lives in the pane reducer (`AgentPaneState`) as a new `shellOpen: boolean`
  field alongside the existing `detailsOpen`.

### 1.6 Shell history panel

A new collapsible panel that expands above the strip (above the composer region) when
`shellOpen` is true.

**Content:** The last N (default 50) messages the user has sent in this pane's composer,
in reverse-chronological order (newest at top). This is the same history array already
maintained by `AgentFooter` for ArrowUp/ArrowDown recall — the panel simply surfaces it
visually.

**Layout:**
```
┌─────────────────────────────────────────────────────────────┐
│  Shell History                                    [× Close] │
├─────────────────────────────────────────────────────────────┤
│  fix the auth bug in providers.rs                           │
│  run the tests                                              │
│  what's in src/commands/mod.rs                              │
│  explain the sandbox flow                                   │
│  …                                                          │
└─────────────────────────────────────────────────────────────┘
```

- Each row is single-line, truncated with ellipsis. Click → sends that message again
  (pre-fills the composer and submits, same as ArrowUp recall then Enter). No hover
  re-send; an explicit click is required to avoid accidents.
- The panel is scrollable (max-height ~200px, overflow-y: auto).
- Empty state: "No messages sent yet." in secondary text.
- The history is session-local (in-memory, same as today's ArrowUp history) — it does
  not persist across reloads.

### 1.7 Context window: text only

**Remove the `ContextWindowBar` fill-track graphic entirely.** Replace with plain text in
the strip right zone:

```
12.1k / 64k
```

- When the context window is unknown (provider doesn't report it): show just `12.1k ctx`
  (same as the existing `ctx-window-raw` fallback, but inline in the strip instead of
  the graphic).
- Color band (low/mid/high/critical) is preserved as text color on the number:
  - `low` → secondary text color (muted)
  - `mid` → main text color
  - `high` → `var(--warning-color)`
  - `critical` → `var(--error-color)`
- Tooltip: unchanged from current `contextTitle()` — full token counts + auto-compact
  threshold.
- `ContextWindowBar.tsx` is deleted (or kept as a dead export with a deprecation comment
  if other consumers exist — check before deleting).

---

## 2. What changes, what stays

### Stays unchanged

- `AgentWorkingRow` — enriched per §1.2 but not replaced.
- Token stats (`↑in ↓out`), elapsed, process badge, permission pill.
- Chevron (`▾ / ▴`) toggling the existing details panel (activity log + full control bar).
- `AgentControlBar` remains in the details panel for the permission mode selector and
  Archive/Export actions — it now duplicates model/effort (which also appear in the
  strip) but that duplication is acceptable: the strip dropdowns are the fast path, the
  control bar is the complete view.
- `ActivityLogPanel` inside the details panel: unchanged.
- ArrowUp/ArrowDown history in `AgentFooter`: unchanged.

### Removed

- Strip left zone (spinner + `⟳ tool` name) — information now lives in `AgentWorkingRow`.
- `ContextWindowBar` fill-track graphic — replaced by inline text in the strip.

### Added

- `currentToolArg` prop on `AgentWorkingRow` — enriched display in the working row.
- Model `<select>` inline in strip.
- Effort `<select>` inline in strip.
- `Shell` toggle button inline in strip.
- Shell history panel (new component: `AgentShellHistoryPanel.tsx`).
- `shellOpen: boolean` field in `AgentPaneState` reducer.
- `ShellToggle`, `ShellExpand`, `ShellClose` reducer actions.
- `app/store/shell-history.ts` module-level registry.
- `AgentFooter` calls `getShellHistory(blockId).push(text)` on send.

---

## 3. Responsive behaviour

The strip is 28–32px tall and must work at any pane width. Priority order when horizontal
space is constrained:

1. Left activity text truncates first (already truncates via `text-overflow: ellipsis`).
2. Context text (`12.1k / 64k`) collapses to `12.1k` at narrow widths.
3. Model and Effort dropdowns collapse to icon-only (`M ▾`, `E ▾`) at narrow widths,
   then are pushed into a combined `⚙ ▾` overflow button at very narrow widths.
4. Shell button collapses to `Sh` at narrow widths.
5. Token stats / elapsed / permission pill hide at very narrow widths (same as today).

Breakpoints: implement via CSS container queries on `.agent-composer-strip`, matching
the existing `narrow` / `very-narrow` pattern used in the agent pane.

---

## 4. Reducer changes

`AgentPaneState` currently has:

```typescript
detailsOpen: boolean
composerUnreadCount: number
```

Add only:

```typescript
shellOpen: boolean   // shell history panel visible
```

New actions (extend existing `AgentPaneCommand` union — mirrors the `Details*` trio):

```typescript
| { type: "ShellToggle" }   // flip open/closed; mutual-exclusion with detailsOpen
| { type: "ShellExpand" }   // idempotent open (keyboard / programmatic)
| { type: "ShellClose" }    // idempotent close
```

Reducer arms:

- `ShellToggle`: sets `shellOpen = !shellOpen`; if opening, also sets `detailsOpen = false`
  (mutual exclusion — only one panel open at a time). Emits no events (same as `DetailsToggle`
  — UI-only state, no downstream sagas need to know).
- `ShellExpand`: idempotent `shellOpen = true`, `detailsOpen = false`. No-op same-ref when
  already open.
- `ShellClose`: idempotent `shellOpen = false`. No-op same-ref when already closed.

Existing arm updates:

- `DetailsToggle / DetailsExpand`: add `shellOpen: false` to the state spread (mutual
  exclusion — opening details closes shell).
- `TurnStart`: add `shellOpen: false` alongside the existing `detailsOpen: false`
  auto-collapse. The user pressed Enter; both panels close so they don't obscure the
  in-flight turn.

`initialState()` gains `shellOpen: false`.

No new `AgentPaneEvent` entries needed — shell panel visibility is pure UI state with no
downstream saga interest.

---

## 5. History data flow

`AgentFooter` already owns `sentHistory: string[]` as a local Solid signal. The reducer
is the wrong place for this data: `AgentPaneState` is explicitly scoped to state with
**cross-field invariants** (see the file's §11 "no god-reducer" comment). Sent-message
history has no invariants against `turnPhase`, `detailsOpen`, `turnTokens`, etc. — it is
leaf data with no reducer participation needed.

**Correct approach — lightweight module-level registry (same pattern as `token-usage.ts`):**

```typescript
// app/store/shell-history.ts

import { createSignal, type Accessor } from "solid-js";

const MAX_HISTORY = 50;
const registry = new Map<string, { get: Accessor<string[]>; push: (msg: string) => void }>();

export function getShellHistory(blockId: string) {
    if (!registry.has(blockId)) {
        const [get, set] = createSignal<string[]>([]);
        registry.set(blockId, {
            get,
            push: (msg) => set((prev) => {
                const next = [msg, ...prev];
                return next.length > MAX_HISTORY ? next.slice(0, MAX_HISTORY) : next;
            }),
        });
    }
    return registry.get(blockId)!;
}

export function clearShellHistory(blockId: string) {
    registry.delete(blockId);
}
```

- `AgentFooter` calls `getShellHistory(blockId).push(text)` after a successful send.
- `AgentShellHistoryPanel` reads `getShellHistory(blockId).get()` reactively.
- No reducer action, no prop drilling, no `AgentPaneState` pollution.
- `clearShellHistory(blockId)` is called on pane unmount (same lifecycle as existing
  per-pane cleanup).

---

## 6. Files affected

| File | Change |
|---|---|
| `app/view/agent/components/AgentComposerStrip.tsx` | Remove left zone; add model/effort selects + Shell button; replace `<ContextWindowBar>` with inline text |
| `app/view/agent/components/AgentFooter.tsx` | `AgentWorkingRow`: add `currentToolArg` prop + abbreviate render; call `getShellHistory(blockId).push(text)` on send |
| `app/view/agent/components/ContextWindowBar.tsx` | Delete (check for other consumers first) |
| `app/view/agent/components/AgentShellHistoryPanel.tsx` | **New** — shell history panel |
| `app/view/agent/components/AgentControlBar.tsx` | Keep as-is (model/effort duplicated in strip is intentional) |
| `app/store/shell-history.ts` | **New** — module-level registry (see §5) |
| `app/store/agent-pane-state/types.ts` | Add `shellOpen: boolean` to `AgentPaneState`; add `ShellToggle`, `ShellExpand`, `ShellClose` to `AgentPaneCommand` |
| `app/store/agent-pane-state/reducer.ts` | Handle `ShellToggle/Expand/Close`; update `DetailsToggle/Expand` and `TurnStart` for mutual exclusion; add `shellOpen: false` to `initialState()` |
| `app/view/agent/styles/_composer-strip.scss` | Style inline selects, Shell button, active state |
| `app/view/agent/styles/_shell-history.scss` | **New** — shell history panel styles |
| `app/view/agent/agent-view.tsx` | Wire `shellOpen` from pane atom; pass to strip + panel; call `clearShellHistory` on unmount |

---

## 7. User input complementary color

### 7.1 Evidence: current state

**Token (`theme.scss:67`):**
```scss
--user-input-color: #00b4d8;   // cyan, hue ≈ 188°
```

**Usage (`_document-nodes.scss:831–832`):**
```scss
background: color-mix(in srgb, var(--user-input-color) 14%, transparent);
border-left: 3px solid var(--user-input-color);
```

The token is also used at `_document-nodes.scss:926, 974, 979, 984, 1033, 1043, 1046, 1058`
for focus rings, action button borders, and inline-tool highlights within the user message
block. All consumers reference `--user-input-color` — there is no hard-coded hex fallback
outside of `theme.scss`.

**Theme accent (`theme.scss:49`):**
```scss
--accent-color: rgb(65, 159, 224);   // steel blue, hue ≈ 210°
```

### 7.2 Problem

`#00b4d8` (cyan, hue 188°) is **analogous** to `--accent-color` (blue, hue 210°) — both
sit in the blue-cyan family. User messages use the same colour family as links, focus
rings, and active buttons. They blend into the agent-heavy blue of the UI rather than
standing out.

### 7.3 Change

Set `--user-input-color` to the **complementary** of the theme accent: hue 210° + 180° = 30°,
which falls in the warm amber / orange range.

**New value:**
```scss
--user-input-color: #e07832;   // warm amber, hue ≈ 28° — complementary of --accent-color
```

`#e07832` at 14% mix on the dark `rgba(34, 34, 34)` surface reads as a subtle warm tint;
the 3px solid border is legible and immediately distinguishable from any blue-family
element on the same screen. Contrast ratio of `#e07832` on `#222` ≈ 6.1:1 (WCAG AA passes
for normal text, well above 4.5:1 threshold).

The token comment block in `theme.scss:60–66` should be updated to note the complementary
relationship:

```scss
// User-input highlight — complementary to --accent-color (blue hue ~210°) so that
// user messages read as visually distinct from agent content and link-blue UI chrome.
// Hue ~28° (warm amber) is the colour-wheel complement of the accent. Per-theme
// overrides land in `frontend/app/themes/*.scss`.
--user-input-color: #e07832;
```

### 7.4 Files affected

| File | Change |
|---|---|
| `app/theme.scss:67` | `--user-input-color: #00b4d8` → `#e07832`; update comment |
| `app/themes/*.scss` | Any per-theme overrides of `--user-input-color` should be re-evaluated against each theme's accent |

No component SCSS changes needed — all consumers already reference the token.

---

## 8. Out of scope

- Persisting shell history across sessions (future).
- Searching shell history (future).
- Provider switching in the strip — provider is set at agent creation; only model and
  effort change at runtime.
- Making the shell history panel show tool outputs, not just user messages. The panel
  is message-recall only; full shell output is in the conversation document.
