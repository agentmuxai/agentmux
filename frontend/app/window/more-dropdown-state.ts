// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The More dropdown's state: the region its close intent aims at, what a row click or right-click does, and the
 * hover controllers and open flyouts of its parent rows.
 */

import {
    createPeerRegistry,
    createSubmenuHover,
    type PeerRegistry,
    type SubmenuHoverController,
} from "@/app/util/submenu-hover";
import { createSignal, onCleanup } from "solid-js";
import { openWidget } from "./action-widgets-config";

/**
 * Mouse handlers of a row that has a submenu. Crossing toward another row's open submenu holds that one
 * (safe triangle) and opens this row's own once it is gone, if the pointer is still on the row.
 */
export function parentRowHover(
    key: string,
    peers: PeerRegistry,
    hover: SubmenuHoverController,
    isItemMenuOpen?: () => boolean
) {
    let pointerOnRow = false;
    return {
        enter: (e: MouseEvent) => {
            pointerOnRow = true;
            peers.closeOthers(key, e, () => {
                if (pointerOnRow) hover.onTriggerEnter();
            });
        },
        leave: (e: MouseEvent) => {
            pointerOnRow = false;
            if (isItemMenuOpen?.()) return;
            hover.onTriggerLeave(e);
        },
    };
}

export type Region = { getBoundingClientRect(): DOMRect };

/**
 * The element the More button's close intent aims at: the dropdown, and while a row's flyout is open, that flyout.
 * A pointer that leaves the dropdown crossing toward the open flyout is held (the safe triangle runs from where
 * it left to the flyout's near edge, however slowly it moves) and keeps both open once it arrives; one that
 * leaves toward anything else closes them.
 */
function createDropdownRegion(publish: ((el: Region | null) => void) | undefined) {
    let body: HTMLElement | null = null;
    const flyouts = new Map<string, HTMLElement>();
    const rect = (): DOMRect => ([...flyouts.values()].at(-1) ?? body)!.getBoundingClientRect();
    return {
        setBody(el: HTMLElement | null) {
            body = el;
            publish?.(el ? { getBoundingClientRect: rect } : null);
        },
        setFlyout(key: string, el: HTMLElement | null) {
            if (el) flyouts.set(key, el);
            else flyouts.delete(key);
        },
    };
}

export type DropdownRegion = ReturnType<typeof createDropdownRegion>;

/**
 * What a row's flyout tells the row and the dropdown around it. The flyout is part of the dropdown for the More
 * button's hover intent: entering it cancels the pending close, leaving it starts one, and while it is open it is
 * what the More button's close intent aims at.
 */
export function parentFlyoutHover(
    key: string,
    hover: SubmenuHoverController,
    region: DropdownRegion,
    more: {
        onSubmenuEnter?: () => void;
        onSubmenuLeave?: (e: MouseEvent) => void;
        isItemMenuOpen?: () => boolean;
    }
) {
    return {
        onSubmenuEnter: () => {
            hover.onSubmenuEnter();
            more.onSubmenuEnter?.();
        },
        onSubmenuLeave: (e: MouseEvent) => {
            if (more.isItemMenuOpen?.()) return;
            hover.onSubmenuLeave(e);
            more.onSubmenuLeave?.(e);
        },
        setSubmenuEl: (el: HTMLDivElement | null) => {
            hover.setSubmenuEl(el);
            region.setFlyout(key, el);
        },
    };
}

export type MoreDropdownState = ReturnType<typeof createMoreDropdown>;

export function createMoreDropdown(args: {
    onClose: () => void;
    onItemContextMenu: (pos: { x: number; y: number }, key: string) => void;
    ref?: (el: HTMLDivElement) => void;
    setSubmenuEl?: (el: Region | null) => void;
}) {
    // AnchoredPopover places it under the More button (bottom-end: right-aligned), follows chrome zoom
    // and cuts the hole in any browser pane behind it (see `frontend/app/platform/pane-overlay.ts`).
    const region = createDropdownRegion(args.setSubmenuEl);
    const registerBody = (el: HTMLDivElement) => (args.ref?.(el), region.setBody(el));
    onCleanup(() => region.setBody(null));

    const handleItemClick = (widget: WidgetConfigType, e?: MouseEvent) => {
        openWidget(widget, { forceNew: e?.shiftKey });
        args.onClose();
    };

    const handleItemContextMenu = (e: MouseEvent, key: string) => {
        e.preventDefault();
        e.stopPropagation();
        // Delegate to ActionWidgets — the item popover is rendered there so it
        // can stay open independently of this dropdown. onClose() is NOT called
        // here; the More dropdown remains visible while the item menu is open.
        args.onItemContextMenu({ x: e.clientX, y: e.clientY }, key);
    };

    // ── Parent rows (Case B, SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md §3.4) ──
    // A row whose widget has `children` expands a nested PinnedWidgetFlyout
    // (placement="right-start") instead of opening a pane — structurally the
    // same hover-intent + peer-close shape as flyoutmenu.tsx's own SubMenu,
    // scoped to this one dropdown instance.
    const parentPeers = createPeerRegistry();
    // Keyed by widget short-name, kept OUTSIDE the <For> over the rows: `widgets()` allocates fresh
    // {key, widget} objects on every config change, so <For> rebuilds every row, and disposing an open
    // controller mid-render would force-close its submenu (onClose -> closeParentSub) out from under the
    // user, e.g. during the "Pin to bar" round-trip for a nested child. parentHoverFor reuses it.
    const parentRowHoverControllers = new Map<string, SubmenuHoverController>();
    onCleanup(() => {
        for (const hover of parentRowHoverControllers.values()) hover.dispose();
        parentRowHoverControllers.clear();
    });
    const [visibleParentSubMenus, setVisibleParentSubMenus] = createSignal<Record<string, boolean>>({});
    const openParentSub = (key: string) => setVisibleParentSubMenus((prev) => ({ ...prev, [key]: true }));
    const closeParentSub = (key: string) =>
        setVisibleParentSubMenus((prev) => {
            if (!(key in prev)) return prev;
            const next = { ...prev };
            delete next[key];
            return next;
        });

    /** A parent row's hover controller: made and registered with its peers the first time, reused after. */
    const parentHoverFor = (key: string): SubmenuHoverController => {
        let hover = parentRowHoverControllers.get(key);
        if (!hover) {
            hover = createSubmenuHover({
                // Stays open when the pointer leaves; another row
                // of this dropdown closes it (closeOthers).
                closeOnLeave: false,
                onOpen: () => openParentSub(key),
                onClose: () => closeParentSub(key),
            });
            parentRowHoverControllers.set(key, hover);
            parentPeers.register(key, hover);
        }
        return hover;
    };

    return {
        region,
        registerBody,
        handleItemClick,
        handleItemContextMenu,
        parentPeers,
        visibleParentSubMenus,
        closeParentSub,
        parentHoverFor,
    };
}
