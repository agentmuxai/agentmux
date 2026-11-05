// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { Placement } from "@floating-ui/dom";
import clsx from "clsx";
import { createSignal, For, JSX, onCleanup, Show } from "solid-js";

import { AnchoredPopover } from "@/app/element/anchored-popover";
import { createPeerRegistry, createSubmenuHover, type SubmenuHoverController } from "@/app/util/submenu-hover";

import "./flyoutmenu.scss";

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
    /** An extra class on the anchor `<div>` around the trigger: the box that is the flex item in its bar. */
    anchorClass?: string;
    placement?: Placement;
    /**
     * Mirror the menu for a right-anchored (left-opening) placement: items are
     * laid out right-to-left (icon on the right, label right-aligned, chevron
     * on the left pointing left) and submenus prefer to open to the LEFT.
     * Use for menus anchored to the right edge (e.g. the macOS far-right
     * AgentMux logo). Best practice for edge-anchored menus — the disclosure arrow
     * points the same way the submenu actually opens. Default off; every other
     * menu keeps the standard left-to-right layout.
     */
    mirrored?: boolean;
    /**
     * Keep the menu inside this element (a browser pane) when it fits, and
     * with `alignToAnchorSide` open it from the side its button is on:
     * SPEC_BROWSER_PANE_THIN_BAR_AND_MENUS_IN_PANE_2026_10_18.md §5. Other
     * menus are kept inside the window only.
     */
    containWithin?: () => HTMLElement | null | undefined;
    alignToAnchorSide?: boolean;
    onOpenChange?: (isOpen: boolean) => void;
    /**
     * Receives a function that closes the menu, for an owner that must close
     * it from outside (the AgentMux logo closes when the "…" More dropdown
     * opens beside it, logo-menu.tsx).
     */
    closeRef?: (close: () => void) => void;
    children?: JSX.Element;
    renderMenu?: (subMenu: JSX.Element, props: any) => JSX.Element;
    renderMenuItem?: (item: MenuItem, props: any) => JSX.Element;
};

const FlyoutMenu = (props: MenuProps): JSX.Element => {
    const [visibleSubMenus, setVisibleSubMenus] = createSignal<{ [key: string]: any }>({});
    const [hoveredItems, setHoveredItems] = createSignal<string[]>([]);
    const [subMenuPosition, setSubMenuPosition] = createSignal<SubMenuPositionMap>({});

    const [isOpen, setIsOpen] = createSignal(false);

    let referenceEl: HTMLElement | null = null;

    const onOpenChangeMenu = (open: boolean) => {
        if (!open) {
            // The menu is leaving the DOM (AnchoredPopover stops following its
            // anchor and container with it); a reopen starts hidden again.
            setVisibleSubMenus({});
            setHoveredItems([]);
            setSubMenuPosition({});
        }
        setIsOpen(open);
        props.onOpenChange?.(open);
    };
    props.closeRef?.(() => {
        if (isOpen()) onOpenChangeMenu(false);
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
        if (!item.keepOpen) onOpenChangeMenu(false);
        item.onClick?.(e);
    };

    return (
        <>
            <div
                ref={(el) => { referenceEl = el; }}
                class={clsx("menu-anchor", props.anchorClass)}
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
                <MenuBody
                    className={props.className}
                    mirrored={props.mirrored}
                    anchor={referenceEl}
                    placement={props.placement ?? "bottom-start"}
                    containWithin={props.containWithin}
                    alignToAnchorSide={props.alignToAnchorSide}
                    onDismiss={() => onOpenChangeMenu(false)}
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
    anchor: HTMLElement | null;
    placement: Placement;
    containWithin?: () => HTMLElement | null | undefined;
    alignToAnchorSide?: boolean;
    /** An outside press or Esc. */
    onDismiss: () => void;
    items: MenuItem[];
};

/**
 * The top level of a menu: one AnchoredPopover, which places it at its anchor, follows chrome zoom
 * (so the menu is the size of the button that opened it), cuts the hole in any native browser pane
 * behind it (on every platform, and dropping it while its window tab is hidden) and dismisses it on
 * an outside press or Esc. It starts hidden and is shown once placed, so a reopened menu never
 * flashes at the corner or where it was last.
 */
const MenuBody = (props: MenuBodyProps): JSX.Element => (
    <AnchoredPopover
        anchor={props.anchor}
        placement={props.placement}
        containWithin={props.containWithin}
        alignToAnchorSide={props.alignToAnchorSide}
        fitHeight
        hideUntilPlaced
        shellClass="anchored-popover--menu"
        class={clsx("menu", props.className, { "menu--mirrored": props.mirrored })}
        onDismiss={() => props.onDismiss()}
        // A press in a submenu (its own panel, mounted apart from this one) is inside.
        keepOpenOn={(t) => !!(t instanceof Element ? t : t.parentElement)?.closest(".menu, .sub-menu")}
    >
        <MenuRows {...props} parentKey={null} />
    </AnchoredPopover>
);

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
                          // Stays open when the pointer leaves; a sibling
                          // row's peers.closeOthers below closes it.
                          closeOnLeave: false,
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
                            <i class="fa-solid fa-chevron-right" />
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

    // Anchored to the parent row's rect, preferred placement right-start: flip() turns that into
    // left-start near the right edge, shift() pulls it back inside near the bottom edge. A mirrored
    // (right-anchored) menu prefers the LEFT. AnchoredPopover places it, follows chrome zoom and cuts
    // the hole in any native pane behind it; it stays hidden until placed (SPEC_SUBMENU_POSITIONING_
    // AND_HOVER_TIMING_2026_08_10 §1.1: no flash at the upper left).
    const subMenu = (
        <Show when={position()}>
            {(pos) => (
                <AnchoredPopover
                    anchor={pos().anchorRect}
                    placement={props.mirrored ? "left-start" : "right-start"}
                    fitHeight
                    hideUntilPlaced
                    overlayKeys
                    shellClass="anchored-popover--menu"
                    class={clsx("menu sub-menu", { "menu--mirrored": props.mirrored })}
                    ref={(el) => {
                        props.hover.setSubmenuEl(el);
                        el.addEventListener("mouseenter", () => props.hover.onSubmenuEnter());
                        el.addEventListener("mouseleave", (e) => props.hover.onSubmenuLeave(e));
                    }}
                >
                    <MenuRows {...props} items={props.subItems} parentKey={props.parentKey} />
                </AnchoredPopover>
            )}
        </Show>
    );

    return props.renderMenu ? props.renderMenu(subMenu, { parentKey: props.parentKey }) : subMenu;
};

export { FlyoutMenu };
