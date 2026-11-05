// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Minimal context menu component. Renders into a Portal so it floats above
// all pane content. Items are plain data; callers build the list and attach
// onSelect handlers. Single-level only — no submenus.

import { createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import { AnchoredPopover } from "@/app/element/anchored-popover";
import { menuIconClass } from "@/app/util/menu-icons";
import "./context-menu.scss";

export interface ContextMenuItem {
    type: "action" | "separator";
    label?: string;
    shortcut?: string;
    disabled?: boolean;
    danger?: boolean;
    /** A Font Awesome name drawn before the label; decoration only. See app/util/menu-icons.ts. */
    icon?: string;
    onSelect?: () => void;
}

interface Props {
    items: ContextMenuItem[];
    x: number;
    y: number;
    onClose: () => void;
}

export function ContextMenu(props: Props): JSX.Element {
    let menuRef: HTMLDivElement | undefined;
    const [focusedIdx, setFocusedIdx] = createSignal(-1);
    // Any row with an icon reserves the slot on every row, so the labels line up.
    const hasIcons = () => props.items.some((i) => i.type === "action" && !!menuIconClass(i.icon));

    // Indices of selectable (non-separator, non-disabled) items.
    const selectableIndices = (): number[] =>
        props.items.reduce<number[]>((acc, item, i) => {
            if (item.type === "action" && !item.disabled) acc.push(i);
            return acc;
        }, []);

    createEffect(() => {
        // Focus the menu div so arrow keys work immediately.
        menuRef?.focus();

        const handlePointerDown = (e: PointerEvent) => {
            if (!menuRef?.contains(e.target as Node)) props.onClose();
        };
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                e.stopPropagation();
                props.onClose();
            } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                e.stopPropagation();
                const sel = selectableIndices();
                if (!sel.length) return;
                const cur = focusedIdx();
                const pos = sel.indexOf(cur);
                if (e.key === "ArrowDown") {
                    setFocusedIdx(sel[(pos + 1) % sel.length]);
                } else {
                    setFocusedIdx(sel[(pos - 1 + sel.length) % sel.length]);
                }
            } else if (e.key === "Enter") {
                e.preventDefault();
                e.stopPropagation();
                const idx = focusedIdx();
                if (idx >= 0) {
                    const item = props.items[idx];
                    if (item?.type === "action" && !item.disabled) {
                        item.onSelect?.();
                        props.onClose();
                    }
                }
            }
        };
        // Capture phase so we intercept before pane handlers.
        document.addEventListener("pointerdown", handlePointerDown, true);
        document.addEventListener("keydown", handleKeyDown, true);
        onCleanup(() => {
            document.removeEventListener("pointerdown", handlePointerDown, true);
            document.removeEventListener("keydown", handleKeyDown, true);
        });
    });

    // The cursor is a point (a zero-size rect) AnchoredPopover opens the menu at, flipping or shifting to stay in
    // the window; it follows chrome zoom and cuts the hole in any browser pane behind it.
    const { x, y } = props;
    const point = { x, y, left: x, top: y, right: x, bottom: y, width: 0, height: 0, toJSON: () => ({}) } as DOMRect;

    return (
        <AnchoredPopover
            anchor={point}
            placement="bottom-start"
            gutter={0}
            fitHeight
            hideUntilPlaced
            // It cannot take focus while hidden: arrow keys work from the moment it is shown.
            onShown={() => menuRef?.focus()}
            overlayKeys
            shellClass="anchored-popover--menu"
            class="ctx-menu"
            ref={(el) => {
                menuRef = el;
                el.tabIndex = -1;
            }}
        >
            {props.items.map((item, i) =>
                item.type === "separator" ? (
                    <div class="ctx-menu-sep" />
                ) : (
                    <div
                        class="ctx-menu-item"
                        classList={{
                            "ctx-menu-item--disabled": !!item.disabled,
                            "ctx-menu-item--danger": !!item.danger,
                            "ctx-menu-item--focused": focusedIdx() === i,
                        }}
                        onPointerDown={(e) => {
                            e.stopPropagation();
                            if (!item.disabled) {
                                item.onSelect?.();
                                props.onClose();
                            }
                        }}
                        onPointerEnter={() => { if (!item.disabled) setFocusedIdx(i); }}
                    >
                        <span class="ctx-menu-main">
                            {hasIcons() &&
                                (menuIconClass(item.icon) ? (
                                    <i class={menuIconClass(item.icon)!} aria-hidden="true" />
                                ) : (
                                    <span class="menu-item-icon" aria-hidden="true" />
                                ))}
                            <span class="ctx-menu-label">{item.label}</span>
                        </span>
                        {item.shortcut && (
                            <span class="ctx-menu-shortcut">{item.shortcut}</span>
                        )}
                    </div>
                )
            )}
        </AnchoredPopover>
    );
}
