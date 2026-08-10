# SPEC: Explicit close button on the Runtime (Mode/Model/Effort) dropup

**Date:** 2026-08-07
**Status:** implemented — PR #2460; verified in code 2026-08-10.
**Author:** AgentX (agent)
**Trigger:** User request — *"when changing the model panel, we dont want it
to auto close after selection, because sometimes the user wants to change
multiple entries. instead add a close button and also clicking away will
close it too."*
**Amends:** `docs/specs/SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md` §5, §9.2,
§11 — read that spec first. This one only adds what's missing; it does not
re-litigate anything already decided there.

---

## 0. Scope check against current `main` — two of three asks are already shipped

Before scoping new work, I verified the panel's current behavior against
`frontend/app/view/agent/components/AgentRuntimeDropup.tsx` on `main`
directly (not just the base spec's intent):

| User ask | Current state | Evidence |
|---|---|---|
| Don't auto-close after selecting a value | **Already true.** `applySelection` (`AgentRuntimeDropup.tsx:166-173`) calls `updateRuntime(...)` and never calls `setOpen(false)`. A code comment directly above it (`:163-165`) states this is deliberate, citing the base spec's §9.2. | `AgentRuntimeDropup.tsx:163-173` |
| Clicking away closes it | **Already true.** `handleClickOutside` (`:213-217`) calls `setOpen(false)` on any `mousedown` whose target is outside both the trigger and the panel, wired for the panel's open lifetime (`:231-241`). | `AgentRuntimeDropup.tsx:213-217, 231-241` |
| An explicit close button | **Missing.** No button with a close affordance exists anywhere in the panel's JSX (`:293-339`). The only ways to close today are `Esc`, outside-click, re-clicking the trigger, or focus leaving the panel by any means (`handleFocusChange`, `:224-226` — see §4 below, this is a fourth, *implicit* close path worth knowing about even though it's not part of this ask). | `AgentRuntimeDropup.tsx:293-339` |

**This spec's actual scope is narrow: add a close button.** If you're seeing
close-on-select in a running build, that build predates the base spec
shipping (or predates commit `9f86d917`'s three-pill precursor) — worth
confirming the build version before assuming there's a regression to chase
down separately from this spec.

---

## 1. Why a close button, given outside-click and Esc already work

Outside-click and `Esc` are real, working close paths — but neither is
*discoverable* the way a visible affordance is, and this panel is unusual
among the app's popups in an important way: **it deliberately stays open
across multiple interactions** (§9.2 of the base spec), unlike every
single-value picker in the app (`FlyoutMenu`, `SlashCommandPicker`, native
context menus) that closes itself the instant you're done. A user who
hasn't internalized "this one's different, it waits for you" may not think
to reach for `Esc` or click elsewhere — an explicit, visible close button
removes that ambiguity and matches the one first-class precedent already in
the codebase for "stays open, dismiss when you're done" panels: `Modal`'s
`showCloseButton` (`frontend/app/element/modal.tsx:416-426`,
`.modal-panel-close-btn` in `frontend/app/element/modal.scss:164-190`).

No non-modal popover in this codebase has a close button today
(`HostPopover`, `TokenBreakdownPopover`, generic `PopoverMenu` all rely on
outside-click/Esc alone) — this would be the first. That's fine: this panel
is also the first non-modal popover in the app designed to stay open across
multiple selections, so it's the first one that actually needs one.

---

## 2. Design

### 2.1 Placement and visual treatment

A small `✕` icon button, absolutely positioned in the panel's top-right
corner — adapted from `Modal`'s pattern but scaled down to match this
panel's much more compact chrome (10px base font, `min-width: 180px`,
2px-scale row padding — see `_composer-strip.scss:151-159` — vs. Modal's
28px button meant for a full-size dialog).

```
┌─ mode ────────────────────────╮✕┐
│  ✓ Bypass (no prompts)         │
│    Auto (AI classifier)        │
│    ...                         │
├─ model ────────────────────────┤
│    ...                         │
└─────────────────────────────────┘
```

Proposed CSS (new rule, `_composer-strip.scss`, near the existing
`.agent-runtime-dropup-*` block):

```scss
.agent-runtime-dropup-close-btn {
    position: absolute;
    top: 2px;
    right: 2px;
    width: 16px;
    height: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-sm, 4px);
    color: var(--secondary-text-color);
    font-size: 9px;
    line-height: 1;
    cursor: default;
    transition: all var(--motion-fast);

    &:hover {
        color: var(--main-text-color);
        background: color-mix(in srgb, var(--main-text-color) 8%, transparent);
        border-color: var(--border-color);
    }

    &:focus-visible {
        outline: none;
        box-shadow: var(--shadow-focus-ring);
    }
}
```

Same interaction states as `Modal`'s button (hover tint, focus ring), same
`✕` glyph, just re-scaled — no new visual language introduced.

