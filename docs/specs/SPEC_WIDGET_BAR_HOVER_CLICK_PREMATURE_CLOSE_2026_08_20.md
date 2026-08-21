# SPEC: Widget bar "More" / pinned-parent flyout closes on its first click when hover already opened it

**Date:** 2026-08-20
**Status:** proposed

---

## 0. Ask

Reported symptom: hovering over the `···More` button, or a pinned parent
widget (e.g. "Messengers"), opens its dropdown/flyout after the usual
90ms hover-intent delay (`SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md`).
A user who then clicks the trigger — the ordinary "move the mouse to the
button and click it" gesture, not a deliberate second interaction —
immediately closes the menu that had just opened. From the user's
perspective this is their *first* click, but the menu behaves as if it
were the second click of an open/close toggle.

Desired behavior: the first click on a trigger whose menu is already open
**because of hover**, not because of a prior click, should keep the menu
open. Only a click on a menu that is open **because the user already
clicked it** should close it — i.e. the toggle-closed behavior should
require two clicks when the open was hover-initiated, not one.

---

## 1. Root cause

Two call sites in `frontend/app/window/action-widgets.tsx` wire a trigger's
`onClick` as a plain open/closed toggle keyed off the same boolean the
hover-intent controller (`createSubmenuHover`,
`frontend/app/util/submenu-hover.ts`) already uses to track "is this
menu currently visible":

**The "More" button** (`action-widgets.tsx`, `openMore`, ~line 249):

```ts
const openMore = (_e: MouseEvent) => {
    if (moreOpen()) {
        moreHover.close();
        return;
    }
    parentPeers.closeOthers(MORE_PEER_KEY);
    moreHover.openNow();
};
```

**A pinned parent widget** (e.g. Messengers) (`action-widgets.tsx`,
`handleParentSlotClick`, ~line 186):

```ts
const handleParentSlotClick = (key: string) => {
    if (openParentKey() === key) {
        closeParentFlyout(key);
        return;
    }
    parentPeers.closeOthers(key);
    parentHoverControllers.get(key)?.openNow();
};
```

Both read `moreOpen()` / `openParentKey() === key` — a signal that is
`true` whenever the menu is visible, **regardless of how it got that way**.
`createSubmenuHover`'s own `onTriggerEnter()` (wired to the trigger's
`onMouseEnter`, `action-widgets.tsx` ~line 488 and ~line 463) already flips
this same signal true after a 90ms hover delay, with no click involved:

```ts
onTriggerEnter() {
    cancelPendingClose();
    if (isOpen || openTimer !== null) return;
    openTimer = setTimeout(() => {
        openTimer = null;
        isOpen = true;
        opts.onOpen();   // → setMoreOpen(true) / setOpenParentKey(key)
    }, openDelayMs);
},
```

So the sequence that reproduces the bug is:

1. Cursor enters the trigger → `onTriggerEnter()` fires.
2. ~90ms later (well within normal mouse-to-click travel time) the open
   timer fires → `moreOpen()` (or `openParentKey()`) becomes `true`.
3. User clicks the trigger (their first and only click) → `openMore`/
   `handleParentSlotClick` runs, sees the now-`true` open signal, and
   takes the "already open → close it" branch.

The click handler has no way to tell "open because of an already-completed
hover" apart from "open because the user clicked it a moment ago and this
is their intentional second click" — both look identical from the
handler's point of view, since both just read the shared `isOpen`-derived
signal.

This is a **general property of the shared toggle-on-click pattern layered
on top of hover-intent**, not a bug specific to Messengers or More — any
future trigger that gets both `onMouseEnter`-driven hover-intent (via
`createSubmenuHover`) and a click-to-toggle handler over the same open
signal will reproduce it. Today that's exactly these two call sites; the
child rows nested inside the More dropdown for a parent widget
(`more-dropdown.tsx` ~line 201) are hover-only (no `onClick` on that row at
all), so they're unaffected.

---

## 2. Proposed fix

Track, per trigger, whether the *current* open state was **confirmed by a
click** — separate from whether it's merely open. A click:

- On a **closed** trigger → opens it (via `openNow()`, as today) and marks
  it click-confirmed.
- On a trigger that is open but **not yet click-confirmed** (i.e. open
  purely from hover) → does **not** close it. Instead it marks the current
  open state as click-confirmed and returns — this is the fix for the
  reported symptom.
- On a trigger that is open **and already click-confirmed** → closes it,
  exactly like today's toggle.

