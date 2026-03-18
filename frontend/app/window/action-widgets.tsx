// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActionWidgets - Right-aligned buttons for creating blocks
 */

import { Tooltip } from "@/app/element/tooltip";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atoms, createBlock, getApi } from "@/store/global";
import { useWindowDrag } from "@/app/hook/useWindowDrag";
import { fireAndForget, isBlank, makeIconClass } from "@/util/util";
import { invoke } from "@tauri-apps/api/core";
import { createSignal, For, Show, type JSX } from "solid-js";
import "./action-widgets.scss";

function getSortedWidgets(
    wmap: { [key: string]: WidgetConfigType },
    settings: Record<string, any>
): { key: string; widget: WidgetConfigType }[] {
    if (wmap == null) return [];
    const order: string[] | undefined = settings["widget:order"];
    const entries = Object.entries(wmap).map(([key, widget]) => ({ key, widget }));
    if (order && order.length > 0) {
        entries.sort((a, b) => {
            const ai = order.indexOf(a.key.replace("defwidget@", ""));
            const bi = order.indexOf(b.key.replace("defwidget@", ""));
            const an = ai === -1 ? 999 : ai;
            const bn = bi === -1 ? 999 : bi;
            if (an !== bn) return an - bn;
            return (a.widget["display:order"] ?? 0) - (b.widget["display:order"] ?? 0);
        });
    } else {
        entries.sort((a, b) => (a.widget["display:order"] ?? 0) - (b.widget["display:order"] ?? 0));
    }
    return entries;
}

/**
 * Determine whether a widget is hidden.
 * Priority: settings["widget:hidden@<key>"] > widget["display:hidden"] > false
 */
function isWidgetHidden(settings: Record<string, any>, widgetKey: string, widgetConfig: WidgetConfigType): boolean {
    const settingsKey = `widget:hidden@${widgetKey}`;
    if (settingsKey in settings) {
        return Boolean(settings[settingsKey]);
    }
    return widgetConfig?.["display:hidden"] ?? false;
}

async function handleWidgetSelect(widget: WidgetConfigType) {
    // Special handling for devtools widget
    if (widget.blockdef?.meta?.view === "devtools") {
        getApi().toggleDevtools();
        return;
    }
    // Special handling for settings widget -- open in external editor
    if (widget.blockdef?.meta?.view === "settings") {
        try {
            const path = await invoke<string>("ensure_settings_file");
            await invoke("open_in_editor", { path });
        } catch (e) {
            console.error("Failed to open settings:", e);
        }
        return;
    }
    const blockDef = widget.blockdef;
    createBlock(blockDef, widget.magnified);
}

const ActionWidget = ({
    widget,
    widgetKey,
    iconOnly,
    settings,
}: {
    widget: WidgetConfigType;
    widgetKey?: string;
    iconOnly: boolean;
    settings: Record<string, any>;
}): JSX.Element => {
    if (widgetKey && isWidgetHidden(settings, widgetKey, widget)) {
        return null;
    }

    return (
        <div data-tauri-drag-region="false">
            <Tooltip
                content={widget.description || widget.label}
                placement="bottom"
                divClassName="flex flex-row items-center gap-1 px-2 py-0.5 text-secondary hover:bg-hoverbg hover:text-white cursor-pointer rounded-sm h-full"
                divOnClick={() => handleWidgetSelect(widget)}
            >
                <div style={{ color: widget.color }} class="text-sm">
                    <i class={makeIconClass(widget.icon, true, { defaultIcon: "browser" })}></i>
                </div>
                <Show when={!iconOnly && !isBlank(widget.label)}>
                    <div class="text-xs whitespace-nowrap">{widget.label}</div>
                </Show>
            </Tooltip>
        </div>
    );
};

const DRAG_THRESHOLD = 5;

