// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import {
    autoUpdate,
    computePosition,
    type Placement,
} from "@floating-ui/dom";
import clsx from "clsx";
import { createSignal, For, JSX, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";

import "./flyoutmenu.scss";

type MenuProps = {
    items: MenuItem[];
    className?: string;
    placement?: Placement;
    onOpenChange?: (isOpen: boolean) => void;
    children?: JSX.Element;
    renderMenu?: (subMenu: JSX.Element, props: any) => JSX.Element;
    renderMenuItem?: (item: MenuItem, props: any) => JSX.Element;
};

const FlyoutMenu = (props: MenuProps): JSX.Element => {
    const [visibleSubMenus, setVisibleSubMenus] = createSignal<{ [key: string]: any }>({});
    const [hoveredItems, setHoveredItems] = createSignal<string[]>([]);
    const [subMenuPosition, setSubMenuPosition] = createSignal<{
        [key: string]: { top: number; left: number; label: string };
    }>({});

    const [isOpen, setIsOpen] = createSignal(false);
    const [floatingStyle, setFloatingStyle] = createSignal("position:absolute;left:0px;top:0px");

    let referenceEl: HTMLElement | null = null;
    let floatingEl: HTMLElement | null = null;
    let cleanupAutoUpdate: (() => void) | null = null;

    const onOpenChangeMenu = (open: boolean) => {
        setIsOpen(open);
        props.onOpenChange?.(open);
    };

    const updatePosition = async () => {
        if (!referenceEl || !floatingEl) return;
        const pos = await computePosition(referenceEl, floatingEl, {
            placement: props.placement ?? "bottom-start",
        });
        setFloatingStyle(`position:absolute;left:${pos.x}px;top:${pos.y}px`);
    };

    const registerFloating = (el: HTMLElement) => {
        floatingEl = el;
        requestAnimationFrame(() => {
            if (referenceEl instanceof Element && floatingEl instanceof Element) {
                cleanupAutoUpdate?.();
                cleanupAutoUpdate = autoUpdate(referenceEl, floatingEl, updatePosition);
            }
        });
    };

    const handleClickOutside = (e: MouseEvent) => {
        if (!isOpen()) return;
        const target = e.target as Node;
        if (referenceEl?.contains(target) || floatingEl?.contains(target)) return;
        onOpenChangeMenu(false);
    };

    onMount(() => {
        document.addEventListener("mousedown", handleClickOutside);
    });

    onCleanup(() => {
        document.removeEventListener("mousedown", handleClickOutside);
        cleanupAutoUpdate?.();
    });

    const subMenuRefs: { [key: string]: HTMLDivElement | null } = {};

    // Position submenus based on available space and scroll position
    const handleSubMenuPosition = (key: string, itemRect: DOMRect, label: string) => {
        setTimeout(() => {
            const subMenuRef = subMenuRefs[key];
            if (!subMenuRef) return;

            const scrollTop = window.scrollY || document.documentElement.scrollTop;
            const scrollLeft = window.scrollX || document.documentElement.scrollLeft;

            const submenuWidth = subMenuRef.offsetWidth;
            const submenuHeight = subMenuRef.offsetHeight;

            let left = itemRect.right + scrollLeft - 2;
            let top = itemRect.top - 2 + scrollTop;

            if (left + submenuWidth > window.innerWidth + scrollLeft) {
                left = itemRect.left + scrollLeft - submenuWidth;
            }

            if (top + submenuHeight > window.innerHeight + scrollTop) {
                top = window.innerHeight + scrollTop - submenuHeight - 10;
            }

            setSubMenuPosition((prev) => ({
                ...prev,
                [key]: { top, left, label },
            }));
        }, 0);
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
        onOpenChangeMenu(false);
        item.onClick?.(e);
    };

    return (
        <>
            <div
                ref={(el) => { referenceEl = el; }}
                class="menu-anchor"
                onClick={() => onOpenChangeMenu(!isOpen())}
            >
                {props.children}
            </div>
            <Show when={isOpen()}>
                <Portal>
                    <div
                        class={clsx("menu", props.className)}
                        ref={registerFloating}
                        style={floatingStyle()}
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

                                const renderedItem = props.renderMenuItem ? (
                                    props.renderMenuItem(item, menuItemProps)
                                ) : (
                                    <div {...menuItemProps}>
                                        <span class="label">{item.label}</span>
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
                                                visibleSubMenus={visibleSubMenus()}
                                                hoveredItems={hoveredItems()}
                                                handleMouseEnterItem={handleMouseEnterItem}
                                                handleOnClick={handleOnClick}
                                                subMenuRefs={subMenuRefs}
                                                renderMenu={props.renderMenu}
                                                renderMenuItem={props.renderMenuItem}
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

type SubMenuProps = {
    subItems: MenuItem[];
    parentKey: string;
    subMenuPosition: {
        [key: string]: { top: number; left: number; label: string };
    };
    visibleSubMenus: { [key: string]: any };
    hoveredItems: string[];
    subMenuRefs: { [key: string]: HTMLDivElement | null };
    handleMouseEnterItem: (
        event: MouseEvent,
        parentKey: string | null,
        index: number,
        item: MenuItem
    ) => void;
    handleOnClick: (e: MouseEvent, item: MenuItem) => void;
    renderMenu?: (subMenu: JSX.Element, props: any) => JSX.Element;
    renderMenuItem?: (item: MenuItem, props: any) => JSX.Element;
};

const SubMenu = (props: SubMenuProps): JSX.Element => {
    const position = () => props.subMenuPosition[props.parentKey];
    const isPositioned = () => {
        const pos = position();
        return pos && pos.top !== undefined && pos.left !== undefined;
    };

    const subMenu = (
        <div
            ref={(el) => { props.subMenuRefs[props.parentKey] = el; }}
            class="menu sub-menu"
            style={{
                top: `${position()?.top || 0}px`,
                left: `${position()?.left || 0}px`,
                position: "absolute",
                "z-index": 1000,
                visibility: props.visibleSubMenus[props.parentKey]?.visible && isPositioned() ? "visible" : "hidden",
            }}
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

                    const renderedItem = props.renderMenuItem ? (
                        props.renderMenuItem(item, menuItemProps)
                    ) : (
                        <div {...menuItemProps}>
                            <span class="label">{item.label}</span>
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
                                    visibleSubMenus={props.visibleSubMenus}
                                    hoveredItems={props.hoveredItems}
                                    handleMouseEnterItem={props.handleMouseEnterItem}
                                    handleOnClick={props.handleOnClick}
                                    subMenuRefs={props.subMenuRefs}
                                    renderMenu={props.renderMenu}
                                    renderMenuItem={props.renderMenuItem}
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
