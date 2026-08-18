# SPEC: Widget bar parent widgets (grouped submenus)

**Date:** 2026-08-12
**Status:** proposed

---

## 0. Ask

Introduce **parent widgets** on the action widget bar: a widget entry that,
instead of opening a pane on click, expands a submenu listing **child**
widgets — "just like our right-click context menus." First concrete case:
a **"Messengers"** parent grouping the 5 existing messenger widgets
(`defwidget@teams`, `defwidget@slack`, `defwidget@discord`,
`defwidget@telegram`, `defwidget@whatsapp`).

Two placements a parent widget can be interacted from, both need to work:

- **A. Pinned** — the parent sits directly in the bar like any other pinned
  widget.
- **B. Inside "More"** — the parent is one row in the `···More` overflow
  dropdown (`frontend/app/window/more-dropdown.tsx`).

---

## 1. Audit — what already exists

### 1.1 Widget bar today (flat, two tiers)

`frontend/app/window/action-widgets.tsx` renders **pinned** widgets directly
in the bar; everything else lives in the **More** dropdown
(`more-dropdown.tsx`). Both tiers are flat lists — no grouping concept
exists anywhere in the widget system today. Config helpers live in
`action-widgets-config.ts` (`getPinnedWidgets`/`getMoreWidgets`/`pinWidget`/
`unpinWidget`), driven by the `widget:pinned` settings array (ordered
short-names) with `display:pinned` in `widgets.json` as the new-install
default (see `docs/specs/widget-pinning.md`).

Widget definitions live in `agentmux-srv/src/config/widgets.json`, typed as
`WidgetConfigType` in `agentmux-srv/src/backend/wconfig/types.rs:478` (Rust)
and mirrored in `frontend/types/gotypes.d.ts:2106` (TS). Current shape:

```rust
pub struct WidgetConfigType {
    display_order: f64,       // "display:order"
    display_hidden: bool,     // "display:hidden"
    display_pinned: bool,     // "display:pinned"
    icon: String,
    color: String,
    label: String,
    description: String,
    magnified: bool,
    block_def: BlockDef,      // "blockdef" — what pane opens on click
}
```

**The 5 messenger widgets already exist as flat, standalone entries** —
`defwidget@discord`, `defwidget@slack`, `defwidget@telegram`,
`defwidget@whatsapp`, `defwidget@teams` (`widgets.json` lines 121–195), each
`display:pinned: false`, each a `view: "browser"` blockdef pointed at the
service's web app. Today they only differ from any other widget by being
pre-set browser shortcuts — nothing already groups them. This is the
concrete first parent/children set for this feature.

**Aside (found during audit, not part of this spec):** `CLAUDE.md`'s widget
table lists `browser`/`terminal`/`sysinfo`/`editor`/`help` as "Pinned"
tier, but `widgets.json` has all five as `display:pinned: false` today.
Since no `widget:pinned` default exists in `settings-template.jsonc`, a
fresh install's actual pinned set is whatever `display:pinned: true`
produces — `agent`, `swarm`, `drone`, `warden`, `media` only. Worth a
docs-fix follow-up; flagging here since a "Messengers" default-pinned
decision (§3.6) should be made with the real current defaults in mind, not
the stale table.

### 1.2 Submenu infrastructure already exists and is directly reusable

Two DOM submenu implementations exist, and — as of
`SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_2026_08_10.md`, already
merged to `main` — both route through one shared, framework-agnostic
hover-intent core: **`frontend/app/util/submenu-hover.ts`**
(`createSubmenuHover`). It gives:

- **90ms open delay** on trigger-row hover (avoids opening every submenu
  the cursor merely passes over).
- **Safe-triangle close** — the submenu stays open while the cursor moves
  through the triangle from the point it left the trigger row toward the
  submenu panel's near edge, with a 300ms absolute backstop timer either
  way. This is exactly the "diagonal mouse movement into the submenu
  shouldn't slam it shut" behavior implied by "expand just like our
  right-click context menus."
