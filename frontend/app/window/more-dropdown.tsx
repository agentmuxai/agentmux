// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MoreDropdown — floating overflow menu for the widget bar. Anchored to the
 * "More" button; lists widgets that aren't pinned (plus any pinned widgets
 * clipped by the responsive collapse). Extracted from action-widgets.tsx.
 */

import { usePaneOverlay } from "@/app/platform/pane-overlay";
import {
    assertMenuInPaintableArea,
    computeMenuPosition,
} from "@/app/util/menu-position";
import { createPeerRegistry, createSubmenuHover } from "@/app/util/submenu-hover";
import { makeIconClass } from "@/util/util";
import { autoUpdate } from "@floating-ui/dom";
import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { handleWidgetSelect } from "./action-widgets-config";
import { PinnedWidgetFlyout } from "./pinned-widget-flyout";

const MoreDropdown = ({
    widgets,
    onClose,
    onItemContextMenu,
    anchor,
    settings,
    wmap,
    ref,
}: {
    widgets: () => { key: string; widget: WidgetConfigType }[];
    onClose: () => void;
    onItemContextMenu: (pos: { x: number; y: number }, key: string) => void;
    anchor: () => HTMLElement | null;
    settings: () => Record<string, any>;
    wmap: () => Record<string, WidgetConfigType>;
    ref?: (el: HTMLDivElement) => void;
}): JSX.Element => {
    let overlayEl: HTMLDivElement | undefined;
    // Cut a transparent hole through any browser pane HWND behind this
    // dropdown so DOM renders above the pane in this rect.
    // See `frontend/app/platform/pane-overlay.ts`.
    usePaneOverlay(() => overlayEl);

    // Positioning routes through the shared primitive (Phase 3): anchored to
    // the More button, preferred placement bottom-end (right-aligned, as
    // before). flip/shift/size + the paintable-area boundary replace the old
    // hand-rolled `window.innerWidth - rect.right` clamp.
    const [floatingStyle, setFloatingStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
    });
    let cleanupAutoUpdate: (() => void) | null = null;

    const registerFloating = (el: HTMLDivElement) => {
        overlayEl = el;
        ref?.(el);
        requestAnimationFrame(() => {
            const anchorEl = anchor();
            if (!(anchorEl instanceof Element) || !(el instanceof Element)) return;
            const update = async () => {
                const a = anchor();
                if (!a) return;
                // avoidNativePanes:false — this dropdown renders with
                // `data-pane-overlay` (+ usePaneOverlay), which clips a hole
                // through any native browser/webview pane behind it so the DOM
                // draws on top. So it should open in place under the More button,
                // not get pushed to the window edge into the largest pane-free
                // rect. Matches FlyoutMenu / showJsContextMenu.
                const pos = await computeMenuPosition(
                    { anchor: a, placement: "bottom-end", gutter: 4, avoidNativePanes: false },
                    el,
                );
                setFloatingStyle(pos.style);
            };
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = autoUpdate(anchorEl, el, update);
            // Dev-only paintable-area guard (spec §6.1).
            assertMenuInPaintableArea(el, "more-dropdown");
        });
    };

    onCleanup(() => cleanupAutoUpdate?.());

    const handleItemClick = (widget: WidgetConfigType) => {
        handleWidgetSelect(widget);
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
        <div
            ref={registerFloating}
            class="action-widget-more-dropdown"
            style={floatingStyle()}
            data-pane-overlay
        >
            <For each={widgets()}>
                {({ key, widget }) => {
                    const isParent = (widget.children?.length ?? 0) > 0;
                    if (!isParent) {
                        return (
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
                        );
                    }

                    let rowEl: HTMLDivElement | undefined;
                    const hover = createSubmenuHover({
                        onOpen: () => openParentSub(key),
                        onClose: () => closeParentSub(key),
                    });
                    const unregister = parentPeers.register(key, hover);
                    onCleanup(() => {
                        unregister();
                        hover.dispose();
                    });

                    return (
                        <>
                            <div
                                ref={(el) => (rowEl = el)}
                                class="action-widget-more-item"
                                onMouseEnter={() => {
                                    parentPeers.closeOthers(key);
                                    hover.onTriggerEnter();
                                }}
                                onMouseLeave={(e) => hover.onTriggerLeave(e)}
                                onContextMenu={(e) => handleItemContextMenu(e, key)}
                            >
                                <span class="action-widget-more-item-icon widget-icon">
                                    <i class={makeIconClass(widget.icon, true, { defaultIcon: "browser" })}></i>
                                </span>
                                <span class="action-widget-more-item-label">{widget.label}</span>
                                <i class="fa-sharp fa-solid fa-chevron-right action-widget-more-item-chevron" />
                            </div>
                            <Show when={visibleParentSubMenus()[key]}>
                                <PinnedWidgetFlyout
                                    widget={widget}
                                    wmap={wmap}
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
                                    onSubmenuLeave={(e) => hover.onSubmenuLeave(e)}
                                    setSubmenuEl={(el) => hover.setSubmenuEl(el)}
                                />
                            </Show>
                        </>
                    );
                }}
            </For>
        </div>
    );
};

MoreDropdown.displayName = "MoreDropdown";

export { MoreDropdown };