**Overlap check (implementation must verify visually):** the panel's
`min-width` is only 180px and section headers/rows are left-aligned but can
run close to the right edge (e.g. a long model label, or the
`.agent-runtime-dropup-description` text — `_composer-strip.scss:177-185`).
A 16×16px button inset 2px from the top-right corner is small, but confirm
in `task dev` that it doesn't visually collide with the top section header's
text or a `current`-row's check icon. If it does, the fix is a few px of
`padding-right` on `.menu.agent-runtime-dropup-panel` or on
`.agent-runtime-dropup-section`, not a redesign.

### 2.2 DOM structure — and why it can't just be prepended like `Modal`'s button

`Modal`'s close button is simply the first child of `.modal-panel`
(`modal.tsx:416-426`, before `{props.children}`) because `.modal-panel` has
no ARIA role of its own. **This panel is different**: the outer element
currently carries `role="listbox"` directly
(`AgentRuntimeDropup.tsx:297-301`), and every row is `role="option"`
(`:316`). Per the ARIA authoring practices for the listbox pattern, a
`role="listbox"` element's children should be its options (and an optional
label) — an interactive `<button>` dropped in as a sibling of the option
rows, inside the same `role="listbox"` container, is a structurally invalid
listbox and would confuse screen-reader listbox navigation (arrow-key
option traversal, `aria-activedescendant`-style expectations, etc.).

**Fix: split the single `role="listbox"` div into two nested elements.**
The outer element keeps the `ref`/positioning/`data-pane-overlay` (unchanged
by this spec) but drops `role="listbox"`; a new inner wrapper around just
the `<For>` rows carries `role="listbox"` and `aria-label="Runtime
settings"` instead. The close button becomes a sibling of that inner
wrapper — both children of the same outer positioned div — so
`floatingEl.contains(closeButton)` still holds true for `handleClickOutside`
and `handleFocusChange`'s containment checks (§2.3), and `computeMenuPosition`
/`autoUpdate` still measure the correct outer boundary.

```tsx
// Before (AgentRuntimeDropup.tsx:293-339, abbreviated):
<Show when={open()}>
    <Portal>
        <div ref={registerFloating} class="menu agent-runtime-dropup-panel"
             style={floatingStyle()} data-pane-overlay
             role="listbox" aria-label="Runtime settings">
            <For each={build().rows}>{...}</For>
        </div>
    </Portal>
</Show>

// After:
<Show when={open()}>
    <Portal>
        <div ref={registerFloating} class="menu agent-runtime-dropup-panel"
             style={floatingStyle()} data-pane-overlay>
            <button type="button" class="agent-runtime-dropup-close-btn"
                    aria-label="Close" onClick={() => setOpen(false)}>
                {"✕"}
            </button>
            <div role="listbox" aria-label="Runtime settings">
                <For each={build().rows}>{...}</For>
            </div>
        </div>
    </Portal>
</Show>
```

The inner `role="listbox"` wrapper needs no new class/styling of its own —
it's a plain `<div>`; all existing `.agent-runtime-dropup-*` rules keep
targeting the same descendants they already do, since none of them depend
on the removed div being the *direct* listbox-role holder, only on being
inside `.agent-runtime-dropup-panel`.

### 2.3 Behavior — purely additive, no existing close path changes

- `onClick={() => setOpen(false)}` — same direct call every other close path
  already uses (`handleKeyDown`'s `Escape` branch, `handleClickOutside`,
  `handleFocusChange`, `toggleOpen`'s re-click branch — all just call
  `setOpen(false)`). No new state, no new close-reason tracking needed.
- **No conflict with `handleClickOutside`:** the button is a DOM descendant
  of `floatingEl` (the div `registerFloating` refs), so a `mousedown` on it
  is caught by the `floatingEl?.contains(target)` check (`:215`) and
  `handleClickOutside` is a no-op for that event — the button's own
  `onClick` is what closes the panel, not the outside-click listener firing
  redundantly.
- **No conflict with `handleFocusChange`:** clicking the button does move
  DOM focus onto it, which is still "inside" per `focusWithinDropup()`
  (`:183-187`, checks `floatingEl?.contains(active)`) — so this handler
  doesn't fire mid-click. Once `setOpen(false)` runs and the `<Show>` tears
  the panel down, the `createEffect` at `:231-241` cleans up all three
  document listeners via its `onCleanup`, same teardown path every other
  close reason already goes through.
- **Keyboard reachability:** the button is a real `<button>`, so it's in the
  native Tab order — placing it as the *first* child (matching `Modal`'s
  convention exactly, §2.2) means it's the first Tab stop after the trigger,
  before any option row. It is **not** part of `build().options` — the
  arrow-key/letter-jump navigation model (`move`, `handleKeyDown`'s
  letter-jump branch) is unchanged and continues to only walk option rows.
  `Esc` remains the fast keyboard-only close path; the button is a
  mouse/discoverability affordance, same division of labor `Modal` already
  has (`Esc` closes a modal too, independent of its close button).

### 2.4 Alternative considered and rejected: a header row instead of a floating corner button

