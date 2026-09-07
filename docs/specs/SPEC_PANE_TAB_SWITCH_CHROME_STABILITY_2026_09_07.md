# SPEC: Switching in-pane tabs must not repaint the pane header or tab bar

**Date:** 2026-09-07
**Status:** proposed — the analysis and the staged plan are the deliverable;
the refactor itself (§5) is **not** implemented. Only §8's documentation fix
ships with this spec. §2.2 is why: every consumer in the `<Block>`/`BlockFrame`
tree is built on "this component instance owns this blockId for its lifetime",
including `useWaveObjectValue(oref: string)`, which takes a plain string and
refcounts it in `onCleanup` — it does not re-subscribe when a blockId changes.
Making the pane header survive a block switch means converting that whole tree
to reactive block identity, which lands on every pane type at once. That needs
to be a deliberate, separately-verified piece of work, not a rider on a spec.
**Related:** `docs/specs/SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md`
(added the reveal gate this spec proposes to make unnecessary),
`docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md` §4.3 (the
block-stack mechanism whose remount design is the root cause),
`docs/specs/SPEC_TERM_PANE_TAB_STRIP_TRAILING_BLUR_2026_09_07.md` (same
surface, written the same day — see §7 for how they interact)

---

## 0. The ask

> when you move between tabs, the entire pane repaints, including the pane
> header. We don't want that, instead we want only the actual terminal to
> repaint, the pane header and tab bar should not flinch at all.

The observation is correct, and the current behavior is not incidental — the
whole pane is *deliberately* hidden and re-revealed on every in-pane tab
switch. This spec is about removing the reason that was necessary.

---

## 1. What actually happens today, end to end

Switching a terminal tab runs this chain:

1. **`term.tsx:134` `handleTermTabSwitch`** calls `holdLeafRevealGate(node.id)`,
   then `setActiveBlockInStack(...)`, then `scheduleLeafRevealLift(...)`.
2. **`layoutStack.ts:87-91` `setActiveBlockInStack`** sets the new active
   member and then evicts the cached node model:
   ```ts
   setActive(node.data, blockId, stack);
   model.nodeModels.delete(nodeId);   // ← forces the leaf to rebuild
   ```
3. **`layoutNodeModels.ts:142` `activeKeyFor(node)`** returns
   `` `${node.id}:${node.data.activeBlockId}` `` — so the key *changes*.
4. **`tilelayout-shared.tsx:171` / `:354`** render leaves inside
   `<Key each={leafs()} by={activeKeyFor}>`. A changed key tears the leaf's
   entire subtree down and rebuilds it.
5. That subtree is **everything**: `tabcontent.tsx:42`'s `renderContent`
   returns `<Block nodeModel={nodeModel} />`, and `<Block>` is the block
   frame — **pane header included** — wrapping the view, which for a terminal
   is `term.tsx`, which itself contains `<PaneTabStrip>`.
6. Because that rebuild is visibly ugly, `TileLayout.core.tsx:553-559` hides
   the **whole tile** while it settles:
   ```ts
   const isRevealGated = () => gatingNodeIds().has(props.node.id);
   visibility: isRevealGated() ? "hidden" : undefined,
   opacity:    isRevealGated() ? 0 : 1,
   transition: "opacity 120ms ease-out",
   ```

So the "flinch" is the pane header and tab bar being **unmounted, rebuilt,
hidden, and faded back in** — up to a 120ms opacity ramp — on every tab
switch. Nothing about that is a paint-performance problem to be tuned; it is
the designed behavior of the current mechanism.

---

## 2. Why it was built this way (do not "just remove the eviction")

### 2.1 The eviction is load-bearing

`layoutStack.ts`'s header comment is explicit that this is not incidental:

> Every mutation here evicts the target node's cached `NodeModel`
> (`model.nodeModels.delete(nodeId)`). **This is required, not optional:**
> `NodeModel.blockId` is captured once at construction time … so switching
> the active block within a stack works by forcing a remount, not by
> reactively updating a live component in place.

And `layoutNodeModels.ts:26` captures it non-reactively on purpose:

```ts
const blockId = node.data.activeBlockId || node.data.blockId;
```

### 2.2 The constraint underneath it

The real constraint is the **`ViewModel` contract**: *one instance, one
immutable `blockId` for its lifetime* (`frontend/app/block/block.tsx`).
`TermViewModel`/`AgentViewModel` are built around a fixed block. Pointing a
live view at a different block is not supported and is not what this spec
proposes.

**So a view remount on tab switch is correct and stays.** What is *not*
justified is that the pane's chrome — the block frame header and the tab
strip — is inside the same remount boundary as the view. Those are properties
of the **leaf**, not of the block, and they are exactly what the user is
watching flinch.

---

## 3. The actual defect, stated precisely

> The remount boundary is drawn around the whole leaf (`<Block>` + frame +
> tab strip + view) when it only needs to be drawn around the block's view.

Everything else here — the reveal gate, the 120ms fade, the whole-tile
`visibility: hidden` — is compensation for that boundary being too wide.

---

## 4. Options

### 4.1 Option A — narrow the reveal gate to the content region only

Keep the remount; stop hiding the header/strip while it happens: apply the
`visibility/opacity` gating to an inner content wrapper instead of `.tile-node`.

- **Cost:** small, contained to `TileLayout.core.tsx` + a wrapper element.
- **Problem:** the header is still *unmounted and rebuilt* — it just isn't
  hidden while it happens. Whether that reads as "no flinch" depends on
  whether the rebuilt header paints identically within one frame. It very
  likely still flickers (fresh DOM, re-resolved icons/titles, re-measured
  layout), and it would reintroduce exactly the flash
  `SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md` added the gate to hide.
