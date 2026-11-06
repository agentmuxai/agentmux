// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AnchoredPopover, pointAnchor } from "@/app/element/anchored-popover";
import { menuIconClass } from "@/app/util/menu-icons";
import clsx from "clsx";
import { createSignal, For, JSX, onCleanup, onMount, Show } from "solid-js";

import "./popover-menu.scss";

// ── Types ──────────────────────────────────────────────────────────────────

type PopoverMenuActionItem = {
    type?: "normal" | "checkbox";
    label: string;
    checked?: boolean;
    disabled?: boolean;
    /** When true, clicking this item does not close the popover. Default false. */
    keepOpen?: boolean;
    /** A Font Awesome name drawn in the row's leading slot (not on a checkbox row, whose check
     *  takes the slot); decoration only. See app/util/menu-icons.ts. */
    icon?: string;
    click: () => void;
};

type PopoverMenuSeparator = { type: "separator" };

type PopoverMenuSection = {
    type: "section";
    label: string;
    /** Whether the section starts expanded. Default true. */
    defaultOpen?: boolean;
    /** Whether clicking the header collapses the section. Default true. When
     *  false the header is a plain label: no chevron, no click, no hover. */
    collapsible?: boolean;
    items: PopoverMenuActionItem[];
};

export type PopoverMenuItem = PopoverMenuActionItem | PopoverMenuSeparator | PopoverMenuSection;

interface PopoverMenuProps {
    items: PopoverMenuItem[];
    /** Cursor position in client px. The menu opens there, and flips or shifts to stay in the window. */
    pos: { x: number; y: number };
    onClose: () => void;
}

// ── Main component ─────────────────────────────────────────────────────────

const PopoverMenu = (props: PopoverMenuProps): JSX.Element => {
    const close = () => props.onClose();

    // The cursor is a point (a zero-size rect) AnchoredPopover places the menu at, without a gap. It
    // follows chrome zoom, cuts the hole in any browser pane behind it and dismisses on an outside press
    // or Esc. Starts hidden and is shown once placed.
    const point = pointAnchor(props.pos.x, props.pos.y);

    const handleVisibilityChange = () => {
        if (document.hidden) close();
    };

    onMount(() => document.addEventListener("visibilitychange", handleVisibilityChange));
    onCleanup(() => document.removeEventListener("visibilitychange", handleVisibilityChange));

    return (
        <AnchoredPopover
            anchor={point}
            placement="bottom-start"
            gutter={0}
            fitHeight
            hideUntilPlaced
            overlayKeys
            shellClass="anchored-popover--menu"
            class="menu popover-menu"
            onDismiss={close}
        >
            <For each={props.items}>
                {(item) => <PopoverMenuItemRenderer item={item} onClose={close} />}
            </For>
        </AnchoredPopover>
    );
};

// ── Item renderers ─────────────────────────────────────────────────────────

const PopoverMenuItemRenderer = (props: {
    item: PopoverMenuItem;
    onClose: () => void;
}): JSX.Element => {
    const { item } = props;

    if (item.type === "separator") {
        return <div class="menu-divider" aria-hidden="true" />;
    }

    if (item.type === "section") {
        return <PopoverMenuSectionRenderer section={item} onClose={props.onClose} />;
    }

    // Normal / checkbox action item
    const handleClick = (e: MouseEvent) => {
        e.stopPropagation();
        if (item.disabled) return;
        item.click();
        if (!item.keepOpen) props.onClose();
    };

    return (
        <div
            class={clsx("menu-item popover-menu-action", { disabled: item.disabled })}
            onClick={handleClick}
        >
            <Show
                when={item.type === "checkbox"}
                fallback={
                    <Show when={menuIconClass(item.icon)} fallback={<span class="menu-item-icon-spacer" />}>
                        {(cls) => <i class={cls()} aria-hidden="true" />}
                    </Show>
                }
            >
                <i
                    class={clsx(
                        "fa-solid fa-fw menu-item-icon menu-item-check",
                        { "fa-check": item.checked === true },
                    )}
                />
            </Show>
            <span class="label">{item.label}</span>
        </div>
    );
};

const PopoverMenuSectionRenderer = (props: {
    section: PopoverMenuSection;
    onClose: () => void;
}): JSX.Element => {
    const collapsible = props.section.collapsible ?? true;
    const [open, setOpen] = createSignal(!collapsible || (props.section.defaultOpen ?? true));

    const handleHeaderClick = (e: MouseEvent) => {
        e.stopPropagation();
        setOpen((v) => !v);
        // Intentionally does NOT call onClose — section collapse is not an action.
    };

    return (
        <div class="popover-menu-section">
            <Show
                when={collapsible}
                fallback={
                    <div class="menu-item popover-menu-section-header popover-menu-section-header--static">
                        <span class="label">{props.section.label}</span>
                    </div>
                }
            >
                <div class="menu-item popover-menu-section-header" onClick={handleHeaderClick}>
                    <i
                        class={clsx(
                            "fa-solid fa-fw menu-item-icon",
                            open() ? "fa-chevron-down" : "fa-chevron-right",
                        )}
                    />
                    <span class="label">{props.section.label}</span>
                </div>
            </Show>
            <Show when={open()}>
                <div class="popover-menu-section-items">
                    <For each={props.section.items}>
                        {(item) => <PopoverMenuItemRenderer item={item} onClose={props.onClose} />}
                    </For>
                </div>
            </Show>
        </div>
    );
};

export { PopoverMenu };
