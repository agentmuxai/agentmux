// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import {
    autoUpdate,
    type Placement,
} from "@floating-ui/dom";
import clsx from "clsx";
import { createSignal, For, JSX, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";

import { usePaneOverlay } from "@/app/platform/pane-overlay";
import {
    assertMenuInPaintableArea,
    computeMenuPosition,
    type MenuPositionResult,
} from "@/app/util/menu-position";
import { createPeerRegistry, createSubmenuHover, type SubmenuHoverController } from "@/app/util/submenu-hover";

import "./flyoutmenu.scss";

/**
 * Serialize a MenuPositionResult (position:fixed + left/top + size cap) to a
 * CSS string. The size() max-height cap makes a menu taller than the free
 * space scroll internally instead of overflowing the window. max-width is
 * deliberately NOT applied — an inline max-width would override (and can
 * loosen) the .menu 400px CSS cap, and horizontal fit is already guaranteed
 * by flip+shift for menus at or under that cap. justify-content is forced to
 * flex-start because the .menu rule's flex-end makes overflow unreachable in
 * a scroll container (a no-op when the menu fits).
 *
 * Deliberately omits `visibility` — the caller's placeholder style carries
 * `visibility:hidden` and this full-string replacement drops it once a real
 * position is known, which is what reveals the menu already in the right
 * spot instead of flashing at the placeholder coordinates first
 * (SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_2026_08_10 §1.1).
 */
function styleToString(pos: MenuPositionResult): string {
    const s = pos.style;
    return (
        `position:${s.position};left:${s.left};top:${s.top};` +
        `max-height:${pos.maxHeight}px;overflow-y:auto;justify-content:flex-start`
    );
}

// createPeerRegistry moved to @/app/util/submenu-hover (hoisted so
// action-widgets.tsx's pinned-parent flyouts (Case A, SPEC_WIDGET_BAR_
// PARENT_SUBMENUS_2026_08_12.md §3.3) can share the same "close my
// siblings" behavior outside of FlyoutMenu). Scoped one per menu level
// (one per MenuBody, one per SubMenu) here — peers never reach across
// levels, so a still-open descendant closes naturally via Solid unmounting
// it when its own ancestor's <Show> flips off, not through this registry.

type MenuProps = {
    items: MenuItem[];
    className?: string;
    placement?: Placement;
    /**
     * Mirror the menu for a right-anchored (left-opening) placement: items are
     * laid out right-to-left (icon on the right, label right-aligned, chevron
     * on the left pointing left) and submenus prefer to open to the LEFT.
     * Use for menus anchored to the right edge (e.g. the macOS far-right
     * hamburger). Best practice for edge-anchored menus — the disclosure arrow
     * points the same way the submenu actually opens. Default off; every other
     * menu keeps the standard left-to-right layout.
     */
    mirrored?: boolean;
    onOpenChange?: (isOpen: boolean) => void;
    children?: JSX.Element;
    renderMenu?: (subMenu: JSX.Element, props: any) => JSX.Element;
    renderMenuItem?: (item: MenuItem, props: any) => JSX.Element;
};

const FlyoutMenu = (props: MenuProps): JSX.Element => {
    const [visibleSubMenus, setVisibleSubMenus] = createSignal<{ [key: string]: any }>({});
    const [hoveredItems, setHoveredItems] = createSignal<string[]>([]);
    const [subMenuPosition, setSubMenuPosition] = createSignal<SubMenuPositionMap>({});

    const [isOpen, setIsOpen] = createSignal(false);
    const [floatingStyle, setFloatingStyle] = createSignal(
        "position:absolute;left:0px;top:0px;visibility:hidden",
    );

    let referenceEl: HTMLElement | null = null;
    let floatingEl: HTMLElement | null = null;
    let cleanupAutoUpdate: (() => void) | null = null;

    const onOpenChangeMenu = (open: boolean) => {
        if (!open) {
            setVisibleSubMenus({});
            setHoveredItems([]);
            setSubMenuPosition({});
        }
        setIsOpen(open);
        props.onOpenChange?.(open);
    };

    const updatePosition = async () => {
        if (!referenceEl || !floatingEl) return;
        // avoidNativePanes:false — this menu always renders with
        // `data-pane-overlay` (below), which clips a hole through any native
        // browser/webview pane HWND so the DOM menu shows on top. So it should
        // open in place at its anchor, NOT be pushed into the largest pane-free
        // rect (which shoved the menu to the window edge whenever a browser/
        // slack/webview pane sat under the anchor). Matches showJsContextMenu
        // and the statusbar popovers.
        const pos = await computeMenuPosition(
            { anchor: referenceEl, placement: props.placement ?? "bottom-start", avoidNativePanes: false },
            floatingEl,
        );
        setFloatingStyle(styleToString(pos));
    };

    const registerFloating = (el: HTMLElement) => {
        floatingEl = el;
        requestAnimationFrame(() => {
            if (!(referenceEl instanceof Element) || !(floatingEl instanceof Element)) return;
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = autoUpdate(referenceEl, floatingEl, updatePosition);
            // Dev-only paintable-area guard (spec §6.1); gated so it is
            // zero-cost in release builds.
            assertMenuInPaintableArea(el, "flyout-menu");
        });
    };

    const handleClickOutside = (e: MouseEvent) => {
        if (!isOpen()) return;
        const target = e.target as Node;
        if (referenceEl?.contains(target) || floatingEl?.contains(target)) return;
        const el = target instanceof Element ? target : (target as Node).parentElement;
        if (el?.closest(".menu, .sub-menu")) return;
        onOpenChangeMenu(false);
    };

    onMount(() => {
        document.addEventListener("mousedown", handleClickOutside);
    });

    onCleanup(() => {
        document.removeEventListener("mousedown", handleClickOutside);
        cleanupAutoUpdate?.();
    });

    const handleSubMenuPosition = (key: string, itemRect: DOMRect, label: string) => {
        // Store the parent menu item's rect; the SubMenu component routes it
        // through computeMenuPosition (right-start, flips to left-start near
        // the right edge) — no hand-rolled window-edge math here.
        setSubMenuPosition((prev) => ({ ...prev, [key]: { anchorRect: itemRect, label } }));
    };

    // Whether a submenu is SHOWN and when it opens/closes now routes entirely
    // through each row's own createSubmenuHover controller (open-delay +
    // safe-triangle close — SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_2026_08_10
    // §4). These two are the controllers' onOpen/onClose targets; nothing else
    // writes to visibleSubMenus. Closing deletes the key (rather than setting
    // visible:false) so every write is a fresh object — no shared, in-place
    // mutation of a previous render's state object.
    const openSubMenu = (key: string, label: string) => {
        setVisibleSubMenus((prev) => ({ ...prev, [key]: { visible: true, label } }));
    };
    const closeSubMenu = (key: string) => {
        setVisibleSubMenus((prev) => {
            if (!(key in prev)) return prev;
            const next = { ...prev };
            delete next[key];
            return next;
        });
    };

    // Highlighting (hoveredItems) and the submenu's anchor rect only — NOT
    // open/close, which is each row's own hover controller's job now.
    const handleMouseEnterItem = (
        event: MouseEvent,
        parentKey: string | null,
        index: number,
        item: MenuItem
    ) => {
        event.stopPropagation();

        const key = parentKey ? `${parentKey}-${index}` : `${index}`;

        const newHoveredItems = key.split("-").reduce((acc: string[], part, idx) => {
            if (idx === 0) return [part];
            return [...acc, `${acc[idx - 1]}-${part}`];
        }, []);
        setHoveredItems(newHoveredItems);

        const itemRect = (event.currentTarget as HTMLElement).getBoundingClientRect();
        handleSubMenuPosition(key, itemRect, item.label);
    };

    const handleOnClick = (e: MouseEvent, item: MenuItem) => {
        e.stopPropagation();
        if (item.subItems) {
            return;
        }
        onOpenChangeMenu(false);
        item.onClick?.(e);
    };

    return (
        <>
            <div
                ref={(el) => { referenceEl = el; }}
                class="menu-anchor"
                data-drag-region="false"
                onPointerDown={() => {
                    // Pre-warm pane freeze-frames ~100ms before the click
                    // completes and the menu opens: browser panes capture
                    // their snapshot now, so the airspace hide (which waits
                    // for the frame to be painted) releases almost
                    // immediately when the menu's overlay rect registers.
                    // See the freeze-frame block in browser-view.tsx.
                    if (!isOpen()) {
                        window.dispatchEvent(new CustomEvent("pane-freeze-prewarm"));
                    }
                }}
                onClick={() => onOpenChangeMenu(!isOpen())}
            >
                {props.children}
            </div>
            <Show when={isOpen()}>
                <Portal>
                    <MenuBody
                        className={props.className}
                        mirrored={props.mirrored}
                        style={floatingStyle()}
                        registerFloating={registerFloating}
                        items={props.items}
                        hoveredItems={hoveredItems()}
                        visibleSubMenus={visibleSubMenus()}
                        subMenuPosition={subMenuPosition()}
                        handleMouseEnterItem={handleMouseEnterItem}
                        handleOnClick={handleOnClick}
                        openSubMenu={openSubMenu}
                        closeSubMenu={closeSubMenu}
                        renderMenu={props.renderMenu}
                        renderMenuItem={props.renderMenuItem}
                    />
                </Portal>
            </Show>
        </>
    );
};

/**
 * Props every menu level needs and forwards unchanged to the next one.
 * FlyoutMenu owns all of this state; MenuBody, MenuRows and SubMenu only
 * pass it down.
 */
type MenuLevelProps = {
    hoveredItems: string[];
    visibleSubMenus: { [key: string]: any };
    subMenuPosition: SubMenuPositionMap;
    handleMouseEnterItem: (
        event: MouseEvent,
        parentKey: string | null,
        index: number,
        item: MenuItem
    ) => void;
    handleOnClick: (e: MouseEvent, item: MenuItem) => void;
    openSubMenu: (key: string, label: string) => void;
    closeSubMenu: (key: string) => void;
    renderMenu?: (subMenu: JSX.Element, props: any) => JSX.Element;
    renderMenuItem?: (item: MenuItem, props: any) => JSX.Element;
    mirrored?: boolean;
};

type MenuBodyProps = MenuLevelProps & {
    className?: string;
    style: string;
    registerFloating: (el: HTMLElement) => void;
    items: MenuItem[];
};

/**
 * Split out from FlyoutMenu's render (rather than inlined under <Show>) so
 * `usePaneOverlay` mounts fresh on every open — mirrors SubMenu below.
 * `data-pane-overlay` alone isn't enough here: the auto-discovery service
 * that watches for it is gated to Windows only (pane-overlay-auto.ts), so
 * on macOS/Linux this explicit hook call is what actually clips the
 * native browser pane behind the menu (same belt-and-suspenders pattern
 * as MoreDropdown in action-widgets.tsx).
 */
const MenuBody = (props: MenuBodyProps): JSX.Element => {
    let menuEl: HTMLDivElement | undefined;
    usePaneOverlay(() => menuEl);

    const registerFloating = (el: HTMLDivElement) => {
        menuEl = el;
        props.registerFloating(el);
    };

    return (
        <div
            class={clsx("menu", props.className, { "menu--mirrored": props.mirrored })}
            ref={registerFloating}
            style={props.style}
            data-pane-overlay
        >
            <MenuRows {...props} parentKey={null} />
        </div>
    );
};

type MenuRowsProps = MenuLevelProps & {
    items: MenuItem[];
    /** `null` for the top-level menu; the owning row's key inside a submenu. */
    parentKey: string | null;
};

/**
 * One level of menu rows. MenuBody (the top level) and SubMenu (every
 * nested level) render exactly this; the only per-level differences are
 * the row-key scheme — `${index}` at the top, `${parentKey}-${index}`
 * below — and the parentKey reported to handleMouseEnterItem, both
 * derived from `parentKey` here. Row keys are what FlyoutMenu's
 * hoveredItems / visibleSubMenus / subMenuPosition maps are keyed on,
 * so the scheme must not change.
 */
const MenuRows = (props: MenuRowsProps): JSX.Element => {
    // One peer registry per level: opening a row's submenu closes any
    // sibling's, never a cousin's.
    const peers = createPeerRegistry();

    return (
        <For each={props.items}>
            {(item, index) => {
                const key = props.parentKey === null ? `${index()}` : `${props.parentKey}-${index()}`;
                const isActive = () => props.hoveredItems.includes(key);

                // One hover-intent controller per row that has a submenu,
                // for the lifetime of this row in the list (disposed when
                // the row leaves the array or the whole menu unmounts).
                // Open/close timing (delay + safe-triangle) lives here;
                // rendering the submenu is still driven by visibleSubMenus
                // via openSubMenu/closeSubMenu, which this controller is
                // the only caller of for this key.
                const hover: SubmenuHoverController | null = item.subItems
                    ? createSubmenuHover({
                          onOpen: () => props.openSubMenu(key, item.label),
                          onClose: () => props.closeSubMenu(key),
                      })
                    : null;
                if (hover) onCleanup(peers.register(key, hover));
                onCleanup(() => hover?.dispose());

                const menuItemProps = {
                    class: clsx("menu-item", { active: isActive() }),
                    onMouseEnter: (event: MouseEvent) => {
                        props.handleMouseEnterItem(event, props.parentKey, index(), item);
                        // An explicit new selection at this level closes any
                        // other open peer submenu right away — no triangle to
                        // protect toward a row the cursor isn't heading for.
                        peers.closeOthers(key);
                        hover?.onTriggerEnter();
                    },
                    onMouseLeave: (event: MouseEvent) => hover?.onTriggerLeave(event),
                    onClick: (e: MouseEvent) => props.handleOnClick(e, item),
                };

                if (item.divider) {
                    return <div class="menu-divider" aria-hidden="true" />;
                }

                const renderedItem = props.renderMenuItem ? (
                    props.renderMenuItem(item, menuItemProps)
                ) : (
                    <div {...menuItemProps}>
                        <Show
                            when={item.checked === undefined}
                            fallback={
                                <i
                                    class={clsx(
                                        "fa-solid fa-fw menu-item-icon menu-item-check",
                                        { "fa-check": item.checked === true },
                                    )}
                                />
                            }
                        >
                            <Show when={item.icon}>
                                <i class={clsx("fa-solid fa-fw", `fa-${item.icon}`, "menu-item-icon")} />
                            </Show>
                        </Show>
                        <span class="label">{item.label}</span>
                        <Show when={item.shortcut && !item.subItems}>
                            <span class="menu-item-shortcut">{item.shortcut}</span>
                        </Show>
                        <Show when={item.subItems}>
                            <i class="fa-sharp fa-solid fa-chevron-right" />
                        </Show>
                    </div>
                );

                return (
                    <>
                        {renderedItem}
                        <Show when={props.visibleSubMenus[key]?.visible && item.subItems && hover}>
                            <SubMenu {...props} subItems={item.subItems!} parentKey={key} hover={hover!} />
                        </Show>
                    </>
                );
            }}
        </For>
    );
};

type SubMenuPositionMap = {
    [key: string]: { anchorRect: DOMRect; label: string };
};

type SubMenuProps = MenuLevelProps & {
    subItems: MenuItem[];
    parentKey: string;
    /** This SubMenu's own hover controller (owned/disposed by the row that renders it). */
    hover: SubmenuHoverController;
};

const SubMenu = (props: SubMenuProps): JSX.Element => {
    const position = () => props.subMenuPosition[props.parentKey];

    // Submenu positioning routes through the Phase 1 primitive: anchored to the
    // parent menu item's rect, preferred placement right-start — flip() turns
    // that into left-start near the right edge, shift() pulls it back inside
    // near the bottom edge, all paintable-area aware. autoUpdate keeps it live
    // on scroll/resize (the old one-shot `flipped` flag never re-evaluated).
    //
    // Starts `visibility:hidden` and only sheds it once computeMenuPosition
    // resolves (styleToString's output never includes `visibility`, so the
    // full style-string replacement below drops it) — otherwise this panel
    // paints fully visible at the placeholder (0,0) for the frame(s) before
    // the real position commits (SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_
    // 2026_08_10 §1.1, the confirmed primary cause of the upper-left flash).
    const [subStyle, setSubStyle] = createSignal("position:fixed;left:0px;top:0px;visibility:hidden");
    let cleanupAutoUpdate: (() => void) | null = null;
    let subMenuEl: HTMLDivElement | undefined;

    // Same rationale as MenuBody above — the submenu also carries
    // `data-pane-overlay`, but macOS/Linux auto-discovery is gated off, so
    // it needs this explicit call to actually occlude the browser pane.
    usePaneOverlay(() => subMenuEl);

    const registerSubMenu = (el: HTMLDivElement) => {
        subMenuEl = el;
        // Registered immediately (not gated on positioning) — the safe-
        // triangle test degrades gracefully on a not-yet-positioned/zero-size
        // rect (submenu-hover.ts falls back to a plain delay) and picks up
        // real geometry as soon as `update()` below commits it.
        props.hover.setSubmenuEl(el);
        requestAnimationFrame(() => {
            const pos = position();
            if (!pos || !(el instanceof Element)) return;
            const update = async () => {
                const cur = position();
                if (!cur) return;
                // Mirrored (right-anchored) menus prefer to open submenus to
                // the LEFT; flip() still sends them right if there's no room.
                // Standard menus prefer right, flipping left near the edge.
                // avoidNativePanes:false — same rationale as the top-level menu:
                // the submenu also carries `data-pane-overlay` and must open at
                // its parent row, not get pushed off toward the window edge when
                // a native pane sits behind it.
                const computed = await computeMenuPosition(
                    {
                        anchor: cur.anchorRect,
                        placement: props.mirrored ? "left-start" : "right-start",
                        avoidNativePanes: false,
                    },
                    el,
                );
                setSubStyle(styleToString(computed));
            };
            cleanupAutoUpdate?.();
            // anchorRect is a static DOMRect, so autoUpdate's reference is a
            // virtual element; it still re-runs on scroll/resize.
            cleanupAutoUpdate = autoUpdate(
                { getBoundingClientRect: () => position()?.anchorRect ?? pos.anchorRect },
                el,
                update,
            );
            // Dev-only paintable-area guard (spec §6.1).
            assertMenuInPaintableArea(el, "submenu");
        });
    };

    onCleanup(() => cleanupAutoUpdate?.());

    const subMenu = (
        <div
            ref={registerSubMenu}
            class={clsx("menu sub-menu", { "menu--mirrored": props.mirrored })}
            style={subStyle()}
            data-pane-overlay
            onMouseEnter={() => props.hover.onSubmenuEnter()}
            onMouseLeave={(e) => props.hover.onSubmenuLeave(e)}
        >
            <MenuRows {...props} items={props.subItems} parentKey={props.parentKey} />
        </div>
    );

    return (
        <Portal>
            {props.renderMenu ? props.renderMenu(subMenu, { parentKey: props.parentKey }) : subMenu}
        </Portal>
    );
};

export { FlyoutMenu };
