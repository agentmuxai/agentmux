// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { renderPaneChromeShell } from "@/app/element/PaneChrome";
import type { NodeModel } from "@/layout/index";
import { Show, type JSX } from "solid-js";
import { renderPaneMembers } from "./pane-leaf-members";
import {
    createViewModelScopes,
    useAgentMountWatch,
    useChromeNodeModel,
    useLatchedChromeVm,
    useMinimizedMembers,
    usePaneKinds,
    useScopedNodeModel,
    useStackMembers,
} from "./pane-leaf-state";

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
 * just already-stacked ones; see `usePaneKinds` (pane-leaf-state.ts) for why gating
 * that on stack size was a catch-22. Any other view type falls through to
 * the bare `content` below — zero behavior change for those, matching what
 * `<Block>` alone already did.
 */

export function PaneLeafChrome(props: { nodeModel: NodeModel }): JSX.Element {
    const nodeModel = props.nodeModel;
    const activeBlockId = () => nodeModel.activeBlockId?.() ?? nodeModel.blockId;

    const { hoisted, keepAlive } = usePaneKinds(activeBlockId);
    const scopes = createViewModelScopes(nodeModel, hoisted);
    const scopedNodeModel = useScopedNodeModel(nodeModel, activeBlockId, hoisted, scopes);
    const members = useStackMembers(nodeModel, activeBlockId, scopes);
    useAgentMountWatch(nodeModel, activeBlockId, members);
    useMinimizedMembers(nodeModel, members.stackBlockIds);
    const chromeNodeModel = useChromeNodeModel(nodeModel, activeBlockId, scopes);

    const content = renderPaneMembers({
        keepAlive,
        scopedNodeModel,
        mountedBlockIds: members.mountedBlockIds,
        activeBlockId,
        keepAliveNodeModelFor: scopes.keepAliveNodeModelFor,
    });

    const chromeVm = useLatchedChromeVm(chromeNodeModel);

    return (
        <Show when={hoisted()} fallback={content}>
            {/* Guards only the leaf's own first-hoist window, before any
                ViewModel has been observed yet — does NOT itself unmount
                the outer chrome once mounted, since `chromeVm()` (above)
                never returns to a falsy value after its first real
                capture. */}
            {/* The shared shell unless a view supplies its own. (Comment kept
                OUTSIDE <Show>: a sibling of the callback child would turn it
                into an array and break the callback form.) */}
            <Show when={chromeVm()}>
                {(vm) => (vm().renderPaneChrome ?? renderPaneChromeShell)(chromeNodeModel(), content)}
            </Show>
        </Show>
    );
}
