// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MoreDropdown — floating overflow menu for the widget bar. Anchored to the
 * "More" button; lists widgets that aren't pinned (plus any pinned widgets
 * clipped by the responsive collapse). Extracted from action-widgets.tsx.
 */

import { AnchoredPopover } from "@/app/element/anchored-popover";
import {
    createPeerRegistry,
    createSubmenuHover,
    type PeerRegistry,
    type SubmenuHoverController,
} from "@/app/util/submenu-hover";
import { makeIconClass } from "@/util/util";
import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { openWidget, widgetIconStyle } from "./action-widgets-config";
import { PinnedWidgetFlyout } from "./pinned-widget-flyout";

/**
 * Mouse handlers of a row that has a submenu. Crossing toward another row's open submenu holds that one
 * (safe triangle) and opens this row's own once it is gone, if the pointer is still on the row.
 */
function parentRowHover(
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

const MoreDropdown = ({
    widgets,
    onClose,
    onItemContextMenu,
    anchor,
    settings,
    wmap,
    ref,
    onSubmenuEnter,
    onSubmenuLeave,
    setSubmenuEl,
    isItemMenuOpen,
}: {
    widgets: () => { key: string; widget: WidgetConfigType }[];
    onClose: () => void;
    onItemContextMenu: (pos: { x: number; y: number }, key: string) => void;
    anchor: () => HTMLElement | null;
    settings: () => Record<string, any>;
    wmap: () => Record<string, WidgetConfigType>;
    ref?: (el: HTMLDivElement) => void;
    /**
     * Optional hover-controller plumbing — present when the caller opens
     * this dropdown via hover-intent (a createSubmenuHover on the "More"
     * button itself) rather than click-only, so moving the cursor into this
     * panel cancels the pending close the same way any other hover-opened
     * flyout/submenu in the app does.
     */
    onSubmenuEnter?: () => void;
    onSubmenuLeave?: (e: MouseEvent) => void;
    setSubmenuEl?: (el: HTMLDivElement | null) => void;
    /**
     * True while the right-click item popover (pin/unpin, open-in-new-
     * window, ...) is open. A right-click's popover renders at the cursor
     * position, which makes the browser re-target hit-testing and fire a
     * spurious mouseleave on whatever's underneath even though the cursor
     * never actually moved — without this guard that leave reaches the
     * per-row hover controller for a Case B parent row (e.g. Messengers
     * nested inside More) and starts a close-intent, collapsing it out from
     * under the user mid right-click.
     */
    isItemMenuOpen?: () => boolean;
}): JSX.Element => {
    // AnchoredPopover places it under the More button (bottom-end: right-aligned), follows chrome zoom
    // and cuts the hole in any browser pane behind it (see `frontend/app/platform/pane-overlay.ts`).
    const registerBody = (el: HTMLDivElement) => {
        ref?.(el);
        setSubmenuEl?.(el);
    };

    onCleanup(() => setSubmenuEl?.(null));

    const handleItemClick = (widget: WidgetConfigType, e?: MouseEvent) => {
        openWidget(widget, { forceNew: e?.shiftKey });
        onClose();
    };

    const handleItemContextMenu = (e: MouseEvent, key: string) => {
        e.preventDefault();
        e.stopPropagation();
        // Delegate to ActionWidgets — the item popover is rendered there so it
        // can stay open independently of this dropdown. onClose() is NOT called
        // here; the More dropdown remains visible while the item menu is open.
        onItemContextMenu({ x: e.clientX, y: e.clientY }, key);
    };

    // ── Parent rows (Case B, SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md §3.4) ──
    // A row whose widget has `children` expands a nested PinnedWidgetFlyout
    // (placement="right-start") instead of opening a pane — structurally the
    // same hover-intent + peer-close shape as flyoutmenu.tsx's own SubMenu,
    // scoped to this one dropdown instance.
    const parentPeers = createPeerRegistry();
    // Keyed by widget short-name, deliberately OUTSIDE the <For> below —
    // see the reuse-not-recreate comment at its one call site (reagent P1
    // on this PR): `widgets()` (clippedPinnedWidgets + moreWidgets)
    // allocates fresh {key, widget} object literals on every
    // fullConfigAtom change, so Solid's reference-diffing <For> tears down
    // and rebuilds every row on any config change — including this PR's
    // own "Pin to bar" RPC round-trip for a child inside an open nested
    // submenu. Disposing an open controller mid-render would force-close
    // that submenu via its onClose (closeParentSub) out from under the
    // user.
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

    return (
        <AnchoredPopover
            anchor={anchor()}
            placement="bottom-end"
            gutter={4}
            hideUntilPlaced
            onDismiss={(reason) => reason === "escape" && onClose()}
            shellClass="anchored-popover--menu"
            class="action-widget-more-dropdown"
            ref={(el) => {
                registerBody(el);
                el.addEventListener("mouseenter", () => onSubmenuEnter?.());
                el.addEventListener("mouseleave", (e) => {
                    if (isItemMenuOpen?.()) return;
                    onSubmenuLeave?.(e);
                });
            }}
        >
            <For each={widgets()}>
                {({ key, widget }) => {
                    const isParent = (widget.children?.length ?? 0) > 0;
                    if (!isParent) {
                        return (
                            <div
                                class="action-widget-more-item"
                                onMouseEnter={(e) => parentPeers.closeOthers(key, e)}
                                onClick={(e) => handleItemClick(widget, e)}
                                onContextMenu={(e) => handleItemContextMenu(e, key)}
                            >
                                <span class="action-widget-more-item-icon widget-icon" style={widgetIconStyle(widget)}>
                                    <i class={makeIconClass(widget.icon, true, { defaultIcon: "window-maximize" })}></i>
                                </span>
                                <span class="action-widget-more-item-label">{widget.label}</span>
                            </div>
                        );
                    }

                    let rowEl: HTMLDivElement | undefined;
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

                    const rowHover = parentRowHover(key, parentPeers, hover, isItemMenuOpen);

                    return (
                        <>
                            <div
                                ref={(el) => (rowEl = el)}
                                class="action-widget-more-item"
                                onMouseEnter={rowHover.enter}
                                onMouseLeave={rowHover.leave}
                                onContextMenu={(e) => handleItemContextMenu(e, key)}
                            >
                                <span class="action-widget-more-item-icon widget-icon" style={widgetIconStyle(widget)}>
                                    <i class={makeIconClass(widget.icon, true, { defaultIcon: "window-maximize" })}></i>
                                </span>
                                <span class="action-widget-more-item-label">{widget.label}</span>
                                <i class="fa-solid fa-chevron-right action-widget-more-item-chevron" />
                            </div>
                            <Show when={visibleParentSubMenus()[key]}>
                                <PinnedWidgetFlyout
                                    widget={widget}
                                    wmap={wmap}
                                    settings={settings}
                                    onClose={() => {
                                        closeParentSub(key);
                                        hover.close();
                                        onClose();
                                    }}
                                    onItemContextMenu={onItemContextMenu}
                                    anchor={() => rowEl ?? null}
                                    placement="right-start"
                                    gutter={8}
                                    onSubmenuEnter={() => hover.onSubmenuEnter()}
                                    onSubmenuLeave={(e) => {
                                        if (isItemMenuOpen?.()) return;
                                        hover.onSubmenuLeave(e);
                                    }}
                                    setSubmenuEl={(el) => hover.setSubmenuEl(el)}
                                />
                            </Show>
                        </>
                    );
                }}
            </For>
        </AnchoredPopover>
    );
};

MoreDropdown.displayName = "MoreDropdown";

export { MoreDropdown };
