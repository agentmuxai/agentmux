// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import {
    autoUpdate,
    computePosition,
    type Placement,
} from "@floating-ui/dom";
import clsx from "clsx";
import { createEffect, createSignal, For, JSX, onCleanup, onMount, Show } from "solid-js";
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
        [key: string]: { bottom: number; left: number; parentLeft: number; parentTop: number; label: string };
    }>({});

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
        const pos = await computePosition(referenceEl, floatingEl, {
            placement: props.placement ?? "bottom-start",
        });
        setFloatingStyle(`position:absolute;left:${pos.x}px;top:${pos.y}px`);
    };

    const registerFloating = (el: HTMLElement) => {
        floatingEl = el;
        requestAnimationFrame(() => {
            if (!(referenceEl instanceof Element) || !(floatingEl instanceof Element)) return;
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = autoUpdate(referenceEl, floatingEl, updatePosition);
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
        const scrollTop = window.scrollY || document.documentElement.scrollTop;
        const scrollLeft = window.scrollX || document.documentElement.scrollLeft;
        const bottom = window.innerHeight + scrollTop - itemRect.bottom - 2;
        const left = itemRect.right + scrollLeft - 2;
        const parentLeft = itemRect.left + scrollLeft;
        const parentTop = itemRect.top + scrollTop;
        setSubMenuPosition((prev) => ({ ...prev, [key]: { bottom, left, parentLeft, parentTop, label } }));
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
                        class={clsx("menu", props.className)}
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
    [key: string]: { bottom: number; left: number; parentLeft: number; parentTop: number; label: string };
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
};

const SubMenu = (props: SubMenuProps): JSX.Element => {
    let subMenuEl: HTMLDivElement | undefined;
    let flipped = false;
    const position = () => props.subMenuPosition[props.parentKey];

    createEffect(() => {
        const pos = position();
        if (!pos || flipped || !subMenuEl) return;
        const rect = subMenuEl.getBoundingClientRect();
        const overflowRight = rect.right - window.innerWidth;
        const overflowTop = -rect.top;
        if (overflowRight <= 0 && overflowTop <= 0) {
            flipped = true;
            return;
        }
        flipped = true;
        props.setSubMenuPosition((prev) => ({
            ...prev,
            [props.parentKey]: {
                label: pos.label,
                parentLeft: pos.parentLeft,
                parentTop: pos.parentTop,
                left: overflowRight > 0 ? pos.parentLeft - rect.width + 2 : pos.left,
                bottom: overflowTop > 0
                    ? window.innerHeight + window.scrollY - pos.parentTop - rect.height + 2
                    : pos.bottom,
            },
        }));
    });

    const subMenu = (
        <div
            ref={(el) => { subMenuEl = el; }}
            class="menu sub-menu"
            style={{
                bottom: `${position()?.bottom ?? 0}px`,
                left: `${position()?.left ?? 0}px`,
                position: "absolute",
                "z-index": 1000,
            }}
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
