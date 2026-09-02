# Widget & Pane Label Casing — Title-Case the User-Visible Names

**Date:** 2026-05-27
**Status:** Draft
**Scope:** UI text only — no runtime/IPC behavior changes

---

## Problem

Widget-bar labels and a subset of pane title fallbacks render lowercase (`agent`, `browser`, `editor`, ...). The internal view identifiers are intentionally lowercase (they are stable keys used in `meta.view`, RPC payloads, settings overrides, blockdef JSON, and many tests), but the user-facing label has been silently inheriting the identifier instead of carrying its own properly-cased display string.

The widget bar in the title bar is the single most prominent piece of chrome in the app, and every entry there is currently lowercase. The fix is one config file plus one fallback table.

---

## Goal

Every user-visible widget/pane label is Title Case (`Agent`, `Browser`, `Editor`, `Terminal`, `Sysinfo`, `Drone`, `Help`, `Swarm`, `Warden`). Internal view IDs stay lowercase — only the display strings change.

---

## In-Scope Surfaces

| # | Surface | File | Current | Target |
|---|---------|------|---------|--------|
| 1 | Widget bar — pinned & "More" dropdown | `agentmux-srv/src/config/widgets.json` | `"label": "agent"`, ... | `"label": "Agent"`, ... |
| 2 | Pane title fallback (when `viewModel.viewName` is empty AND `generateAutoTitle` is not used) | `frontend/app/block/blockutil.tsx::blockViewToName()` | falls through and returns raw view id | explicit Title-Case for all 9 view ids |
| 3 | Repo docs — widget label table | `agentmux/CLAUDE.md` § "Widgets" | `agent`, `browser`, ... in Label column | `Agent`, `Browser`, ... |
| 4 | User-facing docs — references to the bar labels in prose | `agentmux-docs/src/content/docs/{first-agent,quickstart,pane-types}.md` | "click the **agent** icon" | "click the **Agent** icon" |
| 5 | Repo README — feature-list prose | `agentmux/README.md:31` | "alongside terminals, editor, browser, and system metrics" | "alongside **Terminal**, **Editor**, **Browser**, and **Sysinfo** panes" (or leave as common-noun list — see §Open Questions) |

Surface (1) is the dominant fix — every widget-bar button reads `widget.label` directly (`frontend/app/window/action-widgets.tsx:123`, `:197`).

Surface (2) is the safety net: a few code paths (`block.tsx:113`, `blockframe.tsx:260,271`) call `blockViewToName()` and currently get the raw lowercase id back for `agent`, `browser`, `editor`, `sysinfo`, `warden`.

Surfaces (3) and (4) keep the docs consistent with what the user sees in the chrome. The user-doc table in `pane-types.md` already uses Title Case in its **Pane** column — only the prose that says "click the agent icon" needs updating.

Surface (5) is a single optional line. The README's widget table at lines 81–89 is already correct.

---

## Out of Scope (Do NOT Change)