- **Peer close** — a `createPeerRegistry()` helper (in
  `flyoutmenu.tsx`) force-closes any other open sibling submenu the
  instant a new sibling row is entered, on top of (not instead of) the
  safe-triangle grace period for that row's own approach.

`FlyoutMenu`/`SubMenu` (`frontend/app/element/flyoutmenu.tsx`) is the
SolidJS-native consumer: `MenuItem.subItems?: MenuItem[]` renders a nested
`<SubMenu>` positioned via the shared `computeMenuPosition()` primitive
(`frontend/app/util/menu-position.ts`) — anchored to the parent row's rect,
preferred placement `right-start` (flips to `left-start` near the window
edge), `autoUpdate` from `@floating-ui/dom` keeps it live on scroll/resize,
and it stays `visibility:hidden` until the real position is known (fixes
the positioning-flash bug the same spec addresses). It already carries
`data-pane-overlay` / `usePaneOverlay()` so it draws correctly on top of a
native browser-pane HWND sitting behind it — the same requirement
`more-dropdown.tsx` already has (comment there: *"cut a transparent hole
through any browser pane HWND behind this dropdown"*).

`MoreDropdown` itself is a good structural template for a **pinned**
parent's flyout (Case A below): it's a `Portal`-rendered floating panel,
anchored to a trigger button via `computeMenuPosition()` +
`autoUpdate`, closes on outside-mousedown, and separately renders a
`PopoverMenu` for per-item right-click actions that must stay open
independently of the dropdown itself.

`ContextMenuModel.showContextMenu()` (`frontend/app/store/contextmenu.ts`)
is a **third**, unrelated path — it hands off to the OS-native context
menu (`NativeContextMenuItem.submenu`), used for actual right-click
(`handlePinnedContextMenu` in `action-widgets.tsx`). It is **not** a
candidate for the parent-widget expand UI — that's a native OS menu, not a
DOM flyout, and can't be anchored/positioned the way an in-bar expand
needs to be. Case A/B below build on the DOM path (`submenu-hover.ts` +
`computeMenuPosition()`), matching what right-click submenus look and feel
like without literally routing through the native-menu API.

`PopoverMenu` (`frontend/app/element/popover-menu.tsx`) — used today for
the pin/unpin item menu spawned from `ActionWidgets`/`MoreDropdown` — is
**single-level only** (sections, but no nested `subItems`/hover-intent). Not
a fit for the parent/child expand itself; still the right tool for the
existing "pin/unpin" per-item action menu, unchanged by this spec.

### 1.3 Responsive collapse & drag-reorder

`use-widget-bar-responsive.ts` measures pinned-widget width via hidden
mirror elements to decide the 3-tier label/icon/overflow collapse.
`use-widget-drag-reorder.ts` drives pinned-bar drag-to-reorder. Both treat
`pinnedWidgets()` as a flat array of `{key, widget}` — a parent widget is
just one more entry in that array from their point of view; **no changes
needed** to either as long as a parent widget is represented as a single
`{key, widget}` slot (§3.1).

---

## 2. Goals / non-goals

**Goals**

- A widget entry can declare itself a parent with an ordered list of child
  widget keys.
- Pinned parent (Case A) and More-dropdown parent (Case B) both expand a
  child list using the *same* hover-intent timing and positioning
  primitives already used by every other submenu in the app.
- Right-click on a parent still works and is scoped sensibly (group-level
  actions, not a single pane's actions).
- Existing single-widget behavior (pin/unpin, drag-reorder, responsive
  collapse, right-click, "Open in New Window") is unaffected for anything
  that isn't a parent or a grouped child.
- Ship the "Messengers" group as the first real usage, folding in the 5
  existing messenger widgets without changing their individual `blockdef`s.

**Non-goals (v1)**

- Nested groups (a parent whose child is itself a parent). Nothing in the
  design below forbids it later — `FlyoutMenu`'s `SubMenu` already
  recurses — but no current use case needs it, so it's untested scope.
- Per-child pin-to-bar that "promotes" a grouped child out of its parent
  onto the bar as its own standalone pinned entry. Right-click on a child
  still offers "Open in New Window" / "Open in Floating Pane" (unchanged),
  but not "pin this one directly." Flagged as an open question (§6).
- A "customize groups" UI for user-defined parents. `widgets.json` is
  the only place parents are authored, same as widgets today.

---

## 3. Design

### 3.1 Data model

Add one field to `WidgetConfigType`, both sides:

**Rust — `agentmux-srv/src/backend/wconfig/types.rs`**

```rust
pub struct WidgetConfigType {
    // ...existing fields unchanged...

    /// Ordered short-names (no "defwidget@" prefix, same convention as
    /// `widget:pinned`) of this widget's children. Presence of a
    /// non-empty `children` list is what makes a widget entry a "parent":
    /// its own `blockdef` is not used to open a pane and can be omitted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<String>,
}
```

`block_def: BlockDef` stays as-is — it already deserializes to
`BlockDef::default()` (empty `files`/`meta`) when the `blockdef` key is
absent from JSON, so a parent entry can omit `blockdef` entirely today
with no struct change. Add a config-load-time validation (not a type-level
`Option`, to keep the diff minimal): a widget must have **either** a
non-empty `blockdef.meta` **or** a non-empty `children` list, never
neither, never both — surfaced as a startup log warning, not a hard
failure (matches how other soft config issues are handled).

**TS — `frontend/types/gotypes.d.ts:2106`**

```ts
type WidgetConfigType = {
    "display:order"?: number;
    "display:hidden"?: boolean;
    "display:pinned"?: boolean;
    icon?: string;
    color?: string;
    label?: string;
    description?: string;
    magnified?: boolean;
    children?: string[];   // new
    blockdef: BlockDef;
};
```

(This file is generated from the Rust types elsewhere in the build; listed
here for completeness of the frontend-visible contract.)

**`agentmux-srv/src/config/widgets.json`**

```jsonc
"defwidget@messengers": {
    "display:order": 10,
    "display:pinned": false,
    "icon": "comments",
    "label": "Messengers",
    "description": "Team chat and messaging apps",
    "children": ["discord", "slack", "telegram", "whatsapp", "teams"]
    // no "blockdef" — this entry never opens a pane directly
},
```

The 5 existing messenger entries (`defwidget@discord`, `defwidget@slack`,
`defwidget@telegram`, `defwidget@whatsapp`, `defwidget@teams`) are
unchanged in shape — same `blockdef`, same icon/color/label — except they
gain `"display:hidden": true`. §3.2 explains why hidden, not merely
un-pinned.

### 3.2 Grouped children are hidden from the flat lists

Once a widget key appears in some parent's `children`, it must not *also*
appear as its own row in the top-level bar or in More — otherwise a user
sees "Slack" twice: once standalone in More, once inside "Messengers."
`getPinnedWidgets()`/`getMoreWidgets()` (`action-widgets-config.ts`) both
need one new filter step:

```ts
function getGroupedChildKeys(wmap: Record<string, WidgetConfigType>): Set<string> {
    const grouped = new Set<string>();
    for (const w of Object.values(wmap)) {
        for (const child of w.children ?? []) grouped.add(child);
    }
    return grouped;
}
```

`getPinnedWidgets`/`getMoreWidgets` exclude any key in this set from their
top-level enumeration (a parent widget itself is never in this set unless
someone nests groups, which is out of scope — §2). This is why the
messenger entries additionally get `display:hidden: true` in §3.1: it's
belt-and-suspenders documentation of intent in the config file itself (a
human reading `widgets.json` sees these are not top-level-reachable),
even though the *actual* exclusion is enforced by the grouped-key filter
so it also protects against a future author forgetting the flag.

Resolving a parent's children for rendering is a pure lookup, not
filtering: `(widget.children ?? []).map(shortName => wmap[`defwidget@${shortName}`]).filter(Boolean)`.

### 3.3 Case A — pinned parent in the bar

**Trigger element:** the existing `.action-widget-slot` /
`<ActionWidget>` row, unchanged in structure. Its `divOnClick` currently
calls `handleWidgetSelect(widget)` (opens a pane) — for a parent widget
(`widget.children?.length`), this must instead **toggle** the flyout
open/closed, exactly like the existing More button's `openMore` handler
(`action-widgets.tsx:121`), not call `handleWidgetSelect`. This mirrors
"click toggles" as the accessible/discoverable fallback for anyone who
doesn't hover-dwell, same rationale as the More button already having
both.

**Hover-intent:** in addition to click-toggle, wire the row's
`onMouseEnter`/`onMouseLeave` through a `createSubmenuHover` controller —
the same 90ms-open / safe-triangle-close primitive every other submenu in
the app uses. This is the literal "expand just like our right-click
context menus" ask. One controller per pinned parent slot (there may be
more than one parent pinned at once, e.g. "Messengers" + a hypothetical
future "Dev Tools" group) — wrap them in a `createPeerRegistry()` (same
helper `flyoutmenu.tsx` already exports the *pattern* for, though it's
currently private to that file — worth hoisting to `submenu-hover.ts` or a
new small shared module, since both `MenuBody`/`SubMenu` internally and
this new pinned-bar case need "close my siblings when I open" — see §5)
so opening "Messengers" force-closes any other open pinned-parent flyout
immediately, matching in-bar sibling behavior to in-menu sibling behavior.

**Panel:** new component, e.g. `PinnedWidgetFlyout` — structurally a clone
of `MoreDropdown` (§1.2): `Portal`-rendered, `usePaneOverlay()` +
`data-pane-overlay` (browser-pane HWND clipping — non-negotiable, every
floating menu in this app needs it), positioned via `computeMenuPosition()`
anchored to the pinned slot's own element (not the whole bar), preferred
placement `bottom-start` (opens downward under the icon — matches how the
More dropdown already opens under its button; this is *not* a sideways
`right-start` submenu, because the trigger lives in a horizontal bar, not
a vertical menu list). Starts `visibility:hidden` until positioned (same
flash-avoidance requirement as `SubMenu`).

**Rows inside the panel:** each child widget renders like a normal
`MoreDropdown` item — icon + label, click calls `handleWidgetSelect(child)`
and closes the flyout. Right-click on a child row opens the existing
per-item `PopoverMenu` (Open in New Window / Open in Floating Pane /
separator / Pin-Unpin) unchanged — reuse `buildItemMenuItems`, just resolve
`shortName` against the child instead of a top-level widget.

**Closing:** outside mousedown (existing pattern), `Escape`, and the
safe-triangle timeout. Selecting a child closes it (same as More).

### 3.4 Case B — parent as a row inside the More dropdown

This one is a much closer fit for the *existing* `FlyoutMenu`/`SubMenu`
machinery than Case A, because More-dropdown rows are already a vertical
list — a parent row there behaves exactly like a `MenuItem` with
`subItems` inside `FlyoutMenu` (hamburger menu's Theme/Opacity submenus
are the existing example of this exact shape).

Two implementation options, in preference order:

1. **(Recommended) Give `MoreDropdown` its own lightweight `SubMenu`
   support**, structurally identical to `flyoutmenu.tsx`'s `SubMenu`:
   `createSubmenuHover` per parent row, `computeMenuPosition()` anchored
   to that row's rect with placement `right-start` (flips to `left-start`
   near the right edge — More already right-aligns itself to its own
   anchor, so this flip matters more here than in Case A), same
   `visibility:hidden`-until-positioned guard, same `data-pane-overlay`.
   This keeps `MoreDropdown`'s existing item-click / item-context-menu
   wiring (`onItemContextMenu`, `handleItemClick`) intact and just adds a
   third branch (parent row → render `SubMenu` instead of a leaf item).
