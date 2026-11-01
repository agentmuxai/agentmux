// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Menu: the one anchored menu. It hangs from an element, a rect or a point, opens upward and flips
 * down only when there is no room above (the placement default; `AnchoredPopover` does the placing,
 * the chrome zoom and the airspace cut), closes on a pick, an outside press or Esc, and is driven
 * from the keyboard. A row-actions menu, a right-click menu and a short choice ("already open:
 * fork or switch?") are all this one component with different items.
 */

import { AnchoredPopover } from "@/app/element/anchored-popover";
import type { Placement } from "@floating-ui/dom";
import clsx from "clsx";
import { createSignal, For, onMount, Show, type JSX } from "solid-js";
import "./menu.scss";

export interface MenuEntry {
    label: string;
    /** A Font Awesome name, drawn in the leading slot. */
    icon?: string;
    onSelect?: () => void;
    tone?: "danger";
    disabled?: boolean;
    /** Shown right-aligned, already formatted ("Ctrl+P"). */
    shortcut?: string;
    /** A hairline between groups; no other field applies. */
    divider?: boolean;
    testId?: string;
}

/** An element (followed live), a rect, or a point (a right-click). */
export type MenuAnchor = Element | DOMRect | { x: number; y: number };

export interface MenuProps {
    anchor: MenuAnchor | null | undefined;
    items: MenuEntry[];
    /** A line above the rows that says what the menu is about; not a row. */
    header?: JSX.Element;
    /** Upward by default; it flips down when the room above is too small. */
    placement?: Placement;
    /** A pick, an outside press or Esc. The owner unmounts the menu. */
    onClose: () => void;
    "aria-label"?: string;
    class?: string;
    testId?: string;
}

const asAnchor = (a: MenuAnchor | null | undefined): Element | DOMRect | undefined => {
    if (!a) return undefined;
    if (a instanceof Element || a instanceof DOMRect) return a;
    return new DOMRect(a.x, a.y, 0, 0);
};

export const Menu = (props: MenuProps): JSX.Element => {
    const [active, setActive] = createSignal(-1);
    let body: HTMLDivElement | undefined;
    const pickable = (i: number) => {
        const e = props.items[i];
        return !!e && !e.divider && !e.disabled;
    };
    const step = (from: number, dir: 1 | -1): number => {
        const n = props.items.length;
        for (let k = 1; k <= n; k++) {
            const i = (((from + dir * k) % n) + n) % n;
            if (pickable(i)) return i;
        }
        return from;
    };
    const select = (i: number) => {
        if (!pickable(i)) return;
        const entry = props.items[i];
        props.onClose();
        entry.onSelect?.();
    };
    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "ArrowDown") setActive(step(active(), 1));
        else if (e.key === "ArrowUp") setActive(step(active() < 0 ? props.items.length : active(), -1));
        else if (e.key === "Home") setActive(step(-1, 1));
        else if (e.key === "End") setActive(step(props.items.length, -1));
        else if (e.key === "Enter" || e.key === " ") select(active());
        else return;
        e.preventDefault();
        e.stopPropagation();
    };
    onMount(() => body?.focus({ preventScroll: true }));

    return (
        <AnchoredPopover
            anchor={asAnchor(props.anchor)}
            placement={props.placement ?? "top-start"}
            shellClass="anchored-popover--menu"
            class={clsx("menu ui-menu", props.class)}
            role="menu"
            aria-label={props["aria-label"]}
            onDismiss={props.onClose}
            ref={(el) => {
                body = el;
                el.tabIndex = -1;
                el.addEventListener("keydown", onKeyDown);
                if (props.testId) el.dataset.testid = props.testId;
            }}
        >
            <Show when={props.header}>
                <div class="ui-menu-header">{props.header}</div>
            </Show>
            <For each={props.items}>
                {(entry, i) => (
                    <Show when={!entry.divider} fallback={<div class="menu-divider" aria-hidden="true" />}>
                        <div
                            role="menuitem"
                            class={clsx("menu-item ui-menu-item", {
                                active: active() === i(),
                                "ui-menu-item--danger": entry.tone === "danger",
                                "ui-menu-item--disabled": entry.disabled,
                            })}
                            aria-disabled={entry.disabled || undefined}
                            data-testid={entry.testId}
                            onMouseEnter={() => pickable(i()) && setActive(i())}
                            onClick={() => select(i())}
                        >
                            <Show when={entry.icon}>
                                <i class={clsx("fa-solid fa-fw menu-item-icon", `fa-${entry.icon}`)} aria-hidden="true" />
                            </Show>
                            <span class="label">{entry.label}</span>
                            <Show when={entry.shortcut}>
                                <span class="menu-item-shortcut">{entry.shortcut}</span>
                            </Show>
                        </div>
                    </Show>
                )}
            </For>
        </AnchoredPopover>
    );
};
