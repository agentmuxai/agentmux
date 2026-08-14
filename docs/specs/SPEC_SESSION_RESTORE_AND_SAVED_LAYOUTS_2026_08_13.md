# Spec: Restore-on-relaunch + named, reloadable "Layouts"

**Date:** 2026-08-13
**Type:** Design spec (two related, independently-shippable features)
**Status:** Proposed — not yet implemented. Naming decided (§3: "Layout"); hamburger-menu placement decided (§5.1).
**Trigger:** User request — AgentMux used to reopen with the same panes/tabs/Armory layout as when it was last closed; that behavior is gone (root-caused in `docs/retro/retro-pane-layout-restore-was-a-leak-not-a-feature-2026-08-13.md` — it was never a deliberate feature, it was a leaked-window-row bug that has since been correctly fixed). Separately, the user wants a "snapshots" concept: save a named pane arrangement on demand and reload it later, similar to a feature in Wave Terminal (the upstream project AgentMux forked from).
**Builds on:** `docs/specs/SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` (crash-only reproject — deliberately steady-state-never-rebuilds, so it does not cover either feature below, but its topology plumbing is directly reusable), `docs/specs/SPEC_PILLAR1_STEP2_WINDOW_TOPOLOGY_PERSISTENCE_2026_07_06.md` (per-window opacity / floating-pane placement write-through — same pattern this spec extends).

---

## 0. TL;DR

Two features, sharing infrastructure but with different triggers and UX:

1. **Restore-on-relaunch** — after a normal quit, the next launch reopens the same tabs/panes/split layout instead of always reseeding the default 3-pane workspace. Opt-out, not opt-in (matches what users remember and what most prior-art tools default to). No user-facing save/load action — it just always reflects "what was open last."
2. **Layouts** *(name decided, see §3)* — an explicit, named, user-saved pane arrangement that can be reloaded on demand, independent of what's currently open, accessed from a new **Layouts** entry in the hamburger menu (§5.1). This is new: nothing like it exists in AgentMux today, and — despite surface similarity — Wave Terminal's own "Workspaces" feature does not actually provide it either (see §2).

Both build on the same underlying capability: serializing "what panes/tabs/agents are arranged how" into a durable record, and being able to re-materialize windows/tabs/blocks from that record. Restore-on-relaunch is that capability applied automatically to "the last live state." Layouts is that capability applied to an explicit, named, point-in-time save.

---

## 1. Why Pillar 1's reproject doesn't already cover this

It's tempting to assume the crash-reproject work (Pillar 1) is 90% of this for free. It isn't, by design:

