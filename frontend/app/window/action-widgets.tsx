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
import { fireAndForget, isBlank, makeIconClass } from "@/util/util";
import { invoke } from "@tauri-apps/api/core";
import { useAtomValue } from "jotai";
import { Fragment, memo, useCallback, useRef, useState } from "react";
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

const ActionWidget = memo(
    ({
        widget,
        widgetKey,
        iconOnly,
        settings,
    }: {
        widget: WidgetConfigType;
        widgetKey?: string;
        iconOnly: boolean;
        settings: Record<string, any>;
    }) => {
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
                    <div style={{ color: widget.color }} className="text-sm">
                        <i className={makeIconClass(widget.icon, true, { defaultIcon: "browser" })}></i>
                    </div>
                    {!iconOnly && !isBlank(widget.label) && (
                        <div className="text-xs whitespace-nowrap">{widget.label}</div>
                    )}
                </Tooltip>
            </div>
        );
    }
);

ActionWidget.displayName = "ActionWidget";

const ActionWidgets = memo(() => {
    const fullConfig = useAtomValue(atoms.fullConfigAtom);
    const settings: Record<string, any> = fullConfig?.settings ?? {};
    const iconOnly = settings["widget:icononly"] ?? false;
    const sortedWidgets = getSortedWidgets(fullConfig?.widgets, settings);

    const [draggingKey, setDraggingKey] = useState<string | null>(null);
    const [dropIndex, setDropIndex] = useState<number | null>(null);
    const containerRef = useRef<HTMLDivElement>(null);
    const draggingKeyRef = useRef<string | null>(null);
    const dropIndexRef = useRef<number | null>(null);

    const handleDragStart = useCallback((key: string, e: React.DragEvent) => {
        e.dataTransfer.effectAllowed = "move";
        e.dataTransfer.setData("text/plain", key);
        draggingKeyRef.current = key;
        setDraggingKey(key);
    }, []);

    const handleContainerDragOver = useCallback((e: React.DragEvent) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        if (!containerRef.current) return;
        const slots = Array.from(
            containerRef.current.querySelectorAll<HTMLElement>("[data-widget-slot]")
        );
        let newIndex = slots.length;
        for (let i = 0; i < slots.length; i++) {
            const rect = slots[i].getBoundingClientRect();
            if (e.clientX <= rect.right) {
                newIndex = e.clientX <= rect.left + rect.width / 2 ? i : i + 1;
                break;
            }
        }
        if (newIndex !== dropIndexRef.current) {
            dropIndexRef.current = newIndex;
            setDropIndex(newIndex);
        }
    }, []);

    const handleDrop = useCallback(
        (e: React.DragEvent) => {
            e.preventDefault();
            const dk = draggingKeyRef.current;
            const di = dropIndexRef.current;
            if (dk == null || di == null) return;

            draggingKeyRef.current = null;
            dropIndexRef.current = null;
            setDraggingKey(null);
            setDropIndex(null);

            const baseNames = sortedWidgets.map(({ key }) => key.replace("defwidget@", ""));
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
        },
        [sortedWidgets]
    );

    const handleDragEnd = useCallback(() => {
        draggingKeyRef.current = null;
        dropIndexRef.current = null;
        setDraggingKey(null);
        setDropIndex(null);
    }, []);

    const handleWidgetsBarContextMenu = (e: React.MouseEvent) => {
        e.preventDefault();
        const menu: ContextMenuItem[] = [
            {
                label: "Icon Only",
                type: "checkbox",
                checked: iconOnly,
                click: () => {
                    fireAndForget(async () => {
                        await RpcApi.SetConfigCommand(TabRpcClient, { "widget:icononly": !iconOnly } as any);
                    });
                },
            },
        ];
        ContextMenuModel.showContextMenu(menu, e);
    };

    const isDragging = draggingKey != null;

    return (
        <div
            ref={containerRef}
            className="action-widgets"
            data-testid="action-widgets"
            onContextMenu={handleWidgetsBarContextMenu}
            onDragOver={handleContainerDragOver}
            onDrop={handleDrop}
        >
            {sortedWidgets.map(({ key, widget }, idx) => (
                <Fragment key={key}>
                    {isDragging && dropIndex === idx && draggingKey !== key && (
                        <div className="action-widget-drop-indicator" />
                    )}
                    <div
                        className={`action-widget-slot${draggingKey === key ? " dragging" : ""}`}
                        draggable
                        data-widget-slot={idx}
                        data-tauri-drag-region="false"
                        onDragStart={(e) => handleDragStart(key, e)}
                        onDragEnd={handleDragEnd}
                    >
                        <ActionWidget widget={widget} widgetKey={key} iconOnly={iconOnly} settings={settings} />
                    </div>
                </Fragment>
            ))}
            {isDragging && dropIndex === sortedWidgets.length && (
                <div className="action-widget-drop-indicator" />
            )}
        </div>
    );
});

ActionWidgets.displayName = "ActionWidgets";

export { ActionWidgets };
