// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneGrip — the pane's grip, in its header just left of the pane controls
 * (minimize / magnify / close), outside the scrolling tab strip, so it never
 * scrolls or fades with the tabs. Drag it to move the pane: the header's own
 * whole-pane drag covers it. Click it, or press Enter or Space, for the pane
 * menu (the header's own `contextmenu`).
 *
 * See docs/specs/SPEC_DRAG_SPACE_AFFORDANCE_2026_10_13.md §5.2, §5.8.
 */

import type { JSX } from "solid-js";
import { GripDots } from "./GripDots";
import "./PaneGrip.scss";

/** Screen position of the last press on a grip. A floating pane moves its
 *  window under the cursor, so the release can land on the same grip and the
 *  browser still fires `click`; the screen position is what moved. */
let gripPress: { x: number; y: number } | null = null;

/** A click on the grip after it was dragged more than a few pixels is the end
 *  of a move, not a request for the menu. */
function gripClickWasDrag(e: MouseEvent): boolean {
    const start = gripPress;
    gripPress = null;
    return !!start && Math.abs(e.screenX - start.x) + Math.abs(e.screenY - start.y) > 4;
}

/** Opens the pane header's own context menu (the header's `contextmenu`
 *  handler), anchored at `el`: the grip's click and keyboard route to the same
 *  actions a right-click gives. */
function openPaneMenuFrom(el: HTMLElement): void {
    const r = el.getBoundingClientRect();
    el.dispatchEvent(
        new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left, clientY: r.bottom, button: 2 }),
    );
}

export const PaneGrip = (props: {
    /** The pane can't be dragged right now (magnified or ephemeral: the
     *  header's `canDrag` vetoes it). The grip then offers only the menu:
     *  no grab cursor, and its label and tooltip drop "move". */
    dragDisabled?: boolean;
}): JSX.Element => (
    <div
        class="pane-grip"
        classList={{ "pane-grip--no-drag": !!props.dragDisabled }}
        role="button"
        tabIndex={0}
        aria-label={props.dragDisabled ? "Pane actions" : "Move pane / pane actions"}
        aria-haspopup="menu"
        title={props.dragDisabled ? "Pane actions" : "Drag to move this pane. Click for pane actions."}
        onPointerDown={(e) => {
            gripPress = { x: e.screenX, y: e.screenY };
        }}
        onClick={(e) => {
            if (gripClickWasDrag(e)) return;
            openPaneMenuFrom(e.currentTarget);
        }}
        // A double-click on the header magnifies the pane; on the grip it is
        // two clicks on a control, like the pane controls beside it.
        onDblClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
            if (e.key !== "Enter" && e.key !== " ") return;
            e.preventDefault();
            openPaneMenuFrom(e.currentTarget);
        }}
    >
        <GripDots />
    </div>
);