- **Verdict:** not recommended as an endpoint. Might be an acceptable interim
  only if measurement (§6) shows the rebuilt chrome is genuinely
  indistinguishable, which should be *proven*, not assumed.

### 4.2 Option B — move the remount boundary inside the chrome (recommended)

Restructure the leaf so the per-block remount happens *below* the pane chrome:

```
.tile-leaf                       ← stays mounted (keyed on node.id only)
  └─ Block frame / pane header   ← stays mounted, reads active block reactively
      └─ PaneTabStrip            ← stays mounted, hoisted to leaf level
          └─ <Key by=activeBlockId>   ← the ONLY remount boundary
                └─ block view (TermView / AgentView + its ViewModel)
```

Required changes:

1. **Key on `node.id` again at the leaf level** (`tilelayout-shared.tsx`),
   moving the `activeBlockId` component of the key inward to wrap only the
   view. `activeKeyFor` either moves or loses its outer use.
2. **`NodeModel` stops being evicted on switch** (`layoutStack.ts:88`), and
   the active blockId becomes a **reactive accessor** on the NodeModel rather
   than a value captured at construction (`layoutNodeModels.ts:26`). The
   *ViewModel* contract in §2.2 is preserved: a new ViewModel is still built
   per block, at the new inner boundary.
3. **Hoist the tab strip out of the per-block view.** This is the part the
   ask actually hinges on and the largest piece of work:
   - terminal: `<PaneTabStrip>` currently lives in `term.tsx:559`, inside the
     per-block view;
   - agent: same, in `agent-view.tsx`'s `AgentViewWrapper`.
   Both must move up to leaf-level chrome, with their handlers
   (`handleTermTabSwitch`, `onAdd`, rename, close) going with them. Note the
   strip is already *conceptually* leaf-scoped — it lists the leaf's stack —
   so this is aligning the tree with what it already models.
4. **Pane header reads the active block reactively** so title/icon/status
   update on switch without remounting.
5. **Delete the leaf reveal gate** for the switch path once nothing remounts
   above the view — including `holdLeafRevealGate`/`scheduleLeafRevealLift`
   at `term.tsx:143-145` and the agent-pane analog. Keep it for genuine
   whole-leaf rebuilds (Quick Fork, Agent History) if any remain.

- **Cost:** real. Touches layout keying, the NodeModel lifecycle, and both
  pane types. Needs care around focus restoration and xterm refit.
- **Payoff:** the ask, exactly — only the terminal view repaints; header and
  tab bar never unmount, so they cannot flinch. It also deletes a whole
  compensating mechanism rather than adding another.

### 4.3 Option C — accept current behavior

On the record as rejected by the ask, but worth stating: the current design is
*coherent*, just wide. If Option B is judged too invasive right now, Option C
is more honest than Option A's half-measure.

---

## 5. Recommendation

**Option B**, sequenced so it can be abandoned safely partway:

1. **Step 1 — hoist the tab strip to leaf chrome (terminal first).** Visible
   win on its own: the tab bar stops being rebuilt with the view. Keeps the
   existing eviction/remount for everything else, so it is independently
   revertible.
2. **Step 2 — make the header reactive and stop evicting the NodeModel**, key
   only the view.
3. **Step 3 — remove the reveal gate** from the switch path and its now-dead
   plumbing.

Terminal first (per the ask), agent pane second, once the shape is proven.

---

## 6. Acceptance criteria — measured, not eyeballed

"Doesn't flinch" must not be judged by watching it. Required evidence:

1. **DOM identity across a switch.** Capture the header element and the tab
   strip element before the switch and assert they are the *same node
   instances* afterwards (`el === elAfter`), not merely similar. This is the
   criterion that actually distinguishes Option B from Option A.
2. **No reveal gating on the switch path.** `gatingNodeIds()` must not
   contain the node at any point during an in-pane tab switch.
3. **Terminal content still fully remounts** and attaches to the right block:
   correct scrollback, correct cwd, `stty size` sane, no stale PTY binding.
4. **Focus lands in the newly-activated terminal** after the switch, as today.
5. **No regression in the flicker cases the gate was added for**
   (`SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md`): "+" new tab, Quick
   Fork, Agent History — each still settles without a visible flash.
6. Unit coverage for the new keying/eviction behavior in the `layoutStack` /
   `layoutNodeModels` tests, including that switching no longer evicts.

Criteria 1 and 2 are the ones that prove the ask was met; the rest are
guardrails against buying it with a regression elsewhere.

---

## 7. Interaction with the terminal blur spec

`SPEC_TERM_PANE_TAB_STRIP_TRAILING_BLUR_2026_09_07.md` proposes styling the
terminal strip. If that lands first, its `.view-term > .pane-tab-strip`
selector assumes the strip is a child of the terminal view — which Step 1
here deliberately breaks by hoisting the strip to leaf chrome. Whichever
lands second must update the other's selector. Cheapest ordering is **this
spec's Step 1 first**, then the blur spec against the final DOM position.

---

## 8. Documentation drift found while writing this

`layoutStack.ts:26-28` said the remount is driven by the key function in
`TileLayout.{win32,linux,darwin}.tsx`. That was stale: the
`<Key … by={activeKeyFor}>` now lives in
`frontend/layout/lib/tilelayout-shared.tsx` (lines 171 and 354), after the
per-platform TileLayout copies were folded into one core in #3041.

**Fixed in the same PR as this spec**, along with a note recorded at that
comment about what keying the whole leaf actually costs — that the rebuild
takes the pane header and tab strip with it, and that this is what the reveal
gate exists to hide. That cost was previously only discoverable by reading
four files in sequence; a future reader of `layoutStack.ts` now finds it in
place, with a pointer here.
