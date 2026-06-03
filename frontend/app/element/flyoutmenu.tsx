// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import {
    autoUpdate,
    type Placement,
} from "@floating-ui/dom";
import clsx from "clsx";
import { createSignal, For, JSX, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";

import {
    assertMenuInPaintableArea,
    computeMenuPosition,
    type MenuPositionResult,
} from "@/app/util/menu-position";

import "./flyoutmenu.scss";

/** Serialize a MenuPositionResult.style (position:fixed + left/top) to a CSS string. */
function styleToString(s: MenuPositionResult["style"]): string {
    return `position:${s.position};left:${s.left};top:${s.top}`;
}

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
    const [floatingStyle, setFloatingStyle] = createSignal("position:absolute;left:0px;top:0px");

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
        const pos = await computeMenuPosition(
            { anchor: referenceEl, placement: props.placement ?? "bottom-start" },
            floatingEl,
        );
        setFloatingStyle(styleToString(pos.style));
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
        // through useMenuPosition (right-start, flips to left-start near the
        // right edge) — no hand-rolled window-edge math here.
        setSubMenuPosition((prev) => ({ ...prev, [key]: { anchorRect: itemRect, label } }));
    };

    const handleMouseEnterItem = (
        event: MouseEvent,
        parentKey: string | null,
        index: number,
        item: MenuItem
    ) => {
        event.stopPropagation();

        const key = parentKey ? `${parentKey}-${index}` : `${index}`;

        setVisibleSubMenus((prev) => {
            const updatedState = { ...prev };
            updatedState[key] = { visible: true, label: item.label };

            const ancestors = key.split("-").reduce((acc: string[], part, idx) => {
                if (idx === 0) return [part];
                return [...acc, `${acc[idx - 1]}-${part}`];
            }, []);

            ancestors.forEach((ancestorKey) => {
                if (updatedState[ancestorKey]) {
                    updatedState[ancestorKey].visible = true;
                }
            });

            for (const pkey in updatedState) {
                if (!ancestors.includes(pkey) && pkey !== key) {
                    updatedState[pkey].visible = false;
                }
            }

            return updatedState;
        });

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
                onClick={() => onOpenChangeMenu(!isOpen())}
            >
                {props.children}
            </div>
            <Show when={isOpen()}>
                <Portal>
                    <div
                        class={clsx("menu", props.className, { "menu--mirrored": props.mirrored })}
                        ref={registerFloating}
                        style={floatingStyle()}
                        data-pane-overlay
                    >
                        <For each={props.items}>
                            {(item, index) => {
                                const key = `${index()}`;
                                const isActive = () => hoveredItems().includes(key);

                                const menuItemProps = {
                                    class: clsx("menu-item", { active: isActive() }),
                                    onMouseEnter: (event: MouseEvent) =>
                                        handleMouseEnterItem(event, null, index(), item),
                                    onClick: (e: MouseEvent) => handleOnClick(e, item),
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
                                        <Show when={visibleSubMenus()[key]?.visible && item.subItems}>
                                            <SubMenu
                                                subItems={item.subItems!}
                                                parentKey={key}
                                                subMenuPosition={subMenuPosition()}
                                                setSubMenuPosition={setSubMenuPosition}
                                                visibleSubMenus={visibleSubMenus()}
                                                hoveredItems={hoveredItems()}
                                                handleMouseEnterItem={handleMouseEnterItem}
                                                handleOnClick={handleOnClick}
                                                renderMenu={props.renderMenu}
                                                renderMenuItem={props.renderMenuItem}
                                                mirrored={props.mirrored}
                                            />
                                        </Show>
                                    </>
                                );
                            }}
                        </For>
                    </div>
                </Portal>
            </Show>
        </>
    );
};

type SubMenuPositionMap = {
    [key: string]: { anchorRect: DOMRect; label: string };
};

type SubMenuProps = {
    subItems: MenuItem[];
    parentKey: string;
    subMenuPosition: SubMenuPositionMap;
    setSubMenuPosition: (
        updater: (prev: SubMenuPositionMap) => SubMenuPositionMap,
    ) => void;
    visibleSubMenus: { [key: string]: any };
    hoveredItems: string[];
    handleMouseEnterItem: (
        event: MouseEvent,
        parentKey: string | null,
        index: number,
        item: MenuItem
    ) => void;
    handleOnClick: (e: MouseEvent, item: MenuItem) => void;
    renderMenu?: (subMenu: JSX.Element, props: any) => JSX.Element;
    renderMenuItem?: (item: MenuItem, props: any) => JSX.Element;
    mirrored?: boolean;
};

const SubMenu = (props: SubMenuProps): JSX.Element => {
    const position = () => props.subMenuPosition[props.parentKey];

    // Submenu positioning routes through the Phase 1 primitive: anchored to the
    // parent menu item's rect, preferred placement right-start — flip() turns
    // that into left-start near the right edge, shift() pulls it back inside
    // near the bottom edge, all paintable-area aware. autoUpdate keeps it live
    // on scroll/resize (the old one-shot `flipped` flag never re-evaluated).
    const [subStyle, setSubStyle] = createSignal("position:fixed;left:0px;top:0px");
    let cleanupAutoUpdate: (() => void) | null = null;

    const registerSubMenu = (el: HTMLDivElement) => {
        requestAnimationFrame(() => {
            const pos = position();
            if (!pos || !(el instanceof Element)) return;
            const update = async () => {
                const cur = position();
                if (!cur) return;
                // Mirrored (right-anchored) menus prefer to open submenus to
                // the LEFT; flip() still sends them right if there's no room.
                // Standard menus prefer right, flipping left near the edge.
                const computed = await computeMenuPosition(
                    {
                        anchor: cur.anchorRect,
                        placement: props.mirrored ? "left-start" : "right-start",
                    },
                    el,
                );
                setSubStyle(styleToString(computed.style));
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
        >
            <For each={props.subItems}>
                {(item, idx) => {
                    const newKey = `${props.parentKey}-${idx()}`;
                    const isActive = () => props.hoveredItems.includes(newKey);

                    const menuItemProps = {
                        class: clsx("menu-item", { active: isActive() }),
                        onMouseEnter: (event: MouseEvent) =>
                            props.handleMouseEnterItem(event, props.parentKey, idx(), item),
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
                            <Show when={props.visibleSubMenus[newKey]?.visible && item.subItems}>
                                <SubMenu
                                    subItems={item.subItems!}
                                    parentKey={newKey}
                                    subMenuPosition={props.subMenuPosition}
                                    setSubMenuPosition={props.setSubMenuPosition}
                                    visibleSubMenus={props.visibleSubMenus}
                                    hoveredItems={props.hoveredItems}
                                    handleMouseEnterItem={props.handleMouseEnterItem}
                                    handleOnClick={props.handleOnClick}
                                    renderMenu={props.renderMenu}
                                    renderMenuItem={props.renderMenuItem}
                                    mirrored={props.mirrored}
                                />
                            </Show>
                        </>
                    );
                }}
            </For>
        </div>
    );

    return (
        <Portal>
            {props.renderMenu ? props.renderMenu(subMenu, { parentKey: props.parentKey }) : subMenu}
        </Portal>
    );
};

export { FlyoutMenu };
