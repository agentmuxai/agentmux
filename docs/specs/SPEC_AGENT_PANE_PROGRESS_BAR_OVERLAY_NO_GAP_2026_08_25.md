# SPEC — Agent pane: remove the reserved-space gap above the tab strip; progress bar overlays instead

**Date:** 2026-08-25
**Author:** AgentY
**Status:** Draft — deliberate, partial reversal of `SPEC_AGENT_PANE_PROGRESS_BAR_ABOVE_TAB_STRIP_2026_08_10.md`; see §2.1 for why this isn't the same tradeoff being re-litigated blind.
**Scope:** `frontend/app/view/agent/agent-view.tsx` (`AgentViewWrapper`'s slot div) and `frontend/app/view/agent/agent-view.scss` (`.agent-pane-progress-bar-slot` / `.agent-pane-progress-bar`) only. `PaneTabStrip.tsx`/`.scss` and the bar's own portal machinery (`progressBarSlot` signal, `<Show>`/`<Portal>` wrapping in `AgentPresentationView`) are unaffected — see §2.3.
**Related:** `docs/specs/SPEC_AGENT_PANE_PROGRESS_BAR_ABOVE_TAB_STRIP_2026_08_10.md` (introduced the slot this spec removes — must-read first), `docs/specs/SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md` (the tab strip's own overlay-over-content pattern this spec's fix now mirrors one level up).

---

## 1. Problem (repo-owner-reported)

> "there is a gap between the tab bar in agent pane and the pane header, I believe where the progress bar goes... it makes the + on the new tab look off-center... remove the gap, the progress bar should simply overlap, without taking space while it doesn't appear."

Confirmed against source. The agent pane's structure (`agent-view.tsx:520-544`, `agent-view.scss:40-227`):

```
.agent-pane-stack (flex column, height: 100%)
 ├─ .agent-pane-progress-bar-slot   ← flex: 0 0 auto; height: 3px — ALWAYS present
 └─ .agent-pane-stack-content        ← flex: 1 1 auto; position: relative
      └─ .pane-tab-strip             ← position: absolute; top: 0  (relative to the box above)
      └─ AgentPresentationView content
```

`.agent-pane-progress-bar-slot` (`agent-view.scss:46-48,223-227`) is a real flex child of `.agent-pane-stack`, `flex: 0 0 auto; height: 3px`, present unconditionally — whether or not the bar itself is currently visible. `.pane-tab-strip` is `position: absolute; top: 0` (`agent-view.scss:92-94`), but that `top: 0` is relative to `.agent-pane-stack-content` — which only begins *after* the slot's 3px row in the flex column. Net effect: the tab strip (and therefore the "+" button inside it, `PaneTabStrip.scss`'s `.pane-tab-strip-add`, 28×28px) always renders 3px lower than the top of `.agent-pane-stack` — i.e. 3px below the pane header (`.block-frame-default-header`, outside this component tree) — regardless of whether the progress bar is actually showing. That's the reported gap, and the reason the "+" reads as vertically off relative to the header above it.

## 2. Design

### 2.1 Why this is a deliberate, informed reversal — not blindly redoing the 08-10 decision

The 08-10 spec's §2.3 explicitly considered and rejected "only reserving the row's height while the bar is active," specifically to avoid layout shift when the bar toggles on/off. That reasoning was sound *for the mechanism it was choosing between* — a conditionally-sized reserved row vs. an always-3px reserved row. This spec doesn't choose either of those: it takes the bar **out of the flex flow entirely** (`position: absolute`, contributing zero box-model height at any time, active or not) and has it overlay the top of the tab strip's own already-absolutely-positioned box — the same "floats over content instead of reserving a row" pattern `.pane-tab-strip` itself already uses over `.agent-pane-stack-content` (see the comment at `agent-view.scss:56-64`, and `SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md` for why that precedent was chosen there). Since the bar never occupies flex space at all — not "3px always," not "0px or 3px depending on state" — toggling its opacity causes exactly as little layout shift as the 08-10 spec required (zero), just achieved by removing the space reservation entirely rather than fixing it at a constant size. The two specs' goals (no layout shift) are identical; only the mechanism differs, and the new mechanism is strictly more space-efficient without reintroducing the problem §2.3 was written to avoid.

### 2.2 Take the slot out of flex flow; overlay it above the tab strip

`.agent-pane-progress-bar-slot` (`agent-view.scss:46-48` removes its `flex: 0 0 auto` list entry; `agent-view.scss:223-227` changes from a normal flex box to an absolutely-positioned overlay):

