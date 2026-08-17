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

import { usePaneOverlay } from "@/app/platform/pane-overlay";
import {
    assertMenuInPaintableArea,
    computeMenuPosition,
} from "@/app/util/menu-position";
import { makeIconClass } from "@/util/util";
import { autoUpdate, type Placement } from "@floating-ui/dom";
import { createSignal, For, onCleanup, type JSX } from "solid-js";
import { getChildWidgets, handleWidgetSelect } from "./action-widgets-config";

const PinnedWidgetFlyout = (props: {
    widget: WidgetConfigType;
    wmap: () => Record<string, WidgetConfigType>;
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
    const children = () => getChildWidgets(props.widget, props.wmap());

    let overlayEl: HTMLDivElement | undefined;
    // Cut a transparent hole through any browser pane HWND behind this
    // flyout so DOM renders above the pane in this rect — same requirement
    // as MoreDropdown. See `frontend/app/platform/pane-overlay.ts`.
    usePaneOverlay(() => overlayEl);

    const [floatingStyle, setFloatingStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
        visibility: "hidden",
    });
    let cleanupAutoUpdate: (() => void) | null = null;

    const registerFloating = (el: HTMLDivElement) => {
        overlayEl = el;
        props.ref?.(el);
        props.setSubmenuEl?.(el);
        requestAnimationFrame(() => {
            const anchorEl = props.anchor();
            if (!(anchorEl instanceof Element) || !(el instanceof Element)) return;
            const update = async () => {
                const a = props.anchor();
                if (!a) return;
                // avoidNativePanes:false — carries data-pane-overlay, same
                // rationale as MoreDropdown: open in place under the slot,
                // not pushed toward the window edge by a pane behind it.
                const pos = await computeMenuPosition(
                    {
                        anchor: a,
                        placement: props.placement ?? "bottom-start",
                        gutter: props.gutter ?? 4,
                        avoidNativePanes: false,
                    },
                    el,
                );
                setFloatingStyle({ ...pos.style });
            };
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = autoUpdate(anchorEl, el, update);
            assertMenuInPaintableArea(el, "pinned-widget-flyout");
        });
    };

    onCleanup(() => {
        cleanupAutoUpdate?.();
        props.setSubmenuEl?.(null);
    });

    const handleItemClick = (widget: WidgetConfigType) => {
        handleWidgetSelect(widget);
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
        <div
            ref={registerFloating}
            class="action-widget-more-dropdown action-widget-pinned-flyout"
            style={floatingStyle()}
            data-pane-overlay
            onMouseEnter={() => props.onSubmenuEnter?.()}
            onMouseLeave={(e) => props.onSubmenuLeave?.(e)}
        >
            <For each={children()}>
                {({ key, widget }) => (
                    <div
                        class="action-widget-more-item"
                        onClick={() => handleItemClick(widget)}
                        onContextMenu={(e) => handleItemContextMenu(e, key)}
                    >
                        <span class="action-widget-more-item-icon widget-icon">
                            <i class={makeIconClass(widget.icon, true, { defaultIcon: "browser" })}></i>
                        </span>
                        <span class="action-widget-more-item-label">{widget.label}</span>
                    </div>
                )}
            </For>
        </div>
    );
};

PinnedWidgetFlyout.displayName = "PinnedWidgetFlyout";

export { PinnedWidgetFlyout };
