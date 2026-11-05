// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PinnedWidgetFlyout — the child-widget panel a parent widget (e.g.
 * "Messengers") expands, per SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md.
 * Shared by both placements the spec calls out:
 *
 *  - Case A (pinned in the bar, §3.3): anchored to the parent's own bar
 *    slot, `placement="bottom-start"` (opens downward under the icon, like
 *    More) — the default below.
 *  - Case B (a row inside the More dropdown, §3.4): anchored to that row's
 *    rect, `placement="right-start"` (opens sideways, flips to
 *    `left-start` near the right edge) — passed explicitly by the caller.
 *
 * Structurally a clone of MoreDropdown (§1.2) either way — Portal-rendered,
 * pane-overlay-aware, positioned via the shared computeMenuPosition()
 * primitive, closed via the caller's createSubmenuHover controller as well
 * as (Case A only) click-toggle/outside-click/Escape.
 */

import { AnchoredPopover } from "@/app/element/anchored-popover";
import { makeIconClass } from "@/util/util";
import type { Placement } from "@floating-ui/dom";
import { For, onCleanup, type JSX } from "solid-js";
import { getChildWidgets, openWidget, widgetIconStyle } from "./action-widgets-config";

const PinnedWidgetFlyout = (props: {
    widget: WidgetConfigType;
    wmap: () => Record<string, WidgetConfigType>;
    /**
     * Needed so a child that gets individually pinned (promoted out of this
     * group — see getEffectiveGroupedChildKeys) drops out of this list
     * immediately, reactively, instead of still showing here alongside its
     * new standalone bar/More entry.
     */
    settings: () => Record<string, any>;
    onClose: () => void;
    onItemContextMenu: (pos: { x: number; y: number }, key: string) => void;
    anchor: () => HTMLElement | null;
    /** Default "bottom-start" (Case A). Case B passes "right-start". */
    placement?: Placement;
    /** Default 4 (Case A, matches MoreDropdown). Case B passes 8 to match other nested submenus (flyoutmenu.tsx's SubMenu). */
    gutter?: number;
    onSubmenuEnter?: () => void;
    onSubmenuLeave?: (e: MouseEvent) => void;
    setSubmenuEl?: (el: HTMLDivElement | null) => void;
    ref?: (el: HTMLDivElement) => void;
}): JSX.Element => {
    const children = () => getChildWidgets(props.widget, props.wmap(), props.settings());

    // AnchoredPopover places it, follows chrome zoom and cuts the hole in any browser pane behind it
    // (the same requirement as MoreDropdown; see `frontend/app/platform/pane-overlay.ts`).
    onCleanup(() => props.setSubmenuEl?.(null));

    const handleItemClick = (widget: WidgetConfigType, e?: MouseEvent) => {
        openWidget(widget, { forceNew: e?.shiftKey });
        props.onClose();
    };

    const handleItemContextMenu = (e: MouseEvent, key: string) => {
        e.preventDefault();
        e.stopPropagation();
        // Delegate up — the item popover renders independently so it can
        // stay open while this flyout remains visible (matches MoreDropdown).
        props.onItemContextMenu({ x: e.clientX, y: e.clientY }, key);
    };

    return (
        <AnchoredPopover
            anchor={props.anchor()}
            placement={props.placement ?? "bottom-start"}
            gutter={props.gutter ?? 4}
            hideUntilPlaced
            overlayKeys
            shellClass="anchored-popover--menu"
            class="action-widget-more-dropdown action-widget-pinned-flyout"
            ref={(el) => {
                props.ref?.(el);
                props.setSubmenuEl?.(el);
                el.addEventListener("mouseenter", () => props.onSubmenuEnter?.());
                el.addEventListener("mouseleave", (e) => props.onSubmenuLeave?.(e));
            }}
        >
            <For each={children()}>
                {({ key, widget }) => (
                    <div
                        class="action-widget-more-item"
                        onClick={(e) => handleItemClick(widget, e)}
                        onContextMenu={(e) => handleItemContextMenu(e, key)}
                    >
                        <span class="action-widget-more-item-icon widget-icon" style={widgetIconStyle(widget)}>
                            <i class={makeIconClass(widget.icon, true, { defaultIcon: "window-maximize" })}></i>
                        </span>
                        <span class="action-widget-more-item-label">{widget.label}</span>
                    </div>
                )}
            </For>
        </AnchoredPopover>
    );
};

PinnedWidgetFlyout.displayName = "PinnedWidgetFlyout";

export { PinnedWidgetFlyout };
