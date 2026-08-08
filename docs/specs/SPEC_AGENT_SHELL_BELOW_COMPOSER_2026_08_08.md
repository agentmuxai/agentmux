# SPEC: Open the agent-pane Shell drawer below the composer, not above it

**Date:** 2026-08-08
**Status:** Implemented (same-day PR; §3.2 corrected against live CDP verification before merge)
**Author:** Agent3 (agent)
**Trigger:** User request — *"when opening the Shell button in the agent
pane, we need the shell to open below (not above) the agent pane text
input."*
**Area:** Agent pane composer region (`frontend/app/view/agent/`)

---

## 1. Problem

Clicking the **Shell** toggle in the agent pane's composer strip
(`AgentComposerStrip.tsx:298-306`) opens a resizable terminal drawer
(`AgentShellSubblock` inside `ResizableDetailsDrawer`). Today it renders
**above** the composer/text-input (`AgentFooter`), pushing the input further
down the pane every time the drawer opens or is resized taller. The user
wants the opposite stacking order: the composer/text-input should stay where
it is, and the shell should open **below** it.

## 2. Root cause — plain DOM order, not an explicit placement mechanism

Unlike `AgentRuntimeDropup` (a `Portal` + `@floating-ui/dom`-positioned
floating popup with a `placement: "top-start"` option), the Shell drawer has
**no placement concept at all**. It is a normal sibling inside a
`display:flex; flex-direction:column` container, and it renders above the
composer purely because it appears *earlier* in the JSX.

`frontend/app/view/agent/agent-view.tsx:1814-1899` — current structure
inside `.agent-composer-region`:

```tsx
<div class="agent-composer-region">
    <Show when={agentAtoms().detailsOpenAtom[0]()}>
        <div class="agent-composer-details" id={`agent-composer-details-${model.blockId}`}>
            <ResizableDetailsDrawer blockId={model.blockId} persistedHeight={...}>
                <AgentShellSubblock ... />
            </ResizableDetailsDrawer>
            <AgentControlBar blockId={model.blockId} blockAtom={block} providerId={...} />
        </div>
    </Show>
    <Show when={commands.helpVisible()}>
        <SlashHelpPanel ... />
    </Show>
    <Show when={commands.pickerSpec()}>
        {(spec) => <SlashCommandPicker spec={spec()} ... />}
    </Show>
    <AgentFooter ... />                {/* the composer/text-input */}
</div>
```

`frontend/app/view/agent/styles/_control-bar.scss:409-414`:

```scss
.agent-composer-region {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
}
```

No `column-reverse`, no `position: absolute/fixed`, no anchor/placement
prop anywhere in this path. The shell drawer (`.agent-composer-details`)
is simply the first child of a normal (not reversed) flex column, so it
paints above every sibling that follows it — including `AgentFooter`.

`ResizableDetailsDrawer.tsx`'s own doc comment confirms the current
assumption is baked in, not incidental:

> "The drag handle sits on the drawer's top edge (the drawer itself is
> docked above the composer, so dragging the top edge up grows it)."

## 3. Fix

### 3.1 Reorder the JSX — move the shell drawer after `AgentFooter`

In `agent-view.tsx`, move the `<Show when={agentAtoms().detailsOpenAtom[0]()}>...</Show>`
block (currently lines 1818-1856) to **after** the `<AgentFooter .../>`
element (currently lines 1887-1898), while leaving `SlashHelpPanel` /
`SlashCommandPicker` where they are — those are composer-adjacent
autocomplete surfaces (`.slash-picker` renders as a normal in-flow block
directly above the input, `_slash.scss:9-18`) and should stay immediately
above `AgentFooter` regardless of where the shell drawer goes.

New order inside `.agent-composer-region`:

```tsx
<div class="agent-composer-region">
    <Show when={commands.helpVisible()}>
        <SlashHelpPanel ... />
    </Show>
    <Show when={commands.pickerSpec()}>
        {(spec) => <SlashCommandPicker spec={spec()} ... />}
    </Show>
    <AgentFooter ... />                {/* the composer/text-input */}
    <Show when={agentAtoms().detailsOpenAtom[0]()}>
        <div class="agent-composer-details" id={`agent-composer-details-${model.blockId}`}>
            <AgentControlBar blockId={model.blockId} blockAtom={block} providerId={...} />
            <ResizableDetailsDrawer blockId={model.blockId} persistedHeight={...}>
                <AgentShellSubblock ... />
            </ResizableDetailsDrawer>
        </div>
    </Show>
</div>
```

