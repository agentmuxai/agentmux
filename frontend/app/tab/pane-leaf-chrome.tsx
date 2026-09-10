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
 * `"agent"` AND this leaf's stack has ever had 2+ members
 * (`NodeModel.hasEverBeenMultiMember`), wraps that `<Block>` in the
 * currently-active `ViewModel`'s own `renderPaneChrome` — a stable outer
 * shell (tab strip, progress-bar slot) that survives every subsequent
 * switch, instead of `BlockFrame`'s own per-switch-remounting header.
 * Every other case (another view type, or an agent pane that's never gone
 * multi-member) falls through to the bare `content` below — zero behavior
 * change for those, matching what `<Block>` alone already did.
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

    const hoisted = createMemo(
        () => (nodeModel.hasEverBeenMultiMember?.() ?? false) && effectiveViewType() === "agent"
    );

    return (
        <Show when={hoisted()} fallback={content}>
            {/* Guards only the brief same-flush null window between one
                member's <Block> unmounting and the next one's mounting
                (NodeModel.activeViewModel's own doc comment, types.ts) —
                does NOT itself unmount the outer chrome, which is gated
                only by `hoisted()` above and never flips back to false. */}
            <Show when={nodeModel.activeViewModel?.()}>
                {(vm) => vm().renderPaneChrome!(nodeModel, content)}
            </Show>
        </Show>
    );
}
