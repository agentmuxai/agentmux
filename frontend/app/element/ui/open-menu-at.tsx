// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * openMenuAt: a right-click menu drawn by the kit `Menu`, at the pointer. It takes the same
 * `ContextMenuItem[]` the JS context menu (`ContextMenuModel.showContextMenu`) takes, so a site changes one
 * call and its item builders stay as they are. One menu is open at a time: opening another closes the
 * first. Like the JS menu, it opens downward from the pointer (the kit's own menus open upward) and hands
 * focus back to what had it, so a menu that closes without a pick leaves the caret where it was.
 */

import { closeMenuAt, setOpenMenuAt } from "@/app/util/menu-at-open";
import { render } from "solid-js/web";
import { Menu, type MenuEntry } from "./Menu";

/** The kit's rows for a `ContextMenuItem[]`: a hidden item is left out, `enabled: false` disables, `click` selects. */
export function menuEntriesFrom(items: ContextMenuItem[]): MenuEntry[] {
    return items
        .filter((item) => item.visible !== false)
        .map((item): MenuEntry => {
            if (item.type === "separator") return { label: "", divider: true };
            return {
                label: item.label ?? "",
                icon: item.icon,
                type: item.type === "checkbox" || item.type === "radio" ? item.type : undefined,
                checked: item.checked,
                disabled: item.enabled === false,
                onSelect: item.click,
            };
        });
}

/** Show `items` as a kit `Menu` at the pointer of `ev` (a right-click's `contextmenu` event); the arguments of `showContextMenu`, so a site swaps one call. */
export function openMenuAt(items: ContextMenuItem[], ev: { stopPropagation(): void; clientX?: number; clientY?: number }): void {
    ev.stopPropagation();
    closeMenuAt();
    const entries = menuEntriesFrom(items);
    if (entries.length === 0) return;
    const opener = document.activeElement instanceof HTMLElement && document.activeElement !== document.body ? document.activeElement : null;
    const host = document.createElement("div");
    let dispose: (() => void) | null = null;
    let closed = false;
    const close = () => {
        // Twice is harmless, and a menu already replaced must not clear the one that replaced it.
        if (closed) return;
        closed = true;
        setOpenMenuAt(null);
        dispose?.();
        dispose = null;
        host.remove();
        // After the chosen row has run: focus that fell to <body> goes back to where it was. A row that moved focus on purpose keeps it.
        queueMicrotask(() => {
            const active = document.activeElement;
            if ((active == null || active === document.body) && opener?.isConnected) opener.focus({ preventScroll: true });
        });
    };
    setOpenMenuAt(close);
    document.body.appendChild(host);
    dispose = render(
        () => <Menu anchor={{ x: Math.round(ev.clientX ?? 0), y: Math.round(ev.clientY ?? 0) }} items={entries} alignIcons placement="bottom-start" onClose={close} />,
        host,
    );
}
