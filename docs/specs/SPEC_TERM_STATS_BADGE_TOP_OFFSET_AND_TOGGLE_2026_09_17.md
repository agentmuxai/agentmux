# SPEC: Terminal CPU%/Mem badge — fix top-right offset, add a Settings toggle

**Date:** 2026-09-17
**Status:** active — implemented on `clare/term-stats-badge-top-offset-and-toggle`,
not yet merged. Update to `implemented` with the PR number once it lands.

---

## 0. Motivation

The terminal pane's live CPU%/memory badge (`BlockStatsBadge`,
`frontend/app/element/blockstats.tsx`) is meant to sit flush against the
top-right corner of the pane. In practice it renders offset well below the
top — floating inside the terminal's own content area rather than hugging the
corner. Separately, there was no way to turn the badge off if a user doesn't
want it.

This spec covers both: the offset fix (§1–2) and the new Settings toggle
(§3), scoped specifically to terminal panes per the user's request.

## 1. Root cause

`.block-stats-badge` (`frontend/app/element/blockstats.scss`) is positioned:

```scss
.block-stats-badge {
    position: absolute;
    top: calc(var(--header-height) + 4px);
    right: 6px;
    ...
}
```

`--header-height` is a fixed `33px` (`frontend/app/theme.scss`), and the
badge is rendered as the last child of `.block-frame-default-inner`
(`frontend/app/block/blockframe.tsx`), whose first child is normally the
pane's own inline header row — so reserving 33px + 4px of top offset is
correct *when that header row is actually there*.

It often isn't. `BlockFrame_Default` suppresses its own inline header
whenever the ViewModel's `noHeader` reads true — which is exactly the case
for terminal (and agent) panes today: `TermViewModel.noHeader` returns
`this.nodeModel.paneChromeHoisted === true`, meaning
`pane-leaf-chrome.tsx` rendered the *real* header **above** this component
instead (`frontend/app/view/term/termViewModel.ts`, comment: "Terminal now
renders through the ONE shared chrome like every other widget type").

When the header is hoisted out, `.block-frame-default-inner` no longer
reserves any header-height space at its own top — its content starts at
`top: 0` — but `.block-stats-badge`'s CSS still unconditionally subtracts a
phantom 37px, landing the badge well inside the terminal's actual content
instead of flush against the pane's real top-right corner.

## 2. Fix

`blockframe.tsx` already reads `noHeader` (the same value it uses to decide
whether to render its own inline header at all) — threaded it through as a
prop instead of adding a new signal:

```tsx
// blockframe.tsx
const noHeader = util.useAtomValueSafe(props.viewModel?.noHeader);
...
<BlockStatsBadge blockId={nodeModel.blockId} noHeader={noHeader} />
```

```tsx
// blockstats.tsx
export function BlockStatsBadge(props: { blockId: string; noHeader?: boolean }) {
    ...
    <div class={`block-stats-badge ${cpuClass()} ${props.noHeader ? "no-header" : ""}`}>
```

```scss
// blockstats.scss
.block-stats-badge {
    top: calc(var(--header-height) + 4px);
    ...
    &.no-header {
        top: 4px;
    }
}
```

`.block-stats-badge` is a genuine DOM child of `.block-frame-default-inner`
only — `ConnStatusOverlay` and `.block-mask` are siblings at the
`.block-frame-default` level, not descendants, so this fix can't affect
them. Verified via `docs/specs/README.md`-style code tracing (exact JSX
nesting in `blockframe.tsx`, exact selector nesting in `block.scss`) rather
than a live screenshot — this session had no capturable GUI window
available (`DiscoverWindows` returned none), so the positioning claim is
backed by reading the DOM/CSS structure precisely, not by visual
confirmation. Recommend a human sanity-check on the next `task dev` before
merge.

## 3. Settings toggle

Per the user's explicit scoping ("this is specifically a display for the
terminal pane only"), the new toggle only affects `view === "term"` blocks —
agent panes (which also get a stats badge today, since they're PTY-backed
too) keep the prior always-on behavior unchanged, out of scope for this
change.

**Setting key:** `term:showstatsbadge` (boolean, default `true` — matches
prior always-on behavior for existing users). Frontend-only, following the
`term:predictiveecho` precedent: not every `term:*` setting has a
corresponding Rust `wconfig::SettingsType` field (confirmed via
`test_settings_unknown_keys_passthrough` in
`agentmux-srv/src/backend/wconfig/mod.rs` — unknown keys round-trip through
settings.json untouched), since this key is consumed only by the frontend
render path and never read by Rust code.

Touched:
- `frontend/types/srv-types.d.ts` — added `"term:showstatsbadge"?: boolean;`
  to the hand-maintained `SettingsType` (this file predates ts-rs codegen
  removal; see its own header comment).
- `schema/settings.json` — added the matching schema entry (`type:
  "boolean"`, `default: true`, description).
- `frontend/app/view/settings/sections/terminal-section.tsx` — new
  `ToggleControl` row, "Show CPU/mem badge", placed alongside the other
  terminal display toggles.
- `frontend/app/block/blockframe.tsx` — new `showStatsBadge` memo:
  ```tsx
  const showStatsBadge = createMemo(
      () => blockData()?.meta?.view !== "term" || getSettingsKeyAtom("term:showstatsbadge")() !== false,
  );
  ```
  wraps the existing `<BlockStatsBadge>` render in
  `<Show when={showStatsBadge()}>`. Non-term views always pass this check
  (unaffected); term views additionally respect the setting.

Not touched: the backend's per-block CPU/mem sampling
(`WpsEvent.BlockStats`, `useBlockStats.ts`) keeps running and broadcasting
regardless of this setting — it's a pure frontend display toggle, not a
sampling on/off switch. Out of scope; the user asked for a way to hide the
badge, not to reduce backend sampling overhead.

## 4. Testing

- `npx tsc -p tsconfig.citypecheck.json --noEmit` — clean.
- `npx stylelint frontend/app/element/blockstats.scss` — clean.
- `npx vitest run frontend/app/element frontend/app/block frontend/app/view/settings` —
  120/120 passing (no existing test covers `BlockStatsBadge` or
  `terminal-section.tsx` directly; none added here since neither file had
  prior test coverage to extend and the change is a straightforward
  prop/settings-key threading, not new logic worth a dedicated unit test).
- Not done: a live `task dev` visual check (see §2's caveat).