2. **(Rejected for v1) Route More's item list through `FlyoutMenu`
   itself.** `FlyoutMenu` owns its own trigger/open-state
   (`menu-anchor`/`isOpen`) which doesn't match how `MoreDropdown` is
   already triggered (externally, by `ActionWidgets`' own `moreOpen`
   signal) — bending `FlyoutMenu` to accept an externally-controlled open
   state is a larger refactor than duplicating the ~80-line `SubMenu`
   shape, for no behavioral difference to the user. Revisit only if a
   third consumer needing the same "externally-triggered flyout with
   nested subItems" shape shows up.

**Row indicator:** a parent row in More gets the same
`<i class="fa-sharp fa-solid fa-chevron-right" />` trailing icon
`FlyoutMenu` already renders for any `item.subItems` — free visual
consistency, no new icon/affordance to invent.

**Closing:** parent submenu closes via its own safe-triangle/timer,
independent of the outer More dropdown's own outside-mousedown close
(same nesting model `FlyoutMenu`'s `SubMenu`-inside-`MenuBody` already
has — an inner `Show` unmount, not a shared close flag).

### 3.5 Right-click on a parent

Scope right-click to **group-level** actions — it must not offer
"Open in New Window" (there's no single pane a parent represents) or a
generic "Unpin from bar" phrased the same as a leaf widget's, since
unpinning a group has a different mental model (the whole group leaves
the bar, not "this one pane's shortcut"). Proposed menu, reusing
`ContextMenuModel.showContextMenu` exactly as `handlePinnedContextMenu`
does today:

```
Pinned parent (bar):
  Unpin group from bar
More-dropdown parent:
  Pin group to bar
```

Right-click on a **child row**, inside either flyout, keeps today's
per-widget `PopoverMenu` (Open in New Window / Open in Floating Pane /
Pin-Unpin) exactly as-is — §2 already scopes out per-child promotion, so
"Pin to bar" on a child is left as todo/disabled pending the §6 decision,
not silently different behavior.

### 3.6 Should "Messengers" ship pinned or in More?

Given the audit finding in §1.1 (actual current pinned defaults are
`agent`/`swarm`/`drone`/`warden`/`media`, not the CLAUDE.md table), default
`defwidget@messengers` to `"display:pinned": false` — lands in More on
first install, same tier its 5 children individually occupy today. This
is the lower-risk default (existing installs' `widget:pinned` arrays are
untouched; new-install defaults don't change bar density) and lets both
Case A and Case B get exercised by any user who chooses to pin it or not.
Do not special-case a "new group defaults to pinned" rule — no other
recent widget addition has done that either.

---

## 4. Files touched (est.)

| File | Change |
|---|---|
| `agentmux-srv/src/backend/wconfig/types.rs` | Add `children: Vec<String>` to `WidgetConfigType`; load-time validation (blockdef xor children) |
| `agentmux-srv/src/config/widgets.json` | Add `defwidget@messengers`; add `"display:hidden": true` to the 5 messenger entries |
| `frontend/types/gotypes.d.ts` | Mirror `children?: string[]` on `WidgetConfigType` |
| `frontend/app/window/action-widgets-config.ts` | `getGroupedChildKeys()`; filter it out of `getPinnedWidgets`/`getMoreWidgets`; `pinGroup`/`unpinGroup` (or reuse `pinWidget`/`unpinWidget` — parent is just another key in `widget:pinned`) |
| `frontend/app/window/action-widgets.tsx` | Parent-aware click handler (toggle vs `handleWidgetSelect`); per-slot `createSubmenuHover`; peer-close across pinned parents; render `PinnedWidgetFlyout`; group-scoped right-click menu |
| `frontend/app/window/pinned-widget-flyout.tsx` *(new)* | Case A panel — modeled on `more-dropdown.tsx` |
| `frontend/app/window/more-dropdown.tsx` | Parent-row branch → nested `SubMenu`-equivalent (§3.4 option 1) |
| `frontend/app/util/submenu-hover.ts` *(maybe)* | Hoist `createPeerRegistry()` out of `flyoutmenu.tsx` if Case A needs it too (§3.3) |
| `schema/widgets.json` | Mirror the `children` field for editor/schema validation, if this file is hand-maintained JSON Schema rather than generated |

---

## 5. Open questions

1. **Hoisting `createPeerRegistry()`** — currently private to
   `flyoutmenu.tsx`. Case A (§3.3) needs the same "close my siblings"
   behavior across pinned-bar parent slots, which live outside
   `FlyoutMenu` entirely. Move it to `submenu-hover.ts` (alongside
   `createSubmenuHover`, which it already composes with) so both
   `flyoutmenu.tsx` and `action-widgets.tsx` import the same helper,
   rather than forking a second copy.
2. **Per-child "pin to bar directly"** (§2, §3.5) — deferred, but the
   moment a user asks for it, `widget:pinned` needs to accept child
   short-names as first-class entries even though they're
   `display:hidden` in `widgets.json`. `getPinnedWidgets()`'s "is this
   key in `wmap`" check already tolerates that (hidden ≠ absent from
   `wmap`) — worth confirming with a test once/if this ships, not
   blocking v1.
3. **`children` referencing a key that's also independently pinned** —
   e.g. a user's existing `widget:pinned` array already contains `slack`
   from before grouping existed. Recommend: once `slack` is marked
   `display:hidden: true` and becomes a grouped child, strip any bare
   occurrence of it from `widget:pinned` during config load (same spirit
   as the existing migration in `widget-pinning.md` §Migration) — otherwise
   a stale pinned entry could render as an orphaned bar icon for a hidden
   widget. Needs a small backend migration step alongside the `widgets.json`
   change, not just a frontend filter.
4. **Nested groups** (§2) — not building it, but confirm the recursive
   shape in `FlyoutMenu`'s `SubMenu` (§3.4 option 1 borrows its structure)
   doesn't accidentally *require* forbidding it — if `getGroupedChildKeys()`
   walks one level deep only, a nested parent-inside-parent would silently
   misbehave rather than clearly error. Add a load-time validation
   (§3.1) rejecting `children` that itself resolves to another parent,
   until nesting is an actual spec'd feature.

---

## 6. Testing

- `action-widgets-config.test.ts` (or wherever `getPinnedWidgets`/
  `getMoreWidgets` are covered today): grouped children excluded from both
  flat lists; parent itself present in exactly one of the two lists per
  its own pin state.
- Case A: hover-open respects the 90ms delay (no flash on fast mouse
  pass-through); diagonal move from icon into the panel doesn't close it
  early (safe-triangle); opening one pinned parent force-closes another
  already-open one; click toggles without requiring hover; Escape/
  outside-click closes; child click opens the right pane and closes the
  flyout; child right-click still shows the existing per-item menu while
  the flyout itself stays open (matches today's More-dropdown-stays-open-
  during-item-context-menu behavior).
- Case B: parent row shows the chevron; hover opens sideways, flips side
  near the window edge; parent submenu closes independently of outer More
  dropdown; selecting a child closes both levels.
- Responsive collapse: a pinned parent counts as one slot for width
  measurement/clipping — verify `useWidgetBarResponsive`'s tier-3 clip
  math is unaffected (no code change expected, but the parent's icon-only
  tooltip should show the group label, not attempt to enumerate children).
- Drag-reorder: dragging a pinned parent slot reorders it as a unit; no
  drop-target logic needed inside its children (they're not on the bar).
- Config migration (§5 item 3): a fixture `settings.json` with a bare
  `slack` in `widget:pinned` from before grouping existed, loaded against
  a `widgets.json` where `slack` is now a hidden grouped child — confirm
  it's stripped rather than rendering an orphaned icon.
