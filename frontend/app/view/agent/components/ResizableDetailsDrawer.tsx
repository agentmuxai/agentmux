// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ResizableDetailsDrawer — drag-to-height wrapper for the composer details
 * region's content (AgentShellSubblock — bang-command/slash-command
 * ("system"-tagged) activity-log lines write directly into its terminal
 * rather than a separate panel). The drawer is docked below the composer
 * (SPEC_AGENT_SHELL_BELOW_COMPOSER_2026_08_08.md), and the composer region
 * hugs the pane bottom — so the drawer's BOTTOM edge is pinned and growth
 * happens at its TOP edge. The drag handle therefore sits on the top edge
 * and dragging it UP grows the drawer (the IDE bottom-panel pattern, e.g.
 * VS Code's terminal): the handle is the drawer's one free edge and tracks
 * the cursor 1:1. A bottom-edge handle would sit pinned at the pane bottom
 * with almost no downward travel before the cursor leaves the window.
 * Height is persisted on the agent block's meta (`term:shellheight`) so
 * it's remembered per pane across drawer close/reopen, mirroring how
 * `term:zoom` persists per shell.
 *
 * Per SPEC_LOG_TO_SHELL_PANE_2026_07_02.md §5.1: "make the region a
 * resizable drawer ... not the current fixed short strip."
 */

import { createSignal, onCleanup, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WOS } from "@/app/store/global";

interface ResizableDetailsDrawerProps {
    blockId: string;
    persistedHeight: number | undefined;
    children: JSX.Element;
}

const DEFAULT_HEIGHT = 220;
const MIN_HEIGHT = 120;
const MAX_HEIGHT = 600;

const clampHeight = (h: number): number => Math.max(MIN_HEIGHT, Math.min(MAX_HEIGHT, h));

export const ResizableDetailsDrawer = (props: ResizableDetailsDrawerProps): JSX.Element => {
    const [height, setHeight] = createSignal(clampHeight(props.persistedHeight ?? DEFAULT_HEIGHT));
    const [dragging, setDragging] = createSignal(false);

    let dragStartY = 0;
    let dragStartHeight = 0;

    const onPointerMove = (ev: PointerEvent) => {
        // Dragging the TOP handle up (negative clientY delta) grows the
        // drawer: its bottom edge is pinned to the pane bottom, so the top
        // edge is the free one and moves up as the drawer grows.
        const delta = dragStartY - ev.clientY;
        setHeight(clampHeight(dragStartHeight + delta));
    };

    const onPointerUp = () => {
        window.removeEventListener("pointermove", onPointerMove);
        window.removeEventListener("pointerup", onPointerUp);
        setDragging(false);
        void RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", props.blockId),
            meta: { "term:shellheight": height() } as any,
        });
    };

    const onPointerDown = (e: PointerEvent) => {
        e.preventDefault();
        dragStartY = e.clientY;
        dragStartHeight = height();
        setDragging(true);
        window.addEventListener("pointermove", onPointerMove);
        window.addEventListener("pointerup", onPointerUp);
    };

    onCleanup(() => {
        window.removeEventListener("pointermove", onPointerMove);
        window.removeEventListener("pointerup", onPointerUp);
    });

    return (
        <div class="agent-composer-details-resizable" style={{ height: `${height()}px` }}>
            <div
                class="agent-composer-details-resize-handle"
                classList={{ "agent-composer-details-resize-handle--active": dragging() }}
                onPointerDown={onPointerDown}
                title="Drag to resize"
            />
            <div class="agent-composer-details-body">{props.children}</div>
        </div>
    );
};

ResizableDetailsDrawer.displayName = "ResizableDetailsDrawer";
