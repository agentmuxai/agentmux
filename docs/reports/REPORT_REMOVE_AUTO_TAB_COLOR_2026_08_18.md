# Report: Remove automatic tab-color assignment (random + startup default)

**Date:** 2026-08-18
**Status:** Analysis complete — no code changed. Written to inform implementation.
**Scope:** `frontend/app/store/tab-actions.ts`, `frontend/app/tab/tabbar.tsx`.

## Ask

Default tabs and new tabs currently get an automatically-assigned color. Remove that — all tabs (default and new) should start with no color ("clear"). Users can still set a color manually afterward via the existing right-click color picker.

## Current behavior — two separate auto-assignment mechanisms

### 1. Random color on every new tab

`frontend/app/store/tab-actions.ts:22-32`, in `createTab()`:

```ts
function randomNewTabColor(): string {
    return TAB_COLORS[Math.floor(Math.random() * TAB_COLORS.length)].hex;
}
```

Called immediately after `WorkspaceService.CreateTab(...)`, via `ObjectService.UpdateObjectMeta(oref, { "tab:color": randomNewTabColor() })` (`tab-actions.ts:50-54`). Every tab created through this path — `Ctrl+T`, the "+" button, the command palette's "New Tab" — gets a random hex from the 14-entry `TAB_COLORS` palette (`tab.tsx:28-43`), no user input involved.

### 2. Fixed "Blue" on the startup (first) tab

`frontend/app/tab/tabbar.tsx:99-128`, an `onMount` effect: if the workspace has exactly one tab and it has no `tab:color` set yet (and hasn't been backfilled before, per the `tab:color-initialized` guard), it applies `#2562c5` ("Blue" in `TAB_COLORS`) unconditionally. This exists because the backend-created startup tab doesn't get a color meta at creation time, so without this, the very first tab a user ever sees would look different (neutral) from every subsequently-created tab (vibrant) — the backfill was added specifically to make the *first* tab consistent with the *random* behavior of every tab after it, not as an independent feature.

**These two are coupled**, not independent: #2 exists to paper over the inconsistency #1 creates. Removing #1 without #2 would leave the startup tab colored and every new tab neutral — the opposite of today's problem. Removing #1 makes #2 pointless (nothing left for it to stay "consistent" with), so both need to go together, which is what's proposed below.

## Why the random-color feature exists (for context)

The code comment states the rationale directly: "so successive tabs read as visually distinct at a glance instead of all defaulting to the same hue." This is a real, legitimate UX goal — in a multi-tab workspace, color is a fast visual anchor for "which tab am I looking at" without reading labels. Worth naming honestly, since removing the feature does give up that automatic behavior.

## Why removing it is reasonable anyway

- **Unpredictability, not choice.** A *random* color isn't really helping the user express intent — it's assigning them a color they didn't pick and may not want, which they then have to actively override if it clashes with how they'd have organized things themselves (e.g., a user who *does* want color-coding by project/purpose gets a color they now have to change before applying their own scheme).
- **Manual color-coding is already fully supported and low-friction** — right-click a tab → 14-swatch picker → done (`tab.tsx:80-100`, `ColorSwatchPalette`). Nothing about "distinguish tabs visually" is lost as a *capability*; only the *default-on* behavior goes away. A user who wants the "glanceable" benefit still gets it — they just author it instead of receiving it randomly.
- **A neutral default is not a broken/undefined state.** `tab.tsx:129,259,261` already treats an absent `tab:color` as a clean, fully-supported render path (`"tab-colored": !!tabColor()` only applies the colored-background class when a color is actually set — no color reads as the tab's plain default style, not as an error state or a visual gap). There's also already an explicit "✕ Clear color" action in the picker (`color-swatch-palette.tsx:46-50`, `showClear` defaults to `true`), confirming "no color" is a first-class, already-designed-for state in this UI — this change just makes it the *starting* state instead of something a user has to opt into via Clear.
- **Consistency with "start neutral, personalize deliberately"** is arguably the more common pattern elsewhere in the app already (e.g. agent panes don't get a random border color either — `AGENT_COLOR_PALETTE` auto-assignment is scoped to something else per `SPEC_TAB_COLOR_DESATURATION_2026_08_13.md`'s own note that the two palettes were deliberately forked to stay independent).

## What's explicitly NOT affected

- **Tabs that already have a `tab:color` set** — whether from a prior random assignment, the startup backfill, or a manual pick — are untouched. This removes only the *going-forward* auto-assignment on tab creation/first-mount; it is not a migration and does not clear existing colors. (Same non-migration posture the desaturation spec already established for this exact meta field — `tab:color` stores a literal hex, there's no "was this auto-assigned or user-chosen" flag to distinguish retroactively, and there's no reason to guess.)
- **The manual color picker itself** — `TAB_COLORS`, `ColorSwatchPalette`, the right-click menu, `onColorSelect` — none of this changes. Users pick colors exactly as they do today.

## Proposed implementation

1. **`tab-actions.ts`** — in `createTab()`, delete the `ObjectService.UpdateObjectMeta(..., { "tab:color": randomNewTabColor() })` call entirely (not replace it with a null-set — simply never write `tab:color` for a freshly created tab, so it's absent, same as any other never-touched meta key). Delete the now-unused `randomNewTabColor()` function and, if nothing else in the file still needs it, the `TAB_COLORS` import.
2. **`tabbar.tsx`** — delete the entire startup-tab-color `onMount` effect (lines 99-128), including the `tab:color-initialized` guard write — with #1 gone, there's no "every other tab is colored" inconsistency left to backfill against, so the guard has nothing left to protect.
3. No backend/RPC changes needed — both mechanisms are frontend-only meta writes on top of the existing generic `tab:color` field; the field itself, its type, and every read path stay exactly as-is.

## Verification plan

- Open a fresh workspace (or one with a single default tab) — confirm the startup tab renders with no color (no `.tab-colored` class, default tab background).
- Create several new tabs (`Ctrl+T`, "+", command palette) — confirm none get an automatic color.
- Manually pick a color on a tab via right-click → swatch — confirm it still applies and persists normally (this path is unchanged).
- Manually clear a color via "✕ Clear color" — confirm it goes back to the same neutral default new tabs now start with (should already work, since this is the same code path `tabColor()` already treats as the falsy/no-color case).
- Confirm no leftover references to `randomNewTabColor` or the startup-backfill's `tab:color-initialized` meta key elsewhere in the codebase after removal (a quick grep, not expected to find anything given the search already done for this report).
