# Spec: Composer Strip Polish + Context Compaction Indicator + Host Type Coloring

**Date:** 2026-06-25  
**Status:** Draft  
**Scope:** `AgentComposerStrip`, `AgentShellHistoryPanel`, `HostPopover`, Claude Code hook integration  
**Naming decision:** The "Shell" button is renamed to **"Log"**

---

## 1. Remove click-to-expand; rename "Shell" → "Log"; rewire to ActivityLogPanel

### Current behavior (two separate expand mechanisms)

**Strip click (the good one):** `AgentComposerStrip.tsx` lines 239–246 — clicking anywhere on
the strip's blank space calls `props.onToggleExpanded()`, which sets `detailsOpenAtom` true.
`agent-view.tsx` line 1294 renders `ActivityLogPanel` + `AgentControlBar` inside
`.agent-composer-details`. `ActivityLogPanel` shows tagged `[tag] text` log entries (launch
flow, subprocess lifecycle, slash command outcomes) — this is the good-looking activity log.

**Shell button (the gibberish one):** The "Shell" button (`agent-composer-strip-shell-btn`,
line 274) toggles `shellOpenAtom`, which renders `AgentShellHistoryPanel`
(`agent-view.tsx` line 1305). That component shows the last 50 sent messages (raw user text,
newest first) as resendable buttons. Raw multiline/markdown messages render as unformatted
text — this is the "gibberish".

### Target behavior

**Rename button:** "Shell" → **"Log"**. Update the label text, CSS class name
(`agent-composer-strip-shell-btn` → `agent-composer-strip-log-btn`), and aria/title strings
accordingly.

**Remove strip click:** Remove the `onClick` handler from the strip `<div>` entirely. No
behavior on clicking the bar's blank space.

**Remove chevron:** Remove the `agent-composer-strip-chevron` button (lines 327–349). Its
`aria-expanded` / `aria-controls` attributes and `onToggleExpanded` prop wiring go with it.

**Rewire Log button → ActivityLogPanel:** The Log button becomes the sole trigger for the
activity log. Change its `onClick` from toggling `shellOpenAtom` to toggling `detailsOpenAtom`
(i.e., call `onToggleExpanded()` instead of `onToggleShell()`). This makes "Log" show the
`ActivityLogPanel` + `AgentControlBar`.

**Retire `AgentShellHistoryPanel` and `shellOpenAtom`:** `AgentShellHistoryPanel` is removed.
`shellOpenAtom` can be deleted from the atoms map. The sent-message recall functionality
already exists via ArrowUp/ArrowDown in `AgentFooter` (`sentHistory` array, lines 289–291
of `AgentFooter.tsx`) — a separate panel for it is not needed.

**Simplify props:** Rename `AgentComposerStrip` props: `shellOpen` → `logOpen`,
`onToggleShell` → `onToggleLog`. Both now reflect `detailsOpenAtom[0]()` at the call site in
`agent-view.tsx`.

**`unreadCount` badge:** Currently on the chevron. Drop it; the activity log panel's own
header already shows entry count.

---

## 2. Log panel — push upward, not overlay

### Current behavior

The `.agent-composer-details` panel (containing `ActivityLogPanel` + `AgentControlBar`)
renders above the composer strip. It should expand in normal document flow so the conversation
scrolls upward to accommodate it.

### Target behavior

- The log panel must **push the conversation content upward** when it opens, not slide over it.
  Verify `.agent-composer-details` and its parent containers use normal document flow
  (not `position: absolute`) so the parent flex column reflows. If any `position: absolute`
  or `overflow: hidden` on a parent clips the panel or prevents reflowing, fix those styles.
- The open/close transition should animate height, giving the impression of the panel
  "pushing" the conversation up rather than appearing on top of content.

---

## 3. Model and effort selects — open upward

### Current behavior

`<select>` elements for model and effort in the controls zone of the strip use native browser
rendering. Browsers open native selects downward by default and the direction is not
CSS-controllable.

### Target behavior

Replace the two native `<select>` elements for **Model** and **Effort** with custom dropdown
components that:

- Open **above** the strip (i.e., `placement: "top-start"` using the existing `computeMenuPosition` /
  Floating UI stack in `frontend/app/util/menu-position.ts`).
- Match the visual style of the existing native selects as closely as possible (compact, same
  height as the strip, no focus ring artifact on the strip background).
- Use `FlyoutMenu` from `frontend/app/element/flyoutmenu.tsx` as the base, passing
  `placement="top-start"` to override the default `"bottom-start"`.

The click handlers, option lists (`MODEL_OPTIONS`, `EFFORT_OPTIONS`), and `updateRuntime`
calls are unchanged — only the rendering mechanism changes.

---

## 4. Host type color coding in HostPopover

### Current behavior

`HostPopover.tsx` line 125–127 shows `hostInfo()!.hostType` as a plain unstyled `<span>`.

