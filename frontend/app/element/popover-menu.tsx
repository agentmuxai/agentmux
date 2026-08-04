// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { assertMenuInPaintableArea } from "@/app/util/menu-position";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import clsx from "clsx";
import { createSignal, For, JSX, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";

import "./popover-menu.scss";

// ── Types ──────────────────────────────────────────────────────────────────

type PopoverMenuActionItem = {
    type?: "normal" | "checkbox";
    label: string;
    checked?: boolean;
    disabled?: boolean;
    /** When true, clicking this item does not close the popover. Default false. */
    keepOpen?: boolean;
    click: () => void;
};

type PopoverMenuSeparator = { type: "separator" };

type PopoverMenuSection = {
    type: "section";
    label: string;
    /** Whether the section starts expanded. Default true. */
    defaultOpen?: boolean;
    items: PopoverMenuActionItem[];
};

export type PopoverMenuItem = PopoverMenuActionItem | PopoverMenuSeparator | PopoverMenuSection;

interface PopoverMenuProps {
    items: PopoverMenuItem[];
    /** Cursor position in client px. Clamped to viewport on mount. */
    pos: { x: number; y: number };
    onClose: () => void;
}

// ── Main component ─────────────────────────────────────────────────────────

const PopoverMenu = (props: PopoverMenuProps): JSX.Element => {
    const [menuEl, setMenuEl] = createSignal<HTMLDivElement | null>(null);
    const [left, setLeft] = createSignal(props.pos.x);
    const [top, setTop]   = createSignal(props.pos.y);

    // Required: cuts a transparent clip through any CEF browser-pane HWND
    // behind this popover so the DOM renders above native pane content.
    usePaneOverlay(menuEl);

    const registerMenu = (el: HTMLDivElement) => {
        setMenuEl(el);
        requestAnimationFrame(() => {
            if (!(el instanceof Element)) return;
            const r = el.getBoundingClientRect();
            setLeft(Math.min(props.pos.x, window.innerWidth  - r.width  - 8));
            setTop( Math.min(props.pos.y, window.innerHeight - r.height - 8));
            // Dev-only guard: logs if the menu falls outside the paintable area.
            assertMenuInPaintableArea(el, "popover-menu");
        });
    };

    const close = () => props.onClose();

    const handleMouseDown = (e: MouseEvent) => {
        const t = e.target as Node;
        if (menuEl()?.contains(t)) return;
        close();
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape") close();
    };

    const handleVisibilityChange = () => {
        if (document.hidden) close();
    };

    onMount(() => {
        document.addEventListener("mousedown",        handleMouseDown, true);
        document.addEventListener("keydown",          handleKeyDown);
        document.addEventListener("visibilitychange", handleVisibilityChange);
    });

    onCleanup(() => {
        document.removeEventListener("mousedown",        handleMouseDown, true);
        document.removeEventListener("keydown",          handleKeyDown);
        document.removeEventListener("visibilitychange", handleVisibilityChange);
    });

    return (
        <Portal mount={document.body}>
            <div
                ref={registerMenu}
                class="menu popover-menu"
                style={{ position: "fixed", left: `${left()}px`, top: `${top()}px` }}
                data-pane-overlay
            >
                <For each={props.items}>
                    {(item) => <PopoverMenuItemRenderer item={item} onClose={close} />}
                </For>
            </div>
        </Portal>
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
            <Show when={item.type === "checkbox"} fallback={<span class="menu-item-icon-spacer" />}>
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
    const [open, setOpen] = createSignal(props.section.defaultOpen ?? true);

    const handleHeaderClick = (e: MouseEvent) => {
        e.stopPropagation();
        setOpen((v) => !v);
        // Intentionally does NOT call onClose — section collapse is not an action.
    };

    return (
        <div class="popover-menu-section">
            <div class="menu-item popover-menu-section-header" onClick={handleHeaderClick}>
                <i
                    class={clsx(
                        "fa-solid fa-fw menu-item-icon",
                        open() ? "fa-chevron-down" : "fa-chevron-right",
                    )}
                />
                <span class="label">{props.section.label}</span>
            </div>
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