- `SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` §3 states the goal outright: *"Flicker is a crash-path event only — steady state never rebuilds."* Reproject fires only when `Client.windowids` still holds rows a graceful close would have pruned — i.e., only because a crash (not `CloseWindow`) left them behind.
- Today, a graceful close **deliberately cascades a full delete** of the workspace/tabs/blocks (`agentmux-srv/src/backend/wcore/window.rs:139-161`, `agentmux-srv/src/server/service/window_close.rs:24-186`) — added 2026-04-04 to stop orphaned shell processes (commit `e3a6f85c2`, PR #299), and correctly wired to also fire for the main window as of 2026-07-16 (commit `4cbf856b7`, PR #2186). See the retro doc for the full timeline.
- So today, a normal quit always leaves `Client.windowids` empty, and the next launch always seeds a fresh default workspace (`agentmux-srv/src/server/service/window_create.rs:107-211`, `default_three_pane_tree`). There is nothing for reproject to find.

**Consequence for this spec:** Feature 1 cannot be "just let reproject also run on clean quit" — reproject's whole design assumes destroy-on-crash-only and rebuilds from rows a clean close is specifically supposed to remove. Feature 1 needs its own trigger and its own persisted record, decoupled from the close-time destroy cascade (§4). It can and should reuse reproject's *read/rebuild* machinery (window creation, layout-tree materialization) — just not its *write* trigger (crash detection) or its assumption that the destroy cascade always ran.

---

## 2. Prior art (condensed — full detail gathered during research, kept here only as decision-relevant)

| Tool | State captured | Save UX | Load UX | Staleness handling |
|---|---|---|---|---|
| **Wave Terminal Workspaces** (upstream, `docs.waveterm.dev/workspaces`) | tabs, block layout, terminal/AI history | switcher → explicit "save" turns an ephemeral workspace durable; **continuously auto-persists** after that, not a point-in-time save | switch to it from the workspace switcher (replaces current window content) | N/A — it's a live desktop, not a restore point. **No save-a-named-layout-and-reload-later capability exists** — this is an open, unaddressed upstream feature request ([waveterm#2072](https://github.com/wavetermdev/waveterm/issues/2072)). |
| **tmux-resurrect / tmux-continuum** | pane tree, per-pane cwd + running command, focus | manual keybind or auto every N min | manual keybind or auto-restore on server start; idempotent | re-runs last command; silently no-ops if already present |
| **tmuxinator / tmuxp** | declarative YAML: session/window/pane tree, `root`, per-pane command, layout preset | hand-authored config file (version-controllable) | `tmuxinator start <name>` — opens as new session | none built-in; commands fail at load time if paths/binaries are gone |
| **iTerm2 Window Arrangements** | window/tab/split geometry + profile (not process/scrollback) | explicit "Save Window Arrangement," named | "Restore" (in place) vs. "Restore as Tabs" (new); optional auto-restore-on-launch | none — template of profiles/positions, nothing to go stale |
| **VS Code `.code-workspace`** | open folders, editor tabs, terminal sessions, debug configs | explicit "Save Workspace As" → JSON | open the file, replaces current window state | none formalized |
| **Zellij** | panes/tabs + command per pane | automatic continuous serialization to KDL | `zellij --layout file.kdl` | commands shown behind a "press ENTER to run" gate, specifically to avoid silently re-running possibly-stale/dangerous commands |

**Key finding:** the feature the user wants — explicit, named, multiple, reloadable-on-demand layout presets — is closest to iTerm2's Window Arrangements and tmuxinator's config files, **not** to Wave Terminal's Workspaces (which the user's "wavelab" almost certainly refers to; that feature solves a different problem — organizing many *live*, continuously-persisted tabs into named buckets — and explicitly doesn't do point-in-time save/reload either, per the still-open upstream issue above).

---

## 3. Naming — decided: "Layout"

The user asked for a better name than "snapshot," and has settled on **"Layout"** (menu item: **Layouts**; the save action: **Save…**). Recorded here for traceability, including the names considered and ruled out along the way:

- **"Workspace"** is already a live internal object (`Window` → `Workspace` → `Tab` → `Block`, srv-side) with its own delete-cascade semantics (§1). Reusing it for a saved/dormant record would be actively confusing — "delete this workspace" already means something destructive and different. Ruled out.
- **"Snapshot"** collides with Pillar 1's own vocabulary — `Command::GetSnapshot`/`Event::Snapshot` already exists as the launcher's live in-memory window-topology wire protocol (`SPEC_PILLAR1_STEP4_CRASH_REPROJECT_2026_07_07.md` §6, item 4). Ruled out (this was the user's own starting term, and the reason a rename was wanted in the first place).
- **"Preset"** was deliberately retired from this codebase (renamed to "Bundle" for agent config collections, PR #1918, `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`) specifically to stop overloading the word. Ruled out.
- **"Loadout"** and **"Stash"** were both weighed for the *per-agent* config modal (`docs/reports/REPORT_ARMORY_STASH_NAMING_2026_07_27.md`); "Stash" won and shipped (`AgentStashModal`). "Loadout" reads as "what one agent carries" (an inventory), not "how panes are arranged in space." Ruled out.
- **"Formation"** — this spec's first-draft recommendation (unclaimed, fit the Armory/Warden/Drone/Swarm/Stash register). Superseded by the user's own choice below.

**"Layout" is unclaimed as a *user-facing* term** and deliberately reuses the existing *internal* vocabulary (`LayoutState`, the per-tab layout tree, `agentmux-srv/src/backend/storage` layout persistence already discussed in §1/§4) rather than fighting it — a saved Layout is, literally, a saved layout tree plus the block launch-specs needed to repopulate it. This mirrors JetBrains' "Window Layouts" naming precedent (§2) and needs no new metaphor for users to learn. No collision found against any shipped user-facing string (`grep`-checked at the time of drafting).

The rest of this spec uses **Layout** (singular saved item) / **Layouts** (the menu section and the feature as a whole) throughout.

---

## 4. Feature 1 — Restore-on-relaunch

### Goal
After a normal quit, relaunching AgentMux reopens the same windows/tabs/pane layout that were open at quit time — without reintroducing the row-leak semantics the 2026-07-16 fix correctly closed.

### Design
- **Decouple "durable last-known-good topology" from the close-time destroy cascade.** Today, close destroys the workspace as its *only* durable record. Add a second, independent record: on a **graceful** window/app close (not crash — crash already leaves rows behind for reproject, unrelated path), write a compact topology snapshot (window set, per-window tab/layout tree, block view+meta — the same shape Pillar 1 Step 2/3 already persist for opacity/placement/kind) to a new durable slot (e.g. `Client.last_session_topology` or a dedicated small table) **before** the existing destroy cascade runs, or as part of the same transaction.
- **On next cold launch**, `initHostWave` (`frontend/app-init.ts:334`) checks this new slot (not `Client.windowids`, which is correctly still empty) and — if present and restore-on-relaunch is enabled — replays it through the same window/tab/block creation path reproject already uses, instead of falling through to `default_three_pane_tree`.
- **This is opt-out, not opt-in** (a setting under Settings → Window & Panes, default ON) — matches what the user remembers, and matches the default behavior of iTerm2's auto-restore and Windows Terminal's `persistedWindowLayout`.
- **Scope of what's restored:** window set, tab structure, split/layout tree, per-block view type + the block's own launch-relevant meta (e.g., which agent/CLI a block was running, its cwd) — the same "logical topology" Pillar 1 already classifies as durable-worthy (`SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` §2.A). **Not** restored: live process state, terminal scrollback, in-flight agent conversation state (none of that survives a quit regardless of this feature — consistent with every prior-art tool surveyed in §2, none of which resurrect live process memory either).
- **Agent panes specifically:** on restore, re-launch the same agent identity/CLI/cwd a block was running, the same way opening a fresh agent pane does today — do not attempt to resume an in-progress conversation transcript inline (that's a separate, already-solved concern via transcript history, not layout restore's job).

### Non-goals
- Restoring exact terminal scrollback or process memory.
- Covering crash recovery — that's Pillar 1's job, already scoped, unaffected by this feature.

---

## 5. Feature 2 — Layouts (named, on-demand saved layouts)

### Goal
Let a user explicitly save the current pane/tab/window arrangement under a name, and reload it later — independent of, and in addition to, whatever restore-on-relaunch is doing automatically.

### 5.1 UI placement — hamburger menu (user-specified)

Per the user's direction, **Layouts** is a new top-level entry in the hamburger (☰) menu, positioned directly under **Opacity** and above the divider that currently precedes **Settings**.

Concretely, in `frontend/app/window/hamburger-menu.tsx`, the `menuItems` array today reads (lines 100-110):

```ts
{ label: "Opacity", icon: "circle-half-stroke", subItems: opacitySubItems },
{ label: "", divider: true },
{ label: "Settings", icon: "cog", onClick: () => fireAndForget(() => openOrFocusPaneByView("settings")) },
```

New item inserted between `Opacity` and the divider (so the existing Settings divider grouping is undisturbed):

```ts
{ label: "Opacity", icon: "circle-half-stroke", subItems: opacitySubItems },
{ label: "Layouts", icon: "grip", subItems: layoutsSubItems() },
{ label: "", divider: true },
{ label: "Settings", ... },
```

(`grip` is a placeholder icon suggestion, checked against current usage: `table-cells` — the obvious first choice for a grid/pane-arrangement glyph — is already claimed by Settings' own "Window & Panes" rail entry (`frontend/app/view/settings/settings-view.tsx:50`), too close a fit to reuse without confusion one menu over. `grip` (drag-handle/rearrange glyph) is unclaimed as of this spec — re-verify at implementation time, since the icon set shifts between now and then.)

**Submenu contents**, mirroring the existing `themeSubItems`/`opacitySubItems` reactive-`createMemo` pattern (same file, lines 44-52 and 58-78):

1. **"Save…"** — always the first entry. Opens the naming prompt (§5.2).
2. A divider, then one entry per saved Layout, alphabetical or most-recently-saved-first (confirm preference at implementation time). Clicking a saved Layout's name **loads** it (§5.3) — no separate "Load" submenu; the name itself is the load action, matching the Theme submenu's own pattern (clicking a theme name applies it directly).
3. The submenu list needs a live data source — a new reactive atom/store (e.g. `layoutsAtom`, populated from a `ListLayouts`-style RPC) feeding `layoutsSubItems()`, the same shape `settingsAtom()` already feeds `themeSubItems`/`opacitySubItems` in this file.

### 5.2 Save UX — naming prompt

The user's description ("Save… shows a text box input for which you can set a name") needs one implementation decision `MenuItem` doesn't support today: `types/custom.d.ts`'s `MenuItem` type (label/icon/subItems/onClick/divider/checked/shortcut) has **no inline-input variant** — a flyout menu item is a click target, not a form field. Two ways to deliver the described UX:

- **(Recommended) A small anchored popover/modal, opened by "Save…"'s `onClick`, positioned at/near the menu.** Closes the flyout, shows a single text field (name) + Save/Cancel, prefilled empty (or, if invoked while a Layout is "active," prefilled with its current name for easy overwrite — see below). This reuses existing modal infrastructure (`openModal`, already imported in `hamburger-menu.tsx`) rather than extending `MenuItem`'s type, and is a one-file addition, not a `FlyoutMenu` capability change. Visually reads almost identically to "a text box appearing where you clicked."
- **(Alternative, higher-lift) Extend `MenuItem`/`FlyoutMenu` with a new inline-input item kind.** Delivers a more literal "text box inside the menu itself," but touches shared menu infrastructure used by every other hamburger/context menu in the app — a larger, riskier change for a UX difference that's hard to distinguish from the popover option in practice. Not recommended for v1; revisit only if the popover reads wrong once built.

This spec recommends the popover approach; flag before implementation if the more literal inline-in-menu behavior is a hard requirement.

**Name collision handling:** if the entered name matches an existing saved Layout, ask to confirm overwrite (don't silently create a second "My Layout" — and don't silently fail either).

Store as a self-contained, inspectable record (e.g., JSON under a new `db_layouts` table or similar, one row per saved Layout, per-user/per-instance not synced) — diffable/exportable in spirit, even if not literally file-based, per the "explicit artifact over opaque live state" best practice from the research pass (§2). Not automatic, not continuous — a deliberate point-in-time artifact, distinct from Feature 1's always-on background record and distinct from Wave Terminal's continuously-auto-persisting Workspaces.

### 5.3 Load UX

- Clicking a saved Layout's name in the submenu loads it. **Never destructive by default** — opens as new window(s)/tab(s) alongside whatever is currently open, mirroring iTerm2's "Restore as Tabs" and VS Code's file-open model. A "replace current layout instead" option can live behind a modifier-click or a secondary action later — not the default, since a silent replace risks losing unsaved live work (agent panes with in-progress conversations).
- **Gate agent re-launch behind a visible confirmation**, per Zellij's "press ENTER to run" precedent — show what will actually happen (which agents will be (re)launched, in which directories) before executing, since silently re-spawning AI agents against a possibly-stale cwd/branch has real cost (token spend, unintended actions) beyond a plain shell command.

### Staleness handling
- At load time, validate each block's cwd/repo prerequisites before acting (per §2's Zellij/best-practice finding — don't fail the whole Layout or silently no-op like tmux-resurrect; surface **per-pane** status: OK / directory missing / would relaunch agent). Let the user proceed pane-by-pane rather than all-or-nothing.

### Non-goals (initial version)
- Rename/delete of a saved Layout from the menu itself — v1 ships create (Save…) + overwrite-by-same-name + load only. A right-click context menu on a saved Layout entry (rename/delete) is a natural fast-follow, not required to close the user's request.
- Cross-machine/cross-user sharing or export/import of Layouts (worth a follow-up once the core save/load loop is validated).
- Scheduling or auto-applying a Layout (e.g., "load this Layout every morning") — purely user-triggered for v1.

---

## 6. Shared infrastructure between the two features

Both features need the same core capability: (a) serialize a window/tab/layout/block-launch-spec tree to a durable record, (b) materialize windows/tabs/blocks from such a record. Recommend building this once as a shared internal module, consumed two ways:

- Feature 1 writes/reads it automatically, keyed to "the last session," replacing itself on every graceful close.
- Feature 2 writes/reads it explicitly, keyed to a user-chosen name, any number of saved records.

This also directly reuses the block-launch-spec vs. live-state distinction Pillar 1 Step 2/3 already established for opacity and floating-pane placement (write-through the logical facts, never the native handles) — same discipline, applied to the coarser "whole topology" grain instead of individual per-window facts.

---

## 7. Phased plan

1. **Shared serialize/materialize module** — window/tab/layout-tree/block-launch-spec ⇄ durable record, built and unit-tested independent of either feature's trigger.
2. **Feature 1 (restore-on-relaunch)** — wire the graceful-close write and cold-launch read, behind a default-ON setting. Ship first: smaller surface, no new UI beyond one setting toggle, directly closes the regression the user reported.
3. **Feature 2 (Layouts)** — hamburger-menu UI (§5.1), Save popover (§5.2), load-by-click (§5.3), staleness validation, confirmation-gated agent relaunch.
4. **Follow-up (not v1):** rename/delete from the menu, export/import, cross-machine sharing, scheduled/triggered loads.

---

## 8. Risks / honest caveats

- **Feature 1's write must not reintroduce the row-leak class of bug** (the whole reason today's destroy-on-close cascade exists — orphaned shell processes). The new durable-topology write is metadata only (tree shape + launch specs), not a reason to skip or delay killing shell processes on close; the two must stay independent so a bug in one can't resurrect the other.
- **Agent relaunch on Layout load has real cost** (token spend, side effects) if done silently or by accident — the confirmation gate in §5.3 is not optional polish, it's the safety mechanism.
- **The Save… popover (§5.2) is a new small UI surface, not just a menu-copy change** — scope it as real (if small) frontend work, not a one-line addition.
- **Layout staleness UX (§5) is more product design than engineering** — worth a quick look at real usage before over-building the per-pane status UI; start minimal (block-level "will relaunch" list) and iterate.
- **The submenu's saved-Layout list needs a live reactive data source** (§5.1, item 3) — a new atom/store and a small `ListLayouts`/`SaveLayout`/`DeleteLayout`-shaped RPC surface, not purely a frontend change.

---

## 9. Sources

- `docs/retro/retro-pane-layout-restore-was-a-leak-not-a-feature-2026-08-13.md` (root cause of the reported regression)
- `docs/retro/retro-last-window-close-quit-race-2026-07-16.md`
- `docs/specs/SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md`
- `docs/specs/SPEC_PILLAR1_STEP2_WINDOW_TOPOLOGY_PERSISTENCE_2026_07_06.md`
- `docs/specs/SPEC_PILLAR1_STEP4_CRASH_REPROJECT_2026_07_07.md`
- `docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`, `docs/reports/REPORT_ARMORY_STASH_NAMING_2026_07_27.md` (naming precedent, §3)
- `frontend/app/window/hamburger-menu.tsx` (existing menu structure, Theme/Opacity submenu pattern reused in §5.1), `frontend/types/custom.d.ts:446-460` (`MenuItem` type — no inline-input variant, informs §5.2)
- Wave Terminal Workspaces docs: https://docs.waveterm.dev/workspaces
- Wave Terminal open feature request (save/restore tab layout): https://github.com/wavetermdev/waveterm/issues/2072
- tmux-resurrect: https://github.com/tmux-plugins/tmux-resurrect
- tmuxp docs: https://tmuxp.git-pull.com/quickstart/
- iTerm2 Window Arrangements: https://iterm2.com/documentation-preferences-arrangements.html
- Zellij Session Resurrection: https://zellij.dev/documentation/session-resurrection.html
