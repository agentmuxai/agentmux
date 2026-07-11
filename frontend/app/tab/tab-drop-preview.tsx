// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createMemo, For, onCleanup, Show } from "solid-js";
import type { JSX } from "solid-js";
import { computeSchematicRects, DropDirection, getLayoutModelForTabById, schematicLeaves } from "@/layout/index";
import { determineDropDirection } from "@/layout/lib/utils";
import { hoveredDropClientPos, setDropGhost } from "./tabbar-dnd";
import "./tab-drop-preview.scss";

const PREVIEW_WIDTH = 220;
const PREVIEW_HEIGHT = 130;
const PREVIEW_GAP_PX = 6;

/**
 * Maps a DropDirection to a sub-rect within a leaf's rect: Center → full
 * leaf, Top/Right/Bottom/Left → half along that axis, Outer* → a thin 1/5
 * band against that edge. Mirrors `rectForDirection` in app-init.ts
 * (installFloatingRedockHoverListener) — same visual convention, computed
 * here against schematic (popover-local) rects instead of real screen ones.
 */
function rectForDirection(leaf: Dimensions, dir: DropDirection): Dimensions {
    switch (dir) {
        case DropDirection.Top:
            return { top: leaf.top, left: leaf.left, width: leaf.width, height: leaf.height / 2 };
        case DropDirection.Right:
            return { top: leaf.top, left: leaf.left + leaf.width / 2, width: leaf.width / 2, height: leaf.height };
        case DropDirection.Bottom:
            return { top: leaf.top + leaf.height / 2, left: leaf.left, width: leaf.width, height: leaf.height / 2 };
        case DropDirection.Left:
            return { top: leaf.top, left: leaf.left, width: leaf.width / 2, height: leaf.height };
        case DropDirection.OuterTop:
            return { top: leaf.top, left: leaf.left, width: leaf.width, height: leaf.height / 5 };
        case DropDirection.OuterRight:
            return {
                top: leaf.top,
                left: leaf.left + (4 * leaf.width) / 5,
                width: leaf.width / 5,
                height: leaf.height,
            };
        case DropDirection.OuterBottom:
            return {
                top: leaf.top + (4 * leaf.height) / 5,
                left: leaf.left,
                width: leaf.width,
                height: leaf.height / 5,
            };
        case DropDirection.OuterLeft:
            return { top: leaf.top, left: leaf.left, width: leaf.width / 5, height: leaf.height };
        case DropDirection.Center:
        default:
            return { top: leaf.top, left: leaf.left, width: leaf.width, height: leaf.height };
    }
}

export interface TabDropPreviewProps {
    tabId: string;
    /** Viewport rect of the hovered tab wrapper — anchors the popover below it. */
    anchorRect: Dimensions;
}

/**
 * A schematic (non-live, no real pane content) preview of a tab's current
 * split layout, shown while a pane is being dragged over that tab in the
 * tab bar. Reads the target tab's `LayoutModel` directly — which is live
 * in memory even for an inactive/hidden tab (layoutModelHooks.ts) — and
 * renders its tree via `computeSchematicRects`, a standalone pure function
 * (deliberately not the production `updateTreeHelper`, which mutates the
 * real LayoutModel's signals and would corrupt that tab's actual render
 * state — see schematicLayout.ts). A ghost rect tracks the pointer within
 * the preview so the user can choose exactly where the pane will land.
 * See SPEC_PANE_DRAG_TO_TAB_2026_07_10.md §4.3–§4.4.
 */
export function TabDropPreview(props: TabDropPreviewProps): JSX.Element {
    const rootNode = createMemo(() => getLayoutModelForTabById(props.tabId).localTreeStateAtom().rootNode);

    const popoverRect = createMemo<Dimensions>(() => {
        const anchor = props.anchorRect;
        let left = anchor.left;
        // Clamp so the popover doesn't run off the right edge of the window.
        if (typeof window !== "undefined") {
            left = Math.min(left, window.innerWidth - PREVIEW_WIDTH - 4);
        }
        left = Math.max(left, 4);
        return {
            top: anchor.top + anchor.height + PREVIEW_GAP_PX,
            left,
            width: PREVIEW_WIDTH,
            height: PREVIEW_HEIGHT,
        };
    });

    const leaves = createMemo(() => {
        const node = rootNode();
        if (!node) return [];
        const rects = computeSchematicRects(node, { top: 0, left: 0, width: PREVIEW_WIDTH, height: PREVIEW_HEIGHT });
        return schematicLeaves(node, rects);
    });

    // Resolved ghost: which leaf (schematic-local rect) + direction the
    // pointer is currently over, in POPOVER-LOCAL coordinates. Derived
    // entirely from props/signals (no DOM measurement) so there's no
    // ref-mount-timing race — the popover's position is fully determined
    // by popoverRect() above, so its coordinate frame is known up front.
    const ghost = createMemo(() => {
        const pos = hoveredDropClientPos();
        if (!pos) return null;
        const pr = popoverRect();
        const local = { x: pos.x - pr.left, y: pos.y - pr.top };
        for (const leaf of leaves()) {
            const dir = determineDropDirection(leaf.rect, local);
            if (dir !== undefined) {
                return { blockId: leaf.blockId, direction: dir, rect: rectForDirection(leaf.rect, dir) };
            }
        }
        return null;
    });

    createEffect(() => {
        const g = ghost();
        setDropGhost(g ? { targetBlockId: g.blockId, direction: g.direction } : null);
    });

    onCleanup(() => setDropGhost(null));

    return (
        <div
            class="tab-drop-preview"
            style={{
                position: "fixed",
                top: `${popoverRect().top}px`,
                left: `${popoverRect().left}px`,
                width: `${popoverRect().width}px`,
                height: `${popoverRect().height}px`,
            }}
        >
            <For each={leaves()}>
                {(leaf) => (
                    <div
                        class="tab-drop-preview-leaf"
                        style={{
                            top: `${leaf.rect.top}px`,
                            left: `${leaf.rect.left}px`,
                            width: `${leaf.rect.width}px`,
                            height: `${leaf.rect.height}px`,
                        }}
                    />
                )}
            </For>
            <Show when={ghost()}>
                {(g) => (
                    <div
                        class="tab-drop-preview-ghost"
                        style={{
                            top: `${g().rect.top}px`,
                            left: `${g().rect.left}px`,
                            width: `${g().rect.width}px`,
                            height: `${g().rect.height}px`,
                        }}
                    />
                )}
            </Show>
        </div>
    );
}
