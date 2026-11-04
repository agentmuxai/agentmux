// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Blocks a pane leaf renders for its members: one `<Key>`-scoped Block for the active member,
 * or, once the pane keeps tabs alive, every kept-alive member (and the active one) mounted at once
 * in an absolutely positioned slot. Plain functions, not components, so they are called at the
 * place PaneLeafChrome builds its content and create their effects in that order.
 */

import { Block } from "@/app/block/block";
import { setKeepAliveBlockDormant } from "@/app/store/block-component-registry";
import { PaneStackMemberActiveProvider, usePaneStackMemberActive } from "@/app/tab/pane-stack-member";
import type { NodeModel } from "@/layout/index";
import { Key } from "@/util/keyed";
import { createEffect, For, onCleanup, Show, type Accessor, type JSX } from "solid-js";

export interface PaneMembersInput {
    keepAlive: Accessor<boolean>;
    scopedNodeModel: Accessor<NodeModel>;
    mountedBlockIds: Accessor<string[]>;
    activeBlockId: () => string;
    keepAliveNodeModelFor: (id: string) => NodeModel;
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

    return (
        <div
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
                // (unlike display:none / content-visibility:hidden, which
                // the keep-alive guarantee in `usePaneKinds`, pane-leaf-state.ts, rules out).
                // SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §1.
                opacity: id === activeBlockId() ? "1" : "0",
                "pointer-events": id === activeBlockId() ? "auto" : "none",
            }}
        >
            <PaneStackMemberActiveProvider value={memberActive}>
                <Block nodeModel={keepAliveNodeModelFor(id)} preview={false} />
            </PaneStackMemberActiveProvider>
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
                    {(nm) => <Block nodeModel={nm()} preview={false} />}
                </Key>
            }
        >
            <For each={input.mountedBlockIds()}>{(id) => renderKeepAliveSlot(id, input)}</For>
        </Show>
    );
}
