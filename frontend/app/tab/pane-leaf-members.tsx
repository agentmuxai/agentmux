// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Blocks a pane leaf renders for its members: one `<Key>`-scoped Block for the active member,
 * or, once the pane keeps tabs alive, every kept-alive member (and the active one) mounted at once
 * in an absolutely positioned slot. Plain functions, not components, so they are called at the
 * place PaneLeafChrome builds its content and create their effects in that order.
 */

import { Block } from "@/app/block/block";
import { layoutBehind, markLayoutBehind, windowResizing } from "@/app/platform/window-resize";
import { setKeepAliveBlockDormant } from "@/app/store/block-component-registry";
import { AgentStalledBanner } from "@/app/tab/agent-stalled-banner";
import { PaneStackMemberActiveProvider, usePaneStackMemberActive } from "@/app/tab/pane-stack-member";
import type { NodeModel } from "@/layout/index";
import { registerTabMember, tabLandingMountOf } from "@/layout/lib/tabLanding";
import { Key } from "@/util/keyed";
import { createEffect, For, onCleanup, Show, untrack, type Accessor, type JSX } from "solid-js";

export interface PaneMembersInput {
    keepAlive: Accessor<boolean>;
    scopedNodeModel: Accessor<NodeModel>;
    mountedBlockIds: Accessor<string[]>;
    activeBlockId: () => string;
    keepAliveNodeModelFor: (id: string) => NodeModel;
    /** Whether a member's own content can stand in its landing place while its tab is dragged. Default: it can. */
    lands?: (id: string) => boolean;
}

