# SPEC: Move the narrow-width responsive tab bar to the top (from the bottom)

**Date:** 2026-08-24
**Status:** proposed
**Scope:** Armory, Warden, Settings — every pane with the rail↔bottom-tab-bar
responsive pattern.

---

## 0. Ask

> do a quick usability tweak. move the thinnest responsive menu to the top
> (from the bottom) it may apply to other panes too

---

## 1. Current behavior (audited against source, 2026-08-24)

Three panes share the same responsive pattern: a full left-hand rail nav at
normal widths, collapsing at a narrow container-query breakpoint into a
horizontal tab bar rendered as the **last** child of the pane (below the
content), positioned purely by DOM order in a `flex-direction: column`
container — no `order`/`position`/`bottom` CSS anywhere.

| Pane | JSX (root component) | Tab-bar CSS rule | Breakpoint (`@container ... max-width`) |
|---|---|---|---|
| Armory | `frontend/app/view/armory/armory-view.tsx:118` — `<nav class="bundle-manager-tab-bar">`, last child of `.armory-view` | `frontend/app/view/armory/armory-view.scss:91-98` (`.bundle-manager-tab-bar`, `border-top: 1px solid var(--border-color)`) | `armory-view.scss:161-170`, `≤479px` |
| Warden | `frontend/app/view/warden/warden-view.tsx:105` — same class name, own `<nav>`, last child of `.warden-view` | **Reuses Armory's `.bundle-manager-tab-bar` rule by convention** (no local border rule — `warden-view.scss`'s own file-header comment explains this is deliberate, not an oversight) | `warden-view.scss:79-96`, `≤479px` (own `@container warden` block, breakpoint values duplicated, class rules aren't) |
| Settings | `frontend/app/view/settings/settings-view.tsx:110` — `<nav class="settings-tab-bar">`, last child of `.settings-view` | `frontend/app/view/settings/settings.scss:425-432` (`.settings-tab-bar`, `border-top: 1px solid var(--border-color)`) — fully independent class, own rule | `settings.scss:~560-566`, own breakpoint |

No shared `<TabBar>` component — three independently-authored `<nav>` blocks
(Warden's JSX is its own, only its *CSS class* is shared with Armory's).
Moving position requires touching each pane's JSX individually; the CSS
`border-top`→`border-bottom` flip only needs to happen twice (Armory's rule
covers Warden too via the shared class).

---

## 2. Design

**Move each `<nav class="...-tab-bar">` to be the FIRST child of its pane
root** (before the rail and before the content), for all three panes.
Positioning stays pure DOM order — no `order`/`position:sticky` needed, same
mechanism as today, just reordered. The rail (`.bundle-manager-rail` /
`.settings-rail`) itself is unaffected — it's `display: none` at the same
breakpoint the tab bar becomes visible, so their relative order in the DOM
never matters visually; only the tab-bar-vs-content order does.

Each affected `.../-tab-bar` CSS rule's `border-top` becomes `border-bottom`
— separating the bar from the content now rendered *below* it, matching the
same visual "hairline between nav and content" intent, just on the other
edge. Since Armory's rule covers Warden too, this is one CSS edit for both,
plus one for Settings.

No breakpoint values, no `display`/`flex` properties, no button markup
inside each `<nav>` change — purely a reorder + one border-side flip per
pane.

### 2.1 Test updates

`armory-view.test.tsx:173` and `warden-view.test.tsx:162` both have tests
titled `"clicking a bottom tab-bar item writes ..."` — the assertions
themselves (query by `aria-label` + `nav.bundle-manager-tab-bar` selector,
click, assert the RPC call) are unaffected by DOM order, but the test names
go stale ("bottom" no longer accurate). Rename to `"clicking a tab-bar
item..."` — no behavioral test changes needed, this is a pure position
move with identical click/RPC behavior.

### 2.2 Active-tab indicator flips with the bar

The tab-bar's active-item indicator (`&.is-active { border-top: 2px solid
var(--accent-color); }` in `armory-view.scss`/`settings.scss`) was designed
for a bottom-positioned bar — the top border visually "attaches" the active
tab to the content rendered above it. Now that the bar renders at the top,
that indicator flips to `border-bottom` too, so it still attaches to the
content it's showing (now rendered below), not the far edge away from it.
Settings' tab bar has no equivalent per-item active-border rule (its
`.is-active` only recolors the icon), so nothing to flip there.

### 2.3 ABF gets a standing accent highlight (both responsive forms)

Follow-up ask in the same conversation: give the Armory rail's "ABF"
entry (`RAIL`'s `id: "bundles"` in `armory-view.tsx:29`, tooltip "Armory
Bundle Format (ABF)") a highlighted color, in both `.bundle-manager-rail`
(full width) and `.bundle-manager-tab-bar` (narrow) — i.e. regardless of
which responsive form is currently showing.

New class `is-abf-highlight`, applied only to the `id === "bundles"` button
in both `<For>` loops (`armory-view.tsx`). CSS in `armory-view.scss`, on
both `.bundle-manager-rail-item` and `.bundle-manager-tab-bar button`:
`&.is-abf-highlight:not(.is-active) { i { color: var(--accent-color); } }`
— tints just the icon when NOT the active section (so it stands out at
rest), and steps aside when active (the existing `.is-active` rule already
provides full-button contrast via `background`/`color`, so stacking a
second accent treatment on top would be redundant, not additive). Warden
and Settings are unaffected — this is Armory-specific, not part of the
shared tab-bar pattern.

---

## 3. Out of scope

- No new panes get this responsive pattern added — purely repositioning the
  three that already have it.
- No shared `<TabBar>` component extraction — a real, separate cleanup
  opportunity (three copy-pasted implementations), not requested here and
  not needed to satisfy this ask.
- No breakpoint value changes — narrow-width thresholds stay exactly as they
  are today.

---

## 4. Test plan

- [ ] Armory: `<nav class="bundle-manager-tab-bar">` renders before
      `.bundle-manager-section` in the DOM; existing click/RPC test passes
      unmodified (renamed, not rewritten).
- [ ] Warden: same, `<nav>` before `.bundle-manager-section`.
- [ ] Settings: `<nav class="settings-tab-bar">` renders before
      `.settings-body`.
- [ ] Manual (`task dev`, narrow the Armory/Warden/Settings pane below
      479px): confirm the tab bar visually appears at the top with a
      bottom-edge hairline, rail is still hidden, content still scrolls
      correctly below it.
- [ ] ABF's rail button and tab-bar button both carry `is-abf-highlight`;
      no other `RAIL` entry does.
