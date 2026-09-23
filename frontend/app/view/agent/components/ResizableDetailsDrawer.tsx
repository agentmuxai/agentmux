// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ResizableDetailsDrawer — drag-to-height wrapper for a pane-docked drawer.
 * Used twice today, at opposite edges of the same pane:
 *
 *   - `anchor="bottom"` (the default, and the original use): the composer
 *     details region's content (AgentShellSubblock — bang-command/slash-
 *     command ("system"-tagged) activity-log lines write directly into its
 *     terminal rather than a separate panel), docked below the composer
 *     (SPEC_AGENT_SHELL_BELOW_COMPOSER_2026_08_08.md).
 *   - `anchor="top"`: the Stash drawer, docked under the pane header where
 *     its own toggle icon lives
 *     (SPEC_AGENT_STASH_PANE_MIGRATION_2026_09_22.md §3.1).
 *
 * THE HANDLE ALWAYS SITS ON THE DRAWER'S FREE EDGE — the one opposite the
 * pinned one — so it tracks the cursor 1:1 as the drawer grows:
 *
 *   - Bottom-anchored: the composer region hugs the pane bottom, so the
 *     drawer's BOTTOM edge is pinned and growth happens at its TOP edge.
 *     Handle on top; dragging UP grows it (the IDE bottom-panel pattern,
 *     e.g. VS Code's terminal). A bottom-edge handle would sit pinned at
 *     the pane bottom with almost no downward travel before the cursor
 *     leaves the window.
 *   - Top-anchored: mirror image. The TOP edge is pinned under the header,
 *     so the handle sits on the BOTTOM edge and dragging DOWN grows it.
 *
 * That mirroring is the whole of the anchor difference: which end of the
 * flex column the handle renders at, and the sign of the drag delta.
 * Everything else (clamping, persistence, pointer capture/cleanup) is
 * anchor-independent and stays shared — a near-duplicate second component
 * would have to be kept in sync by hand for no benefit.
 *
 * Height is persisted on the agent block's meta (`persistMetaKey`) so it's
 * remembered per pane across drawer close/reopen, mirroring how `term:zoom`
 * persists per shell. Each drawer owns its own key — they are independent
 * surfaces and must not clobber each other's remembered size.
 *
 * Per SPEC_LOG_TO_SHELL_PANE_2026_07_02.md §5.1: "make the region a
 * resizable drawer ... not the current fixed short strip."
 */

import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { MOS } from "@/app/store/global";

interface ResizableDetailsDrawerProps {
    blockId: string;
    persistedHeight: number | undefined;
    children: JSX.Element;
    /** Which edge is pinned. Default `"bottom"` — the shell drawer's original behavior. */
    anchor?: "top" | "bottom";
    /** Block-meta key the dragged height is persisted under. Default is the shell drawer's. */
    persistMetaKey?: string;
    /**
     * Class namespace for the three structural elements
     * (`${classPrefix}-resizable` / `-resize-handle` / `-body`). Defaults to
     * the shell drawer's existing names so its stylesheet needs no change;
     * a second drawer passes its own prefix rather than inheriting styles
     * named for the composer.
     */
    classPrefix?: string;
    defaultHeight?: number;
    minHeight?: number;
    maxHeight?: number;
}

const DEFAULT_HEIGHT = 220;
const MIN_HEIGHT = 120;
const MAX_HEIGHT = 600;

export const ResizableDetailsDrawer = (props: ResizableDetailsDrawerProps): JSX.Element => {
    const anchor = () => props.anchor ?? "bottom";
    const prefix = () => props.classPrefix ?? "agent-composer-details";
    const minHeight = () => props.minHeight ?? MIN_HEIGHT;
    const maxHeight = () => props.maxHeight ?? MAX_HEIGHT;
    const clampHeight = (h: number): number => Math.max(minHeight(), Math.min(maxHeight(), h));

    const [height, setHeight] = createSignal(
        clampHeight(props.persistedHeight ?? props.defaultHeight ?? DEFAULT_HEIGHT),
    );
    const [dragging, setDragging] = createSignal(false);

    let dragStartY = 0;
    let dragStartHeight = 0;

    const onPointerMove = (ev: PointerEvent) => {
        // The free edge tracks the cursor: for a bottom-anchored drawer that
        // edge is on TOP, so moving the cursor UP (negative clientY delta)
        // grows it; for a top-anchored drawer the free edge is on the BOTTOM,
        // so the sign flips. See this module's doc comment.
        const delta = anchor() === "bottom" ? dragStartY - ev.clientY : ev.clientY - dragStartY;
        setHeight(clampHeight(dragStartHeight + delta));
    };

    const onPointerUp = () => {
        window.removeEventListener("pointermove", onPointerMove);
        window.removeEventListener("pointerup", onPointerUp);
        setDragging(false);
        void RpcApi.SetMetaCommand(TabRpcClient, {
            oref: MOS.makeORef("block", props.blockId),
            meta: { [props.persistMetaKey ?? "term:shellheight"]: height() } as any,
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

    const handle = () => (
        <div
            class={`${prefix()}-resize-handle`}
            classList={{ [`${prefix()}-resize-handle--active`]: dragging() }}
            onPointerDown={onPointerDown}
            title="Drag to resize"
        />
    );

    return (
        <div class={`${prefix()}-resizable`} style={{ height: `${height()}px` }}>
            {/* Handle renders BEFORE the body when the free edge is on top,
                AFTER it when the free edge is on the bottom — DOM order is
                what puts it on the right edge of the flex column. */}
            <Show when={anchor() === "bottom"}>{handle()}</Show>
            <div class={`${prefix()}-body`}>{props.children}</div>
            <Show when={anchor() === "top"}>{handle()}</Show>
        </div>
    );
};

ResizableDetailsDrawer.displayName = "ResizableDetailsDrawer";