/** One kept-alive member: its dormant flag, its slot (visible or hidden) and its Block. */
function renderKeepAliveSlot(id: string, input: PaneMembersInput): JSX.Element {
    const { activeBlockId, keepAliveNodeModelFor } = input;
    // Every "all panes" consumer of the block-component registry
    // (multi-input broadcast, all-panes zoom, the
    // multi-input-eligible terminal count) assumes one
    // registry entry per currently-VISIBLE pane (#3187). Mark this
    // member dormant whenever it isn't the active one, so
    // those consumers keep seeing exactly what they did
    // before keep-alive existed — see
    // block-component-registry.ts's own comment for why
    // this is tracked explicitly here rather than
    // re-derived from a layout/tab lookup. Cleared on this
    // row's own unmount (the id left the stack — tab
    // closed), not just flipped to false, so a closed tab
    // can never linger as a phantom dormant entry.
    createEffect(() => {
        setKeepAliveBlockDormant(id, id !== activeBlockId());
    });
    onCleanup(() => setKeepAliveBlockDormant(id, false));
    // For what this member renders through a Portal (pane-stack-member.ts).
    const outerMemberActive = usePaneStackMemberActive();
    const memberActive = () => outerMemberActive() && id === activeBlockId();

    // A dormant member does not lay its content out while the window is resized, nor until it is its turn to catch up
    // afterwards (window-resize.ts): four or five hidden agent views relaying out their transcripts and composers on
    // every frame of a drag is most of the lag, and only the active member changes what is on screen. The DOM, the
    // mount and the slot's box stay, so the observers inside fire once at the final size when it is released.
    const behindKey = `keepalive:${id}`;
    createEffect(() => {
        // Only the resize flag is tracked: a member that goes dormant mid-drag still skips (skipsLayout) and
        // simply has no catch-up turn of its own, so it lays out when the drag settles.
        if (windowResizing() && untrack(() => id !== activeBlockId())) markLayoutBehind(behindKey);
    });
    const skipsLayout = () => id !== activeBlockId() && (windowResizing() || layoutBehind(behindKey));

    // The member's content, in an element of its own inside the slot: while the tab is dragged out of the
    // pane and its landing place is shown, the element stands in the place (layout/lib/tabLanding.ts), laid
    // out for it, and comes back when the place goes. Moved, not mounted again, as a maximized pane's leaf is
    // (TileLayout.core.tsx `useMagnifiedLeafMount`). The slot itself never moves: the list places its slots
    // by their neighbours.
    let slot: HTMLDivElement | undefined;
    let member: HTMLDivElement | undefined;
    const lands = () => input.lands?.(id) ?? true;
    createEffect(() => {
        if (lands()) onCleanup(registerTabMember(id));
    });
    createEffect(() => {
        const home = (lands() && tabLandingMountOf(id)) || slot;
        if (member && home && member.parentElement !== home) home.appendChild(member);
    });
    // The slot's own removal takes the member with it only while the member is in it.
    onCleanup(() => {
        if (member && member.parentElement !== slot) member.remove();
    });

    return (
        <div
            ref={slot}
            class="pane-leaf-keepalive-slot"
            // Overlays in a dormant member cut no hole through the
            // browser panes on screen (pane-overlay.ts DORMANT_SLOT_ATTR).
            data-pane-dormant={id === activeBlockId() ? undefined : ""}
            style={{
                position: "absolute",
                inset: "0",
                // `inherit`, not `visible`: an explicit `visible`
                // would show through a hidden window tab kept laid out
                // with `visibility: hidden` (workspace.tsx).
                visibility: id === activeBlockId() ? "inherit" : "hidden",
                // Opacity as well as visibility: a descendant can keep
                // painting through `visibility: hidden` (a CSS transition
                // on `visibility` holds it `visible` for its duration, and
                // an explicit `visibility: visible` escapes it outright),
                // which showed the Help tab's content as a ~300ms "ghost"
                // over the newly selected tab. Nothing escapes a
                // container's opacity, and it keeps the slot's real size
                // (unlike display:none, which the keep-alive guarantee in
                // `usePaneKinds`, pane-leaf-state.ts, rules out; and a
                // permanent content-visibility:hidden, ruled out for the same
                // reason: the fit logic needs a live size — it is used only
                // while the window is resized, via `skipsLayout`).
                // SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §1.
                opacity: id === activeBlockId() ? "1" : "0",
                "pointer-events": id === activeBlockId() ? "auto" : "none",
                "content-visibility": skipsLayout() ? "hidden" : undefined,
            }}
        >
            <div ref={member} class="pane-leaf-member" style={{ position: "absolute", inset: "0" }}>
                <PaneStackMemberActiveProvider value={memberActive}>
                    <Block nodeModel={keepAliveNodeModelFor(id)} preview={false} />
                </PaneStackMemberActiveProvider>
            </div>
            <AgentStalledBanner blockId={id} />
        </div>
    );
}

/**
 * The members' Blocks. Every KEPT-ALIVE member (plus the active one, whatever its
 * type — see `mountedBlockIds`) is mounted simultaneously, absolutely
 * positioned over one another inside the (already
 * `position: relative`) content slot each keep-alive pane
 * type reserves (e.g. term.scss's `.term-pane-stack-content`)
 * — visibility, not existence, is what switches. A hidden
 * member keeps its real (non-zero) box size the whole time
 * (`visibility: hidden` does not collapse layout the way
 * `display: none` would), so its own ResizeObserver-driven
 * fit logic (e.g. TermWrap's) stays correct in the background
 * and there is nothing to re-fit when it's revealed again.
 */
export function renderPaneMembers(input: PaneMembersInput): JSX.Element {
    return (
        <Show
            when={input.keepAlive()}
            fallback={
                <Key each={[input.scopedNodeModel()]} by={(nm) => nm.blockId}>
                    {(nm) => (
                        <>
                            <Block nodeModel={nm()} preview={false} />
                            <AgentStalledBanner blockId={nm().blockId} />
                        </>
                    )}
                </Key>
            }
        >
            <For each={input.mountedBlockIds()}>{(id) => renderKeepAliveSlot(id, input)}</For>
        </Show>
    );
}