```scss
.agent-pane-progress-bar-slot {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 3px;
    overflow: hidden;
    // Sits above .pane-tab-strip (z: var(--z-pane-overlay, 4)) so the bar,
    // when active, draws visibly on top of the tab strip's own top edge
    // rather than being hidden behind it — both now occupy the same
    // top:0 coordinates once neither reserves flex space.
    z-index: calc(var(--z-pane-overlay, 4) + 1);
    pointer-events: none;
}
```

`.agent-pane-stack` needs `position: relative` added (`agent-view.scss:40-49`) so the slot's `position: absolute` resolves against it, not some further-out ancestor — `.agent-pane-stack` already fills `height: 100%; width: 100%` of its own container, so this is a no-op for every other layout concern in that block.

With the slot out of flow, `.agent-pane-stack-content` (`flex: 1 1 auto`) becomes the *only* flex child contributing height to `.agent-pane-stack` — it now starts at true `top: 0`, and `.pane-tab-strip`'s existing `position: absolute; top: 0` (unchanged, `agent-view.scss:92-94`) now anchors flush against the pane header with no 3px offset. This is the actual gap fix; §2.2's slot change is what makes it possible without also breaking the bar's own visibility.

`.agent-pane-progress-bar` itself (`agent-view.scss:242-291`) needs no changes — it's already `position: absolute; inset: 0` *inside* the slot and fades via `opacity`; none of that depended on the slot being flex-positioned vs. absolutely-positioned.

### 2.3 What's explicitly unchanged

- The portal machinery (`progressBarSlot` signal in `AgentViewWrapper`, `<Show when={progressBarMount()}><Portal mount={progressBarMount()!}>...` in `AgentPresentationView`) — §2.2 of the 08-10 spec's reasoning for why the bar's *state* stays owned by `AgentPresentationView` (avoiding a double-subscribe on `useAgentControllerStatus`) is untouched by this change; only the slot div's own CSS position changes, not which component renders what.
- The zoom-compensation removal from §2.4 of the 08-10 spec — still correct, still applies; this spec doesn't touch `.agent-pane-progress-bar`'s own rule.
- `PaneTabStrip.tsx`/`.scss` — no changes. The "+" button's own 28×28px sizing is correct today; it only *looked* off-center because of the 3px offset applied to its entire containing strip, not because of anything in the button's own styling.

## 3. Files touched

- `frontend/app/view/agent/agent-view.scss` — `.agent-pane-stack` gains `position: relative`; its `> .agent-pane-progress-bar-slot { flex: 0 0 auto; }` rule is removed (no longer a flex participant); `.agent-pane-progress-bar-slot`'s own rule (currently `position: relative; height: 3px; overflow: hidden;`) becomes the absolutely-positioned overlay in §2.2.
- `frontend/app/view/agent/agent-view.tsx` — no structural change expected (the slot div at line 526 stays exactly where it is in JSX; only its CSS changes), but re-check the comment at lines 521-525 ("Progress bar's own row — always reserved...") for accuracy once the fix lands — it currently documents the exact behavior this spec removes and needs updating to describe the overlay instead.

## 4. Verification

- With no agent activity (bar inactive/opacity 0): the tab strip's "+" button should sit flush against the pane header with no visible gap, and read as vertically centered the way it does on panes without a header directly above (editor/terminal panes, unaffected by this change, for a side-by-side comparison).
- Trigger agent activity (bar active): the marching-ants bar should still render as a thin strip at the top of the pane, visually in front of (not clipped by, not hidden behind) the tab strip's top edge — confirm the new `z-index: calc(var(--z-pane-overlay, 4) + 1)` actually wins against `.pane-tab-strip`'s `var(--z-pane-overlay, 4)`.
- Toggle activity on/off repeatedly: confirm zero layout shift in the tab strip or conversation content, matching the 08-10 spec's original guarantee (see §2.1 for why the new mechanism preserves this).
- Multi-tab case (2+ tabs open in one pane): confirm the fix doesn't regress `SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md`'s glass-panel/click-through behavior — this spec doesn't touch that logic, but it's the same DOM region and worth a quick visual pass.
- **Visual verification is mandatory** — same caveat as every other UI spec this session: a sandboxed environment has no display; needs a live `task dev` or packaged-build check before this is considered confirmed, not just "compiles and the CSS looks right on paper."