### Target behavior

Apply the same color treatment already used by `RuntimeBadge.tsx` / `RuntimeBadge.scss`:

| `hostType` value | Color |
|---|---|
| `"host"` (full system access) | `var(--warning-color, #f59e0b)` — warning yellow |
| `"container"` | `var(--success-color, #22c55e)` — green |
| anything else | default text color |

Implementation options:
1. **Preferred:** Render `<RuntimeBadge runtime={hostInfo()!.hostType} />` directly in the
   HostPopover row, reusing the existing badge component.
2. Alternatively, inline the color logic with a helper: `hostTypeColor(type: string)`.

The color must match the My Agents list exactly so users see the same cue in both places.

---

## 5. Context compaction indicator

### Background

Claude Code exposes three relevant hooks (documented at `code.claude.com/docs/en/hooks`):

| Hook | When | Payload |
|---|---|---|
| `StatusLine` | Every turn | `context_window.remaining_percentage` |
| `PreCompact` | Before compaction starts | `matcher: "auto" \| "manual"` |
| `PostCompact` | After compaction ends | `compact_summary` |

There is no incremental progress signal during compaction; only start/end detection.

### 5a. Context percentage display in the composer strip

The strip already shows `ctxText()` (e.g. `"12k / 200k"`, line 317). Augment this:

- Add a thin **context fill bar** directly underneath the strip (or as a `::after` pseudo-element
  on the strip itself) whose width tracks `context_window.remaining_percentage` received via the
  `StatusLine` hook.
- Color thresholds (matching the existing `ctxBand` bands already in the component, lines 220–228):
  - `> 40%` remaining → green / neutral
  - `20–40%` remaining → yellow (`--warning-color`)
  - `< 20%` remaining → red (`--error-color`)
- The bar is purely informational; it does not need a click target.
- The `StatusLine` hook fires for every agent turn. Wire it via the existing agent pane RPC event
  system (or a new `context_window` field on the agent state atom), not a polling loop.

### 5b. Compaction spinner / badge

- When `PreCompact` fires (auto trigger): show a "Compacting…" badge in the agent pane tab header
  (or the strip itself) using the existing `agent-composer-strip-stats` span or a new dedicated
  element. A spinner glyph (`⟳` or CSS animation) is sufficient — no percentage is possible.
- When `PostCompact` fires: dismiss the badge.
- If the agent tab is not in focus when compaction starts, the badge must also be visible on the
  tab chip so the user can notice it from other tabs.

### 5c. Hook wiring (Claude Code SDK / settings.json)

AgentMux agent panes already spawn Claude Code sessions. The hooks should be registered either:

1. **Via SDK** (if panes use the TypeScript/Python Agent SDK): pass `hooks` option to
   `ClaudeAgentOptions` with `PreCompact` and `PostCompact` matchers that post a message back
   to the pane via muxbus or IPC.
2. **Via `settings.json`** (if sessions run as CLI subprocesses): inject shell hook commands
   that write to a named pipe or send an IPC event to `agentmux-srv`.

The `StatusLine` hook fires synchronously on every turn and should deliver
`context_window.remaining_percentage` to the frontend via the same channel as agent streaming
events (the existing `AgentOutput` / `BlockData` stream), adding a new field rather than a
separate channel.

---

## Files affected

| File | Change |
|---|---|
| `frontend/app/view/agent/components/AgentComposerStrip.tsx` | Rename Shell→Log, remove onClick expand, remove chevron, rewire Log button, add context bar, add compaction badge |
| `frontend/app/view/agent/styles/_composer-strip.scss` | Rename shell CSS classes to log, style context fill bar, compaction badge |
| `frontend/app/view/agent/styles/_shell-history.scss` | Delete (panel retired) |
| `frontend/app/view/agent/components/AgentShellHistoryPanel.tsx` | Delete (panel retired) |
| `frontend/app/store/shell-history.ts` | Delete (no longer needed) |
| `agent-view.tsx` | Rewire Log button → detailsOpenAtom, remove shellOpenAtom wiring, verify push-up layout on `.agent-composer-details` |
| `frontend/app/view/agent/components/RuntimeBadge.tsx` | (no change) |
| `frontend/app/statusbar/HostPopover.tsx` | Replace plain hostType span with RuntimeBadge |
| Agent pane state / RPC layer | Add `context_window.remaining_percentage` field to streaming output |
| Claude Code hook config (SDK or settings.json) | Register PreCompact / PostCompact / StatusLine hooks |

---

## Out of scope

- The expansion panel's content (details panel) — it is removed wholesale; no migration of its
  contents is needed unless specific items need a new home.
- The `FlyoutMenu` default `placement` — do **not** change the global default. Only the two
  selects in `AgentComposerStrip` switch to `"top-start"`.
- Compaction progress percentage — not available from any API; not implemented.
