// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Minimal context menu component. Renders into a Portal so it floats above
// all pane content. Items are plain data; callers build the list and attach
// onSelect handlers. Single-level only — no submenus.

import { createEffect, onCleanup, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import "./context-menu.scss";

export interface ContextMenuItem {
    type: "action" | "separator";
    label?: string;
    shortcut?: string;
    disabled?: boolean;
    danger?: boolean;
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

    createEffect(() => {
        const handlePointerDown = (e: PointerEvent) => {
            if (!menuRef?.contains(e.target as Node)) props.onClose();
        };
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                e.stopPropagation();
                props.onClose();
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

    // Clamp to viewport so the menu never clips off-screen.
    const style = (): JSX.CSSProperties => {
        const x = Math.min(props.x, window.innerWidth - 220);
        const y = Math.min(props.y, window.innerHeight - 400);
        return { position: "fixed", left: `${x}px`, top: `${y}px`, "z-index": "9999" };
    };

    return (
        <Portal>
            <div ref={(el) => { menuRef = el; }} class="ctx-menu" style={style()}>
                {props.items.map((item) =>
                    item.type === "separator" ? (
                        <div class="ctx-menu-sep" />
                    ) : (
                        <div
                            class="ctx-menu-item"
                            classList={{
                                "ctx-menu-item--disabled": !!item.disabled,
                                "ctx-menu-item--danger": !!item.danger,
                            }}
                            onPointerDown={(e) => {
                                e.stopPropagation();
                                if (!item.disabled) {
                                    item.onSelect?.();
                                    props.onClose();
                                }
                            }}
                        >
                            <span class="ctx-menu-label">{item.label}</span>
                            {item.shortcut && (
                                <span class="ctx-menu-shortcut">{item.shortcut}</span>
                            )}
                        </div>
                    )
                )}
            </div>
        </Portal>
    );
}