const ActionWidgets = (): JSX.Element => {
    const { dragProps } = useWindowDrag();
    const fullConfig = atoms.fullConfigAtom;
    const settings = (): Record<string, any> => fullConfig()?.settings ?? {};
    const iconOnly = (): boolean => settings()["widget:icononly"] ?? false;
    const sortedWidgets = () => getSortedWidgets(fullConfig()?.widgets, settings());

    const [draggingKey, setDraggingKey] = createSignal<string | null>(null);
    const [dropIndex, setDropIndex] = createSignal<number | null>(null);
    let containerRef!: HTMLDivElement;
    let draggingKeyRef: string | null = null;
    let dropIndexRef: number | null = null;
    let dragStartRef: { x: number; y: number; key: string } | null = null;

    const handlePointerDown = (key: string, e: PointerEvent) => {
        dragStartRef = { x: e.clientX, y: e.clientY, key };
    };

    const handlePointerMove = (e: PointerEvent) => {
        if (!dragStartRef) return;

        if (!draggingKeyRef) {
            const dx = e.clientX - dragStartRef.x;
            const dy = e.clientY - dragStartRef.y;
            if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
            // Threshold crossed — start drag with pointer capture
            (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
            draggingKeyRef = dragStartRef.key;
            setDraggingKey(dragStartRef.key);
        }

        e.preventDefault();
        if (!containerRef) return;

        const slots = Array.from(containerRef.querySelectorAll<HTMLElement>("[data-widget-slot]"));
        let newIndex = slots.length;
        for (let i = 0; i < slots.length; i++) {
            const rect = slots[i].getBoundingClientRect();
            if (e.clientX <= rect.right) {
                newIndex = e.clientX <= rect.left + rect.width / 2 ? i : i + 1;
                break;
            }
        }
        if (newIndex !== dropIndexRef) {
            dropIndexRef = newIndex;
            setDropIndex(newIndex);
        }
    };

    const handlePointerUp = (_e: PointerEvent) => {
        const wasActuallyDragging = draggingKeyRef != null;
        const dk = draggingKeyRef;
        const di = dropIndexRef;

        dragStartRef = null;
        draggingKeyRef = null;
        dropIndexRef = null;
        setDraggingKey(null);
        setDropIndex(null);

        if (!wasActuallyDragging || dk == null || di == null) return;

        const currentWidgets = sortedWidgets();
        const baseNames = currentWidgets.map(({ key }) => key.replace("defwidget@", ""));
        const dragBaseName = dk.replace("defwidget@", "");
        const fromIdx = baseNames.indexOf(dragBaseName);
        if (fromIdx === -1) return;

        const next = [...baseNames];
        next.splice(fromIdx, 1);
        const adjustedDrop = fromIdx < di ? di - 1 : di;
        next.splice(adjustedDrop, 0, dragBaseName);

        if (next.join(",") !== baseNames.join(",")) {
            fireAndForget(async () => {
                await RpcApi.SetConfigCommand(TabRpcClient, { "widget:order": next } as any);
            });
        }
    };

    const handlePointerCancel = () => {
        dragStartRef = null;
        draggingKeyRef = null;
        dropIndexRef = null;
        setDraggingKey(null);
        setDropIndex(null);
    };

    const handleWidgetsBarContextMenu = (e: MouseEvent) => {
        e.preventDefault();
        const menu: ContextMenuItem[] = [
            {
                label: "Icon Only",
                type: "checkbox",
                checked: iconOnly(),
                click: () => {
                    fireAndForget(async () => {
                        await RpcApi.SetConfigCommand(TabRpcClient, { "widget:icononly": !iconOnly() } as any);
                    });
                },
            },
        ];
        ContextMenuModel.showContextMenu(menu, e);
    };

    return (
        <div
            ref={containerRef!}
            class="action-widgets"
            data-testid="action-widgets"
            onContextMenu={handleWidgetsBarContextMenu}
        >
            <For each={sortedWidgets()}>
                {({ key, widget }, idx) => (
                    <>
                        <Show when={draggingKey() != null && dropIndex() === idx() && draggingKey() !== key}>
                            <div class="action-widget-drop-indicator" />
                        </Show>
                        <div
                            class={`action-widget-slot${draggingKey() === key ? " dragging" : ""}`}
                            data-widget-slot={idx()}
                            data-tauri-drag-region="false"
                            onPointerDown={(e) => handlePointerDown(key, e)}
                            onPointerMove={handlePointerMove}
                            onPointerUp={handlePointerUp}
                            onPointerCancel={handlePointerCancel}
                        >
                            <ActionWidget widget={widget} widgetKey={key} iconOnly={iconOnly()} settings={settings()} />
                        </div>
                    </>
                )}
            </For>
            <Show when={draggingKey() != null && dropIndex() === sortedWidgets().length}>
                <div class="action-widget-drop-indicator" />
            </Show>
        </div>
    );
};

ActionWidgets.displayName = "ActionWidgets";

export { ActionWidgets };
