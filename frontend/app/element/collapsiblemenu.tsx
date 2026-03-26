// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createSignal, For, JSX, Show } from "solid-js";
import "./collapsiblemenu.scss";

interface VerticalNavProps {
    items: MenuItem[];
    className?: string;
    renderItem?: (
        item: MenuItem,
        isOpen: boolean,
        handleClick: (e: MouseEvent, item: MenuItem, itemKey: string) => void
    ) => JSX.Element;
}

const CollapsibleMenu = ({ items, className, renderItem }: VerticalNavProps): JSX.Element => {
    const [open, setOpen] = createSignal<{ [key: string]: boolean }>({});

    // Helper function to generate a unique key for each item based on its path in the hierarchy
    const getItemKey = (item: MenuItem, path: string) => `${path}-${item.label}`;

    const handleClick = (e: MouseEvent, item: MenuItem, itemKey: string) => {
        setOpen((prevState) => ({ ...prevState, [itemKey]: !prevState[itemKey] }));
        if (item.onClick) {
            item.onClick(e);
        }
    };

    const renderListItem = (item: MenuItem, index: number, path: string): JSX.Element => {
        const itemKey = getItemKey(item, path);
        const isItemOpen = () => open()[itemKey] === true;
        const hasChildren = item.subItems && item.subItems.length > 0;

        return (
            <li class="collapsible-menu-item">
                <Show
                    when={renderItem}
                    fallback={
                        <div
                            class="collapsible-menu-item-button"
                            onClick={(e) => handleClick(e, item, itemKey)}
                        >
                            <div
                                class={clsx("collapsible-menu-item-content", {
                                    "has-children": hasChildren,
                                    "is-open": isItemOpen() && hasChildren,
                                })}
                            >
                                <Show when={item.icon}>
                                    <div class="collapsible-menu-item-icon">{item.icon}</div>
                                </Show>
                                <div class="collapsible-menu-item-text ellipsis">{item.label}</div>
                            </div>
                            <Show when={hasChildren}>
                                <i class={`fa-sharp fa-solid ${isItemOpen() ? "fa-angle-up" : "fa-angle-down"}`} />
                            </Show>
                        </div>
                    }
                >
                    {renderItem!(item, isItemOpen(), (e) => handleClick(e, item, itemKey))}
                </Show>
                <Show when={hasChildren}>
                    <ul class={`nested-list ${isItemOpen() ? "open" : "closed"}`}>
                        <For each={item.subItems}>
                            {(child, childIndex) => renderListItem(child, childIndex(), `${path}-${index}`)}
                        </For>
                    </ul>
                </Show>
            </li>
        );
    };

    return (
        <ul class={clsx("collapsible-menu", className)} role="navigation">
            <For each={items}>
                {(item, index) => renderListItem(item, index(), "root")}
            </For>
        </ul>
    );
};

export { CollapsibleMenu };