Note `AgentControlBar` and `ResizableDetailsDrawer` also swap places
*within* `.agent-composer-details`: `AgentControlBar` is a thin,
fixed-height strip (`_control-bar.scss:399-404`, `border-top`) meant to
read as a "sub-footer" adjacent to whatever's above it. With the drawer now
below the composer, `AgentControlBar` should sit directly under
`AgentFooter` (immediately below the text input, as it conceptually already
does today — it's still adjacent to the composer, just now the drawer
trails it instead of leading it) and the terminal fills the remaining space
below that. This also keeps `AgentControlBar`'s `border-top` reading
correctly as a separator from the composer above it, not from the terminal.

No change needed to `.agent-composer-region`'s CSS
(`display:flex; flex-direction:column` still does the right thing under the
new order) or to any `Show` condition — this is a pure JSX reorder.

### 3.2 Resize handle: keep it on the drawer's TOP edge, keep the drag math

An earlier draft of this spec proposed moving the handle to the drawer's
bottom edge and flipping the drag sign ("drag down to grow"). **Live
verification against a `task dev` build showed that's wrong**, because of a
geometry fact the draft missed: `.agent-composer-region` is bottom-anchored
in the pane (the document scroll area above it flexes; the region hugs the
pane bottom). So with the drawer docked below the composer, the drawer's
**bottom edge is pinned to the pane bottom** — when its height changes, the
edge that actually moves is its **top** edge, pushing the composer up.
Measured concretely via CDP on the running build: a bottom-edge handle sat
~30px above the window bottom and stayed pinned there while the drawer grew
upward on its other side — the handle didn't track the cursor, and a real
user would run out of downward mouse travel almost immediately.

The correct arrangement is the standard IDE bottom-panel pattern (VS Code's
terminal): **handle on the drawer's top edge, drag UP to grow** — which is
exactly the existing handle position and existing drag math
(`onPointerMove`: `delta = dragStartY - ev.clientY`). The handle is the
drawer's one free edge, so it tracks the cursor 1:1 with full upward travel
range.

**Net change to `ResizableDetailsDrawer.tsx`: comments only.** The handle
JSX stays first (top edge), the drag math stays as-is; only the doc comment
changes, to explain the docked-below-composer / pinned-bottom-edge geometry
instead of the old "docked above the composer" description. No SCSS changes
(`border-top` on the handle remains the correct free-edge accent).

### 3.3 Nothing else references drawer placement

Confirmed no other file assumes "shell above composer":
- `AgentComposerStrip.tsx`'s Shell button (`:298-306`) only toggles
  `detailsOpenAtom` via `onToggleLog` — no positional assumption.
- `AgentShellSubblock.tsx` and `AgentControlBar.tsx` render into whatever
  container places them; neither reads or assumes DOM position.
- `term:shellheight` persistence (`ResizableDetailsDrawer.tsx:53-56`) stores
  a plain height number — direction-agnostic, no change needed.

## 4. Out of scope

- No change to `AgentRuntimeDropup` or any other floating/portaled popup —
  this spec only concerns the in-flow Shell drawer.
- No change to the Shell button's icon, label, or toggle behavior.
- Not introducing a general "placement" prop/system for the drawer (unlike
  `AgentRuntimeDropup`, this panel has exactly one placement and no product
  need for a configurable one) — this is a fixed reorder, not new
  infrastructure.

## 5. Test plan

Note on expectations: because the composer region is bottom-anchored,
opening the shell below the composer necessarily shifts the composer *up*
by the drawer's height (the shell can't extend past the pane bottom).
"Below the input" is about stacking order, not about the input staying
frozen in place.

- [x] Open an agent pane, click **Shell** — drawer appears below the
      composer/text-input (verified live via CDP: footer spans y 901-930,
      drawer starts at exactly y 930; DOM order in
      `.agent-composer-region` is `agent-footer` then
      `agent-composer-details`).
- [x] Drag the resize handle up — drawer grows (bottom edge pinned, top
      edge follows the cursor); drag down shrinks, clamped at `MIN_HEIGHT`
      (120px) / `MAX_HEIGHT` (600px) same as today (drag math verified live
      via synthesized pointer events: height responds 1:1 to drag delta).
- [ ] Close and reopen the pane (or reload) — persisted `term:shellheight`
      still restores the same height.
- [x] Slash-command help/picker (`/` at start of composer) still renders
      directly above the composer, unaffected by the drawer's new position
      (unchanged DOM position before `AgentFooter`).
- [x] `AgentControlBar` (cwd/copy/clear controls) still reads visually as
      attached to the composer, now sitting between `AgentFooter` and the
      terminal.

## 6. Files to change

| File | Change |
|------|--------|
| `frontend/app/view/agent/agent-view.tsx` | Move the `detailsOpenAtom` `Show` block to after `AgentFooter`; swap `AgentControlBar`/`ResizableDetailsDrawer` order within it (§3.1) |
| `frontend/app/view/agent/components/ResizableDetailsDrawer.tsx` | Doc-comment update only — handle stays on the top edge, drag math unchanged (§3.2) |
