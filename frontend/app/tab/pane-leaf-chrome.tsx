// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Block, resolveEffectiveViewType } from "@/app/block/block";
import { WOS } from "@/app/store/global";
import type { NodeModel } from "@/layout/index";
import { Key } from "@solid-primitives/keyed";
import { createMemo, Show, type JSX } from "solid-js";

/**
 * Router `tabcontent.tsx` renders instead of `<Block>` directly — the
 * leaf-level entry point for the pane-tab-switch chrome-stability fix
 * (`docs/specs/SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`).
 *
 * ALWAYS renders a per-active-member, `<Key>`-scoped `<Block>` — the inner
 * remount boundary `layoutStack.ts`'s own header comment refers to. This
 * alone is what updates displayed content on a switch now that the OUTER,
 * whole-leaf remount (`tilelayout-shared.tsx`'s `DisplayNodesWrapper`) no
 * longer does — for every view type, uniformly, including one that never
 * hoists chrome at all: the end result (`<Block>` remounts on every switch)
 * is byte-for-byte identical to what the old outer-leaf-key mechanism
 * produced, just driven from a narrower boundary.
 *
 * When the active member's EFFECTIVE view type (`resolveEffectiveViewType`
 * — the same migration/rename redirects `block.tsx`'s own `makeViewModel`
 * applies, so a still-live "forge" block routes through this too) is
 * `"agent"`, wraps that `<Block>` in the currently-active `ViewModel`'s own
 * `renderPaneChrome` — a stable outer shell (header, tab strip,
 * progress-bar slot) that survives every subsequent switch, instead of
 * `BlockFrame`'s own per-switch-remounting header. Every agent pane, not
 * just already-stacked ones; see the `hoisted` memo below for why gating
 * that on stack size was a catch-22. Any other view type falls through to
 * the bare `content` below — zero behavior change for those, matching what
 * `<Block>` alone already did.
 */
export function PaneLeafChrome(props: { nodeModel: NodeModel }): JSX.Element {
    const nodeModel = props.nodeModel;
    const activeBlockId = () => nodeModel.activeBlockId?.() ?? nodeModel.blockId;

    // Block-scoped NodeModel wrapper for the inner, per-activation <Block>
    // mount. The LEAF's own NodeModel.blockId is frozen — captured once,
    // never re-derived (see that field's own doc comment in types.ts) —
    // and, now that layoutStack.ts no longer disposes this NodeModel on a
    // switch, would show the SAME member forever if passed straight into
    // <Block>. Reconstructed (new object identity) exactly when
    // activeBlockId() changes — i.e. exactly when the <Key> below remounts
    // anyway — so nothing ever observes a wrapper whose .blockId doesn't
    // match its own <Block> mount's lifetime, satisfying
    // NodeModel.blockId's "one instance, one immutable blockId" contract
    // at this narrower, per-activation granularity. Every other NodeModel
    // field (focus/magnify/minimize/close/etc.) delegates straight through
    // to the real, leaf-level nodeModel via the spread — those stay
    // leaf-scoped, unaffected by which member is active.
    const scopedNodeModel = createMemo<NodeModel>(() => ({ ...nodeModel, blockId: activeBlockId() }));

    const content = (
        <Key each={[scopedNodeModel()]} by={(nm) => nm.blockId}>
            {(nm) => <Block nodeModel={nm()} preview={false} />}
        </Key>
    );

    // Effective view type of the ACTIVE member, reactive — getWaveObjectAtom
    // inside a memo, not useWaveObjectValue, the same reactive-oref pattern
    // PR #3134 established for BlockFrame_Header (frontend/app/store/wos.ts's
    // own doc comments explain why: useWaveObjectValue's onCleanup-ref-count
    // is tied to THIS component's mount, and never re-subscribes if the
    // oref it was called with later changes).
    const activeBlockData = createMemo(() => WOS.getWaveObjectAtom<Block>(WOS.makeORef("block", activeBlockId()))());
    const effectiveViewType = createMemo(() => resolveEffectiveViewType(activeBlockData()?.meta?.view ?? ""));

    // Hoist for EVERY agent pane, not just ones whose stack has already
    // gone multi-member. An earlier version gated this on
    // `NodeModel.hasEverBeenMultiMember` to keep never-stacked panes on a
    // byte-for-byte passthrough — but that was a catch-22, found live:
    // AgentPaneChrome owns the tab strip, the tab strip owns the "+"
    // button, and "+" is the only way to reach a 2nd stack member. Gated
    // that way, a single-member pane rendered no strip and therefore no
    // "+", so `hasEverBeenMultiMember` could never become true and the
    // whole feature was unreachable. The strip's own visibility rules
    // (`shouldShowTabStrip` — hidden on a fresh picker pane, "+"-only for
    // one live conversation, pills once there are 2+) still live inside
    // chrome and are unchanged; this only decides whether chrome EXISTS.
    //
    // Latched for the same reason the ViewModel below is: `effectiveViewType()`
    // reads the ACTIVE member's block meta, which is briefly undefined
    // while a newly-activated member's data loads, so an unlatched read
    // would blip false->true on a switch and remount chrome — the exact
    // flash this file exists to prevent.
    let latchedHoisted = false;
    const hoisted = createMemo(() => {
        if (!latchedHoisted && effectiveViewType() === "agent") {
            latchedHoisted = true;
        }
        return latchedHoisted;
    });

    // ReAgent P1, confirmed by an empirical repro before trusting it:
    // `NodeModel.activeViewModel()` genuinely blips through `null` on
    // EVERY switch, not just the first hoist. `block.tsx`'s `onCleanup`
    // (clearing the OLD member's vm) runs synchronously during the inner
    // `<Key>`'s reconciliation; the `createEffect` that sets the NEW
    // member's vm is deferred to the next effects flush. A naive
    // `<Show when={nodeModel.activeViewModel()}>` callback form looks safe
    // (`<Show>` only re-runs its child function on a falsy->truthy edge),
    // but that real null blip IS such an edge every time, so it would
    // re-invoke `renderPaneChrome` — mounting a BRAND NEW `AgentPaneChrome`
    // on every switch and reproducing the exact flash this file exists to
    // eliminate. Fixed by latching the FIRST non-null vm ever observed
    // into a plain closure variable: once captured, this memo's OWN return
    // value stops changing (same object reference) regardless of how
    // `activeViewModel()` fluctuates underneath, so Solid's default `===`
    // memo comparison means no consumer — including `<Show>` below — ever
    // observes a falsy value again after the first real capture. Renders
    // via whichever vm happened to be active at that FIRST hoist and never
    // re-derives afterward — the same "frozen after first hoist" identity
    // `anchorBlockId` (agent-model.ts's own `renderPaneChrome` closure)
    // already assumes.
    let latchedChromeVm: ViewModel | null = null;
    const chromeVm = createMemo(() => {
        if (!latchedChromeVm) {
            latchedChromeVm = nodeModel.activeViewModel?.() ?? null;
        }
        return latchedChromeVm;
    });

    return (
        <Show when={hoisted()} fallback={content}>
            {/* Guards only the leaf's own first-hoist window, before any
                ViewModel has been observed yet — does NOT itself unmount
                the outer chrome once mounted, since `chromeVm()` (above)
                never returns to a falsy value after its first real
                capture. */}
            <Show when={chromeVm()}>
                {(vm) => vm().renderPaneChrome!(nodeModel, content)}
            </Show>
        </Show>
    );
}