The confirmed flag must reset to `false` whenever the menu closes through
*any* path (hover-away safe-triangle timeout, outside click, Escape, a
peer forcing it closed) — otherwise a stale `true` from a previous open
session could make some future hover-open look pre-confirmed. The cleanest
place for that reset is each controller's own `onClose` callback, since
every close path in `submenu-hover.ts` already funnels through `doClose()`
→ `opts.onClose()`.

### 2.1 More button

```ts
const [moreConfirmedByClick, setMoreConfirmedByClick] = createSignal(false);

const moreHover = createSubmenuHover({
    onOpen: () => setMoreOpen(true),
    onClose: () => {
        setMoreOpen(false);
        setMoreConfirmedByClick(false);
    },
});

const openMore = (_e: MouseEvent) => {
    if (moreOpen()) {
        if (!moreConfirmedByClick()) {
            // Opened by hover; this is the user's first real click on it —
            // keep it open, but arm the toggle so the *next* click closes it.
            setMoreConfirmedByClick(true);
            return;
        }
        moreHover.close();
        return;
    }
    parentPeers.closeOthers(MORE_PEER_KEY);
    moreHover.openNow();
    setMoreConfirmedByClick(true);
};
```

### 2.2 Pinned parent widgets (Messengers, and any future parent)

Only one pinned-parent flyout can be open at a time (`openParentKey` is a
single scalar, peers force-close each other) — a single non-reactive flag
alongside it is enough, no per-key map needed:

```ts
let parentOpenConfirmedByClick = false;

const registerParentHover = (key: string): SubmenuHoverController => {
    const existing = parentHoverControllers.get(key);
    if (existing) return existing;
    const hover = createSubmenuHover({
        onOpen: () => setOpenParentKey(key),
        onClose: () => {
            setOpenParentKey((cur) => (cur === key ? null : cur));
            parentOpenConfirmedByClick = false;
        },
    });
    parentHoverControllers.set(key, hover);
    parentPeers.register(key, hover);
    return hover;
};

const handleParentSlotClick = (key: string) => {
    if (openParentKey() === key) {
        if (!parentOpenConfirmedByClick) {
            parentOpenConfirmedByClick = true;
            return;
        }
        closeParentFlyout(key);
        return;
    }
    parentPeers.closeOthers(key);
    parentHoverControllers.get(key)?.openNow();
    parentOpenConfirmedByClick = true;
};
```

`closeParentFlyout` (used by the outside-click/Escape effect and by
`MoreDropdown`'s Case B `onClose`) already routes through the controller's
`close()` → `doClose()` → `onClose` callback, so it resets the flag for
free — no separate call needed there.

### 2.3 Why not fold this into `submenu-hover.ts` itself

The "confirmed by click" concept only applies to the two trigger-level call
sites that pair `createSubmenuHover` with a click-to-toggle handler; the
module itself has no notion of clicks at all today (`openNow()` is
generic — FlyoutMenu's own `SubMenu` never calls it, for instance). Adding
click semantics to the shared core would couple it to one specific calling
pattern. Keeping the flag local to each call site (two small, near-
identical blocks) is more consistent with how `action-widgets.tsx` already
duplicates the peer-registration/open-tracking shape between the More
button and pinned parents rather than generalizing it prematurely.

---

## 3. Scope / out of scope

**In scope:** the two click handlers above (`openMore`,
`handleParentSlotClick`) in `frontend/app/window/action-widgets.tsx`.

**Out of scope:**
- `more-dropdown.tsx`'s nested Case B parent rows (Messengers shown
  *inside* More rather than pinned) — hover-only today, no `onClick`, so
  the toggle-close bug cannot occur there. No change needed.
- `flyoutmenu.tsx`'s `SubMenu` (right-click context-menu submenus) — those
  open purely via hover-intent, never via a click-to-toggle handler, so
  the same conflation doesn't exist there either.
- The 90ms open delay / 300ms safe-triangle timing themselves
  (`SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_2026_08_10.md`) — unaffected
  by this fix.

---

## 4. Testing

- Hover the More button until it opens (or wait past 90ms), then click it
  once → stays open. Click it again → closes.
- Hover Messengers until its flyout opens, then click it once → stays
  open. Click again → closes.
- Click More (or a pinned parent) with **no** prior hover (e.g. keyboard
  focus + Enter, or a fast click before the 90ms delay elapses) → still
  opens on the first click, exactly as today (this path never sets the
  confirmed flag from hover, so the very first click's "closed → open"
  branch already marks it confirmed = no regression).
- Open More by click, move the cursor onto a pinned parent (peer-close
  fires, closing More) — confirm `moreConfirmedByClick` resets, so hovering
  back onto More and clicking it doesn't immediately close it again.
- Outside-click and Escape still close an open menu regardless of the
  confirmed flag (those paths don't go through the click handlers at all).