- `meta.view` values in any blockdef / settings JSON / RPC payload (`"view": "agent"` stays lowercase — it's a stable key).
- Internal `viewType = "agent"` strings on view models.
- `widgets.json` keys (`defwidget@agent`) and `blockdef.meta.view` fields.
- All test fixtures using lowercase view ids (`viewType="agent"` in `BlockErrorBoundary.test.tsx` etc.) — these probe the internal id, not the label.
- Drone canvas block labels (`frontend/app/view/drone/block-registry.ts`) — already Title-Case.
- `autotitle.ts::generateAutoTitle` — already Title-Cases via `generateDefaultTitle` (`charAt(0).toUpperCase()`), and explicit cases (`"Help"`, `"System Info"`) are already correct.
- Common-noun usage in docs: "the **agent pane** has a cog icon", "the **browser pane**'s lifecycle", "open a **terminal**" — these are concept-noun references, not UI-label references, and should stay lowercase.
- Architectural/internal docs that describe widgets as components (e.g. `internals/agent-pane-virtualization.md`, `internals/warden.md`) — they document the implementation, not the visible label.

---

## Changes

### 1. `agentmux-srv/src/config/widgets.json`

Replace the 9 `"label"` values:

| Key | Before | After |
|---|---|---|
| `defwidget@agent` | `"agent"` | `"Agent"` |
| `defwidget@browser` | `"browser"` | `"Browser"` |
| `defwidget@editor` | `"editor"` | `"Editor"` |
| `defwidget@terminal` | `"terminal"` | `"Terminal"` |
| `defwidget@sysinfo` | `"sysinfo"` | `"Sysinfo"` |
| `defwidget@drone` | `"drone"` | `"Drone"` |
| `defwidget@help` | `"help"` | `"Help"` |
| `defwidget@swarm` | `"swarm"` | `"Swarm"` |
| `defwidget@warden` | `"warden"` | `"Warden"` |

No other keys touched. Display order, icons, colors, descriptions, `blockdef.meta.view` all unchanged.

### 2. `frontend/app/block/blockutil.tsx`

Extend `blockViewToName(view)` so every known view returns a Title-Case label instead of falling through to the raw id:

```ts
const VIEW_LABELS: Record<string, string> = {
    agent: "Agent",
    "agent-def": "Agent Definition",
    browser: "Browser",
    chat: "Chat",
    drone: "Drone",
    editor: "Editor",
    help: "Help",
    identity: "Identity",
    memory: "Memory",
    subagent: "Subagent",
    swarm: "Swarm",
    sysinfo: "Sysinfo",
    term: "Terminal",
    warden: "Warden",
};

export function blockViewToName(view: string): string {
    if (util.isBlank(view)) return "(No View)";
    return VIEW_LABELS[view] ?? view;
}
```

The five existing explicit cases (`term`, `help`, `subagent`, `swarm`, `drone`) keep their current text; the new entries match the widget-bar labels in §1 plus the three non-widget panes documented in `CLAUDE.md` ("Not widgets" table).

### 3. `agentmux/CLAUDE.md` — widget table (§ "Widgets")

Update the **Label** column of the widget table from lowercase to Title Case:

| Widget Key | View | Label | Tier |
|---|---|---|---|
| `defwidget@agent` | `agent` | **Agent** | Pinned |
| `defwidget@browser` | `browser` | **Browser** | Pinned |
| `defwidget@terminal` | `term` | **Terminal** | Pinned |
| `defwidget@sysinfo` | `sysinfo` | **Sysinfo** | Pinned |
| `defwidget@editor` | `editor` | **Editor** | Pinned |
| `defwidget@drone` | `drone` | **Drone** | Pinned |
| `defwidget@help` | `help` | **Help** | Pinned |
| `defwidget@swarm` | `swarm` | **Swarm** | Pinned |
| `defwidget@warden` | `warden` | **Warden** | Pinned |

No other rows or columns change.

### 4. `agentmux-docs` — prose references to bar labels

Update the following prose mentions so that when a doc instructs the reader to click a button by its bar label, the label is shown in Title Case. Bold the label to mirror how the doc already styles UI-named affordances ("**Memory** tab").

| File | Line(s) | Change |
|---|---|---|
| `src/content/docs/first-agent.md` | 29 | "click the **agent** icon in the top bar" → "click the **Agent** icon in the top bar" |
| `src/content/docs/quickstart.md` | 31 | same as above |
| `src/content/docs/pane-types.md` | 52 | "click the **terminal** icon in the top bar" → "click the **Terminal** icon in the top bar" |
| `src/content/docs/settings.md` | 121 | `Show help widget in top bar` → `Show Help widget in top bar` (description text only; JSON key `widget:showhelp` stays unchanged) |

The "Pane" column in `pane-types.md` is **already Title Case** — no change there.

A broader sweep is not necessary: most other lowercase mentions ("the agent pane", "the browser pane", "open a terminal") are common-noun usage referring to the pane concept, not the bar label, and are correct as-is.

### 5. `agentmux/README.md` — feature-list prose

The widget table at `README.md:81-89` **already uses Title Case** in the **Pane** column — no change required there.

The only candidate is **line 31**:

> "...alongside terminals, editor, browser, and system metrics."

This is a feature list, not an instruction to click a labeled button, so the lowercase reads naturally. Recommended change for consistency with the new bar labels:

> "...alongside **Terminal**, **Editor**, **Browser**, and **Sysinfo** panes."

Optional — if the implementer prefers the looser prose reading, leave line 31 unchanged. This is the only mention; no other README prose references widget names by label.

---

## Open Questions

1. ~~**`sysinfo` → `Sysinfo` vs. `System Info`?**~~ **Resolved:** `Sysinfo` everywhere (widget bar label and `blockViewToName`). The auto-generated pane title in `autotitle.ts:175` (`"System Info"`) stays unchanged — it's the title above an opened Sysinfo pane and reads naturally as two words there; the bar label is the single-word `Sysinfo`. Small bar/title divergence accepted.
2. **`agent-def` casing.** Not a widget-bar entry, but reachable as a pane. Spec proposes `"Agent Definition"`; `generateDefaultTitle` currently produces `"Agent-def"`. Lowest-risk option: also `"Agent Definition"` for both — or leave the fallback alone since this is rarely reached.
3. **README line 31 — change or leave?** See §5. Recommended change; non-blocking.

---

## Risk

Low. No behavioral changes — only string content in a config file and a lookup table. Internal identifiers, RPC, settings overrides, and tests are untouched. User-installed `settings.json` overrides for widgets continue to take precedence over `widgets.json`, so this doesn't disturb customized installations.

---

## Acceptance

- [ ] Widget bar (top of every AgentMux window) shows `Agent · Browser · Editor · Terminal · Sysinfo · Drone · Help · Swarm · Warden` (icon-only mode still triggers correctly when narrow).
- [ ] The "More" dropdown lists labels in Title Case.
- [ ] Pinning/unpinning a widget still works; tooltip uses the new label.
- [ ] Opening a fresh pane whose `viewModel.viewName` is empty shows the Title-Case fallback (not the raw view id).
- [ ] All existing tests pass without modification. No test depends on lowercase labels.

---

## Implementation Notes

- **Two PRs**, one per repo:
  1. `agentmuxai/agentmux` — surfaces (1), (2), (3), (5). ~10 lines of JSON, ~20 lines of TypeScript, 9 cells in `CLAUDE.md`, ≤1 line in `README.md`. Changeset: `task changeset -- patch "fix(ui): title-case widget bar and pane labels"`. No version bump in the PR (per `RFC #857 Phase 2` — release PRs own bumps).
  2. `agentmuxai/agentmux-docs` — surface (4). ~4 lines across 4 markdown files. Standard PR, no version concerns.
- Land the agentmux PR first so the docs PR can link to a released bar that already matches the new prose. Practically, both can land in either order — text on a docs page never "breaks" if the bar lags by a build.