A full-width header row (`Runtime` title text + right-aligned `✕`) was
considered, matching how some apps title every popover. **Rejected:** the
base spec was explicit that "Runtime" as a word appears only in the trigger's
`aria-label` and in docs — never as visible panel chrome (§2 of the base
spec) — adding a visible title now would be scope creep beyond what was
asked (a close button, not a redesign), and would add a full extra row of
vertical space to a panel whose entire design language up to now is
deliberately tight (2px row padding, 9-10px fonts). The corner-button
approach adds a close affordance with zero extra vertical space and zero new
visible text.

---

## 3. Edge cases

| Case | Behavior |
|---|---|
| Panel open, user clicks the close button | Closes immediately via the same `setOpen(false)` every other path uses — no different from `Esc`. |
| Panel open, user selects a value, then clicks close | Selection already applied via `applySelection` (unchanged §9.2 behavior) before the close click is a separate, later event — no ordering issue since these are two independent user actions, not one combined gesture. |
| Keyboard-only user | `Esc` remains the primary close path (unchanged); the button is additionally Tab-reachable and `Enter`/`Space`-activatable as a native `<button>`, so it's not mouse-only, just not part of the arrow-key option list (§2.3). |
| Screen reader | Panel's listbox semantics stay valid post-restructure (§2.2) — the button sits outside the `role="listbox"` subtree, announced as an ordinary button ("Close, button"), not as a spurious listbox option. |
| Narrow viewport | No interaction with the existing `_composer-strip.scss` container-query breakpoints (§3.4 of the base spec) — this only changes the panel's own internal layout, not the trigger/composer-strip sizing rules. |

---

## 4. Related but out of scope

`handleFocusChange` (`AgentRuntimeDropup.tsx:224-226`) closes the panel
whenever DOM focus moves outside it **by any means**, not just a click —
e.g. a programmatic `.focus()` call elsewhere in the app (the file's own
comment cites `AgentFooter`'s `acceptCompletion()` as a real example,
`:176-182`). This is a fourth, already-existing close trigger, broader than
literal "clicking away." It isn't part of what was asked (which named
clicking away and an explicit button specifically) and changing it isn't
implied by this request — flagged here only so whoever implements this
knows it exists and isn't surprised by it during testing (e.g. tabbing out
of the panel closes it even without a click, independent of this spec's
changes).

---

## 5. Testing

**No test file exists for this component today**
(`AgentRuntimeDropup.test.tsx` does not exist; confirmed via
`grep -rl "AgentRuntimeDropup" frontend --include="*.test.*"` returning no
hits). This spec's implementation should add one, not extend one. Minimum
coverage, following this codebase's `@solidjs/testing-library` conventions
(e.g. `DocumentRow.test.tsx`, `AgentDocumentVirtualList.resize.test.tsx`):

1. Selecting an option does **not** close the panel (regression guard for
   the already-shipped §9.2 behavior — currently has zero coverage of its
   own, worth locking in while touching this file).
2. Clicking the new close button closes the panel (`open` signal reflected
   via the trigger's `aria-expanded` or the panel's presence in the DOM).
3. Click-outside still closes the panel (existing behavior, currently
   untested — worth covering in the same pass).
4. The close button is not present with `role="option"`/inside the
   `role="listbox"` subtree (structural assertion for §2.2's restructure).
5. Clicking the close button does not double-fire — i.e. `setOpen` is
   called in a way that's idempotent/harmless even if `handleClickOutside`
   also runs for the same event (defensive test, since §2.3 argues this
   can't happen, but the interaction is subtle enough to be worth pinning
   down explicitly rather than trusting the reasoning alone).

---

## 6. Acceptance criteria

1. The panel has a visible `✕` close button in its top-right corner,
   present whenever the panel is open.
2. Clicking it closes the panel via the same `setOpen(false)` path every
   other close reason already uses — no new close-state machinery.
3. Selecting a Mode/Model/Effort value continues to leave the panel open
   (already true on `main` — this spec must not regress it).
4. Click-outside and `Esc` continue to close the panel (already true on
   `main` — this spec must not regress either).
5. The panel's listbox semantics remain valid post-change: `role="listbox"`
   wraps only the option rows, not the close button.
6. New test coverage exists for this component (§5) — this file currently
   has none.

---

## 7. Files this change touches

```
# Modified
frontend/app/view/agent/components/AgentRuntimeDropup.tsx    add close button + role="listbox" restructure (§2.2, §2.3)
frontend/app/view/agent/styles/_composer-strip.scss          add .agent-runtime-dropup-close-btn (§2.1)

# New
frontend/app/view/agent/components/AgentRuntimeDropup.test.tsx   new — no prior coverage (§5)

# Unchanged (reused, not modified)
docs/specs/SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md            §9.2's "stays open" decision is amended-by, not replaced-by, this spec
frontend/app/view/agent/runtime-apply.ts                      applyRuntimeChange — no changes needed
frontend/app/element/modal.tsx / modal.scss                   close-button pattern referenced, not imported — this panel isn't a Modal
```

---

*End of spec. Ready for review + go/no-go decision.*
