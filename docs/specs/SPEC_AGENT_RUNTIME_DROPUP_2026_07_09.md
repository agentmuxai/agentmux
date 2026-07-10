# SPEC: Consolidate Mode / Model / Effort into a single Runtime dropup

**Date:** 2026-07-09
**Status:** Approved — open questions resolved, ready to implement
**Author:** Agent2
**Trigger:** User request — *"switch the Bypass/Model/effort dropups with a single panel dropup ... think of good names for the button. The panel includes all the options and models and effort, listed out efficiently."*
**Revision note:** This spec was originally drafted against a stale checkout that predated commit `9f86d917` (#1922, *"Mode/Model/Effort drop-ups (replace native `<select>`)"*) and its dependents (#1912, #1920, #1926). That work already replaced the old native `<select>`s with three separate `FlyoutMenu`-based drop-up pills directly in `AgentComposerStrip`. This revision retargets the same consolidation goal at that current architecture. §2 (naming) and §9 (resolved open questions) are unaffected by the retarget and carry over unchanged; everything else is rewritten.

---

## 1. Why

Today `AgentComposerStrip` renders Mode, Model, and Effort as **three separate drop-up pills**, each an independent `FlyoutMenu` (`placement="top-start"`) — a `StripSelect` helper wraps each one (`AgentComposerStrip.tsx`, prior to this change). This already replaced the older `<select>`-based `AgentControlBar` UI (per `SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02`), so the "no more native chrome, opens upward" goal is done — but the user's actual ask, one consolidated control instead of three, is still open: three separate trigger pills sit side by side, each independently clickable, each with its own popup.

This spec replaces those three pills with **one button that opens one floating panel** containing all three axes, grouped under labeled sections and keyboard-navigable — reusing the exact positioning primitives `FlyoutMenu` itself uses (so it looks and behaves like a native part of the app, not a bolted-on widget), but as a bespoke component, because `FlyoutMenu` only renders a flat `MenuItem[]` list and has no concept of grouped sections with headers (confirmed by reading `frontend/app/element/flyoutmenu.tsx` — no section/group field on `MenuItem`, no content-injection point besides the trigger).

**Prior art check:** `SPEC_COMPOSER_STRIP_RESPONSIVE_ARCHITECTURE_2026_07_02.md` §3 sketches a "Tier TINY (<240px)" fallback where the three pills collapse into one combined chip (`⚙ Byp·Son·XHi ▴`) purely as a narrow-viewport degradation — the three-pill layout is otherwise kept at every wider tier in that spec. This spec supersedes that narrow-tier idea: the single consolidated control becomes the *only* form, at every width, not just a <240px fallback. Nothing else in either composer-strip spec proposes a universal single-button design.

---

## 2. Naming the button

The button's label doubles as a live summary (mode · model · effort), so the *name* only matters as a fallback/aria-label and as the word used in docs, `/help`, and hover tooltips. Four candidates, ranked:

| Name | Rationale | Downside |
|---|---|---|
| **`Runtime`** (recommended) | Zero new vocabulary — the codebase already calls this exact bundle `AgentRuntimeConfig`, stores it under meta key `agent:runtime`, builds it via `buildRuntimeArgs()`, and the existing `/runtime` command *already* prints it back to the user as `"runtime config — permission: x · model: y · effort: z"`. Adopting "Runtime" as the button name means the mental model, the code, and the slash command all say the same word. | Slightly abstract to a first-time user vs. more concrete names. |
| `Loadout` | Gaming-flavored, memorable, evokes "how you're configured for this run." Short. | Introduces a brand-new term with no precedent anywhere in the codebase or docs. |
| `Controls` | Was the exact word `AgentControlBar` used historically (`▸ Controls` chevron) — no longer applicable since that chevron/label is already gone from the current UI. | Generic — doesn't hint at *what* it controls; easy to confuse with other UI chrome. |
| `Dial` | Short, playful, "turning a dial" fits adjusting mode/model/effort. Terminal-aesthetic-friendly. | No existing precedent; slightly cute for a tool used by professionals under time pressure. |

**Recommendation: `Runtime`.** It's the only option that requires zero new terminology anywhere else in the product (help text, `/runtime`, code identifiers all already agree).

The button itself does **not** render the word "Runtime" in its default state — it renders the live summary instead (e.g. `Bypass · Sonnet · High`), the same way the current three pills already show live values individually. "Runtime" appears as: the panel's `aria-label`, and in docs/tooltips only.

---

## 3. Visual layout — before / after

### 3.1 Before (current `main`)

Three independent drop-up pills, each its own `FlyoutMenu`:

```
┌──────────────────────────────────────────────────────────────────────┐
│ [Bypass ▴] [Sonnet 4.6 ▴] [high ▴]  [Shell]      ↑2.1k ↓480  1m12s  ⚙3  12.1k/64k │
└──────────────────────────────────────────────────────────────────────┘
```
Clicking any one pill opens only that pill's own popup — three separate click targets, three separate popups, no shared view across axes.

### 3.2 After (proposed)

One button, one popup, all three axes grouped inside it:

```
┌──────────────────────────────────────────────────────────────────────┐
│ [Bypass · Sonnet 4.6 · high ▴]  [Shell]      ↑2.1k ↓480  1m12s  ⚙3  12.1k/64k │
└──────────────────────────────────────────────────────────────────────┘
```

### 3.3 The panel — expanded state

Floats **upward** from the trigger (`placement: "top-start"`, same primitive `FlyoutMenu` uses — `computeMenuPosition` + `@floating-ui/dom` `autoUpdate`), grouped into three labeled sections:

```
┌─ mode ──────────────────────────┐
│  ✓ Bypass (no prompts)          │
│    Auto (AI classifier)         │
│    Accept Edits                 │
│    Plan (read-only)             │
│    Default (prompt all)         │
├─ model ─────────────────────────┤
│    Opus 4.8                     │
│  ✓ Sonnet 4.6                   │
│    Haiku 4.5                    │
├─ effort ────────────────────────┤
│    low                          │
│    medium                       │
│  ✓ high                         │
│    xhigh                        │
│    max                          │
└──────────────────────────────────┘
[Bypass · Sonnet 4.6 · high ▴]   ← trigger stays in place, panel floats above it
```

Legend / behavior notes:
- Check mark (`✓`, via the same `.menu-item-check` / `fa-check` convention `FlyoutMenu` uses) marks the current selection per section — no custom marker glyph invented.
- Mode's left-border color accent (permission-mode color coding) carries over from the current trigger pill onto the *consolidated* trigger's left edge.
- Section headers (`mode` / `model` / `effort`, lowercase to match the current Effort option casing convention) are non-interactive; arrow-key navigation skips over them.
- Model options are **registry-driven** (`getProvider(providerId)?.models`), so this list reflects whatever the live catalog overlay has resolved to at click time — not a hardcoded 3-item list.

### 3.4 Narrow-pane behavior

No change needed to the existing container-query rules: a single trigger pill is *narrower* than the three-pill layout it replaces, so the current 240px/320px breakpoints in `_composer-strip.scss` keep working without adjustment — this consolidation is itself a simplification of the exact narrow-width problem `SPEC_COMPOSER_STRIP_RESPONSIVE_ARCHITECTURE_2026_07_02` was solving.

---

## 4. Panel content rules

### 4.1 Structure

Single scrollable panel (reusing the app's `.menu` chrome, the same base class every `FlyoutMenu` popup uses), three labeled groups (`mode`, `model`, `effort`), each rendering its existing enum in existing UI-label order — **no reordering**, so muscle memory from the current three pills transfers directly:

- **mode** — `PermissionMode`, 5 values, same `menuLabel` descriptive text the current `MODE_OPTIONS` constant already defines (`"Bypass (no prompts)"`, etc.).
- **model** — from `getProvider(providerId)?.models` (falls back to the same 3-entry static list `AgentComposerStrip` currently falls back to when a provider defines none). No invented copy — labels come straight from the registry.
- **effort** — `EffortLevel`, 5 values (low/medium/high/xhigh/max), same plain lowercase labels the current `EFFORT_OPTIONS` constant already uses.

No new data modeling — this is a straight reuse of the same three constants/registry call the three-pill implementation already has, just rendered in one grouped list instead of three separate `FlyoutMenu` instances.

### 4.2 Provider-conditional rendering

The current implementation gates the **entire** controls zone on `providerId === "claude"` (`showControls()` in `AgentComposerStrip.tsx`) — non-Claude providers (codex/gemini/kimi) see no Mode/Model/Effort UI at all today. This consolidation preserves that exact gate rather than expanding it: the whole `AgentRuntimeDropup` is wrapped in the same `<Show when={showControls()}>` the three pills were wrapped in. Extending Mode-only visibility to non-Claude providers (which the original draft of this spec proposed, based on `buildRuntimeArgs.ts`'s per-provider `--yolo` capability) is **out of scope** here — it would be a scope expansion beyond "consolidate the existing three pills," not implied by the user's request, and would deviate from a gate the current codebase owners deliberately chose. Tracked as a possible follow-up, not part of this change.

### 4.3 Data-inconsistency check

The original draft of this spec flagged `docs/specs/agent-pane-runtime-controls.md`'s "Effort defaults to High" line as stale against a `DEFAULT_RUNTIME_CONFIG.effort` of `"medium"`. On current `main`, `DEFAULT_RUNTIME_CONFIG.effort` is `"high"` (`types.ts`) — the doc line is now **accurate**, and no longer needs the fix this spec originally proposed. Confirmed by reading current `types.ts` before writing this revision; no doc change ships with this PR.

---

## 5. Interaction & keyboard rules

- **Open:** click the trigger button. No dedicated keyboard shortcut for v1 — see §9.1 (unaffected by the retarget).
- **Navigate:** `↑`/`↓` move through every option row across all three sections, treating section headers as non-stops.
- **Letter-jump:** typing a letter jumps to the next row whose label starts with it.
- **Select:** `Enter` applies the highlighted row's value for its section and **stays open** — deliberate departure from `FlyoutMenu`'s close-on-select (`handleOnClick` in `flyoutmenu.tsx` always calls `onOpenChangeMenu(false)` after a leaf click), because the whole point of consolidating is letting one visit touch Mode, then Model, then Effort without reopening. See §9.2.
- **Close:** `Esc`, or click outside the panel (mirrors `FlyoutMenu`'s own `handleClickOutside` pattern). Clicking the trigger again also toggles closed.
- Every selection calls `applyRuntimeChange(blockId, provider, updatedConfig)` — the exact same function the three-pill implementation already calls (writes `agent:runtime` meta, and additionally resyncs the persistent controller process for Claude). No data-model or RPC-path changes.

---

## 6. Component restructure

### 6.1 New

- `frontend/app/view/agent/components/AgentRuntimeDropup.tsx` — the trigger button + floating panel. Positioning reuses the exact primitives `FlyoutMenu` uses (`@floating-ui/dom` `autoUpdate` + `computeMenuPosition` from `@/app/util/menu-position` + `Portal` + `data-pane-overlay`, `placement: "top-start"`, `avoidNativePanes: false`) rather than reinventing positioning — this keeps native-pane-HWND clipping and edge-avoidance behavior identical to every other popup in the app. Owns transient open/close + keyboard-selection-index state as local Solid signals (not reducer-owned — this is ephemeral UI state, same reasoning as `FlyoutMenu`'s own `isOpen` signal).

### 6.2 Deleted (from `AgentComposerStrip.tsx`)

- The `StripSelect` helper component.
- The `MODE_OPTIONS` / `EFFORT_OPTIONS` / `PERMISSION_COLORS` constants and the `MODEL_OPTIONS` fallback (all move into `AgentRuntimeDropup.tsx`).
- The `runtime()` / `updateRuntime()` locals (their logic moves into `AgentRuntimeDropup.tsx`, which reads/writes block meta directly given `blockId`/`blockAtom`/`providerId`).
- The `FlyoutMenu` import (no longer used directly by this file).

`AgentControlBar.tsx` is **not touched** by this change — its Mode/Model/Effort UI was already removed in a prior PR (#1912); it now holds only session-management banners/buttons, unrelated to this consolidation.

### 6.3 Modified

- `frontend/app/view/agent/components/AgentComposerStrip.tsx` — replace the three `<StripSelect>` invocations inside `<Show when={showControls()}>` with one `<AgentRuntimeDropup blockId={...} blockAtom={...} providerId={...} />`.
- `frontend/app/view/agent/styles/_composer-strip.scss` — replace `.agent-composer-strip-select*` and `.menu.strip-flyout*` rules with `.agent-runtime-dropup-trigger*` / `.menu.agent-runtime-dropup-panel` / `.agent-runtime-dropup-section` rules (same visual language: `border-radius: 0`, same border/color-mix conventions, same 10px font scale).

---

## 7. Edge cases

| Case | Behavior |
|---|---|
| Panel open, agent starts a new turn from elsewhere (e.g. voice input) | Panel stays open — a user may deliberately be queuing runtime changes for the *next* turn while a message is in flight. |
| Panel open, pane loses focus/unmounts | Closes for free (local signal, component unmount tears down the `autoUpdate` cleanup same as `FlyoutMenu`'s `onCleanup`). |
| Non-Claude provider | Entire trigger + panel hidden, matching current `showControls()` gating exactly (§4.2) — no partial/Mode-only display. |
| Live model-catalog overlay resolves after panel already opened | Panel re-renders with the new labels automatically — `getProvider()` reads a reactive Solid signal (`modelOverlay`), and the panel's row list is built from a plain function called in JSX, so it's inside Solid's tracking scope. No manual subscription needed. |
| `--prefers-reduced-motion` | No slide/fade transition — `FlyoutMenu` itself has none either (it just toggles `isOpen`), so this matches existing behavior with zero extra work. |

---

## 8. Out of scope (deferred follow-ups)

- **Extending Mode visibility to non-Claude providers** — a real scope expansion beyond consolidating the existing three pills; not requested, deviates from the current deliberate gate (§4.2). Worth a separate spec if wanted.
- **A dedicated keyboard shortcut to open the panel** — resolved against for v1, see §9.1.
- **Reordering options within a section** — explicitly not doing this (§4.1); stability over cleverness.
- **Progressive-collapse tiers from `SPEC_COMPOSER_STRIP_RESPONSIVE_ARCHITECTURE_2026_07_02` §3** — moot once there's only one trigger pill instead of three; that spec's narrow-width problem was specifically about *three* pills competing for space.

---

## 9. Resolved decisions (formerly open questions)

Unaffected by the architecture retarget — same reasoning applies whether the trigger being replaced was a native `<select>` or a `FlyoutMenu` pill.

### 9.1 Keyboard shortcut to open the panel directly

**Decision: no new chord for v1.** The button remains the mouse/discoverability path; `/model`, `/effort`, `/permission-mode` (and the freeform `/bypass`, `/plan`) remain the keyboard-first power-user path.

- **Best practice:** command-palette-style chords converge almost universally on `Cmd/Ctrl+K` or `Cmd/Ctrl+Shift+P`, and chords are explicitly "best reserved for power-user contexts" — not every action needs a shortcut, and any new chord must avoid colliding with conventions already claimed elsewhere in the same app ([Command Palette UX Patterns](https://medium.com/design-bootcamp/command-palette-ux-patterns-1-d6b6e68f30c1), [UX Patterns for Developers — Command Palette](https://uxpatterns.dev/patterns/advanced/command-palette), [Quentin Golsteyn — Keyboard shortcuts on the web](https://golsteyn.com/writing/designing-keyboard-shortcuts/)).
- **Codebase precedent:** `Ctrl:P` opens the app's `CommandPaletteModal`; `Cmd:k` already clears the terminal in terminal-focused panes. No free `Cmd/Ctrl+K`-style slot exists, and the obvious mnemonic `Ctrl:R` collides with the browser/OS "reload" convention.
- Revisit only if user feedback shows the click-to-open button is a measured bottleneck for power users.

### 9.2 Does selecting a value close the panel?

**Decision: stays open** — `Enter` or a click applies the highlighted/clicked row and the panel remains open; it closes only on `Esc`, outside-click, or re-clicking the trigger.

- **Best practice:** dropdown-interaction literature draws a sharp line between single-select controls (close immediately) and controls hosting **multiple independent choices in one place** (stay open across selections) ([NN/g — Listboxes vs. Dropdown Lists](https://www.nngroup.com/articles/listbox-dropdown/), [UXPin — Dropdown Interaction Patterns](https://www.uxpin.com/studio/blog/dropdown-interaction-patterns-a-complete-guide/)). This panel is the second kind: three independent axes, not one value from one list.
- **Codebase precedent:** generic single-value pickers (`FlyoutMenu`, `SlashCommandPicker`, native context menus) close on the first click. But purpose-built multi-control popovers (`HostPopover`, `TokenBreakdownPopover`) stay open across interactions, closing only via outside-click/Esc — the closer precedent for a panel hosting three independent settings.

### 9.3 Hide vs. disable for inapplicable options

**Decision: hide entirely**, not disabled-and-grayed — this now applies one level higher than originally scoped (§4.2): the *whole panel* is hidden for non-Claude providers (matching the current `showControls()` gate) rather than individual rows being hidden/disabled *within* an always-visible panel. The original per-option reasoning (structural vs. contextual unavailability — [Smashing Magazine — Hidden vs. Disabled in UX](https://www.smashingmagazine.com/2024/05/hidden-vs-disabled-ux/)) still applies to *why* it's hidden rather than disabled, just resolved at the provider-gate level instead of a within-panel level, since that's what the current codebase actually does.

---

## 10. Acceptance criteria

1. One always-visible trigger button in `AgentComposerStrip` (when `showControls()` is true) shows a live `Mode · Model · Effort` summary, replacing the three separate pills.
2. Clicking it opens a single floating panel using the same positioning primitives `FlyoutMenu` uses (`computeMenuPosition` + `autoUpdate`, `placement: "top-start"`), grouped into `mode`/`model`/`effort` sections in existing option order.
3. Model options are read from `getProvider(providerId)?.models` (registry/live-overlay-driven), not a hardcoded list.
4. Keyboard nav (`↑↓`, letter-jump, `Enter`, `Esc`) works across all rows/sections; `Enter` does not auto-close (§9.2).
5. Selecting a value calls the same `applyRuntimeChange(blockId, provider, updatedConfig)` every current control already calls — zero data-model or RPC-path changes.
6. Non-Claude providers see no trigger/panel at all, matching current behavior exactly (§4.2) — no regression, no scope expansion.
7. `StripSelect` and the three-pill JSX are fully removed from `AgentComposerStrip.tsx`; `AgentControlBar.tsx` is untouched.

---

## 11. Files this change touches

```
# New
frontend/app/view/agent/components/AgentRuntimeDropup.tsx       NEW

# Modified
frontend/app/view/agent/components/AgentComposerStrip.tsx       replace 3 StripSelect pills with 1 AgentRuntimeDropup
frontend/app/view/agent/styles/_composer-strip.scss             replace .agent-composer-strip-select*/.strip-flyout* with .agent-runtime-dropup-*

# Unchanged (reused, not modified)
frontend/app/view/agent/runtime-apply.ts                        applyRuntimeChange — reused as-is
frontend/app/view/agent/providers/index.ts                      getProvider/.models registry — reused as-is
frontend/app/view/agent/buildRuntimeArgs.ts                      no data-model changes
frontend/app/view/agent/types.ts                                 AgentRuntimeConfig/EffortLevel unchanged
frontend/app/view/agent/components/AgentControlBar.tsx           untouched — no Mode/Model/Effort content remains there
frontend/app/element/flyoutmenu.tsx                              not reused directly (flat MenuItem[] can't express grouped sections) but its positioning primitives are
```

---

*End of spec. Ready for review + go/no-go decision.*
