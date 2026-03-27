// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActionWidgets — Widget bar with pinned widgets + "More" overflow dropdown.
 *
 * Pinned widgets appear directly in the bar. Everything else lives in the More
 * dropdown. Users can pin/unpin via right-click on any widget.
 *
 * Settings keys:
 *   widget:pinned   — ordered short-name array (e.g. ["agent","terminal","sysinfo"])
 *   widget:icononly — icons only, no labels
 */

import { Tooltip } from "@/app/element/tooltip";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atoms, createBlock, getApi } from "@/store/global";
import { fireAndForget, isBlank, makeIconClass } from "@/util/util";
import { invoke } from "@tauri-apps/api/core";
import { createEffect, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import "./action-widgets.scss";

// ── Helpers ───────────────────────────────────────────────────────────────────

/**
 * Return the effective pinned short-names (no "defwidget@" prefix), in order.
 *
 * Priority:
 *  1. widget:pinned is set → authoritative
 *  2. Not set → derive from display:pinned in widget config
 */
function getPinnedKeys(
    settings: Record<string, any>,
    wmap: Record<string, WidgetConfigType>
): string[] {
    const pinned: string[] | undefined = settings["widget:pinned"];
    if (pinned !== undefined) {
        return pinned.filter((shortName) => wmap[`defwidget@${shortName}`] != null);
    }
    return Object.entries(wmap)
        .filter(([, w]) => w["display:pinned"])
        .sort(([, a], [, b]) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0))
        .map(([key]) => key.replace("defwidget@", ""));
}

function getPinnedWidgets(
    settings: Record<string, any>,
    wmap: Record<string, WidgetConfigType>
): { key: string; widget: WidgetConfigType }[] {
    if (!wmap) return [];
    return getPinnedKeys(settings, wmap)
        .map((shortName) => {
            const key = `defwidget@${shortName}`;
            return { key, widget: wmap[key] };
        })
        .filter((e) => e.widget != null);
}

function getMoreWidgets(
    settings: Record<string, any>,
    wmap: Record<string, WidgetConfigType>
): { key: string; widget: WidgetConfigType }[] {
    if (!wmap) return [];
    const pinnedSet = new Set(getPinnedKeys(settings, wmap));
    return Object.entries(wmap)
        .filter(([key]) => !pinnedSet.has(key.replace("defwidget@", "")))
        .sort(([, a], [, b]) => (a["display:order"] ?? 0) - (b["display:order"] ?? 0))
        .map(([key, widget]) => ({ key, widget }));
}

// ── Widget actions ────────────────────────────────────────────────────────────

async function handleWidgetSelect(widget: WidgetConfigType) {
    if (widget.blockdef?.meta?.view === "devtools") {
        getApi().toggleDevtools();
        return;
    }
    if (widget.blockdef?.meta?.view === "settings") {
        try {
            const path = await invoke<string>("ensure_settings_file");
            await invoke("open_in_editor", { path });
        } catch (e) {
            console.error("Failed to open settings:", e);
        }
        return;
    }
    createBlock(widget.blockdef, widget.magnified);
}

function pinWidget(shortName: string, settings: Record<string, any>, wmap: Record<string, WidgetConfigType>) {
    fireAndForget(async () => {
        const current = getPinnedKeys(settings, wmap);
        if (current.includes(shortName)) return;
        await RpcApi.SetConfigCommand(TabRpcClient, { "widget:pinned": [...current, shortName] } as any);
    });
}

function unpinWidget(shortName: string, settings: Record<string, any>, wmap: Record<string, WidgetConfigType>) {
    fireAndForget(async () => {
        const current = getPinnedKeys(settings, wmap);
        await RpcApi.SetConfigCommand(TabRpcClient, {
            "widget:pinned": current.filter((k) => k !== shortName),
        } as any);
    });
}


// ── ActionWidget ──────────────────────────────────────────────────────────────

const ActionWidget = ({
    widget,
    iconOnly,
    onContextMenu,
}: {
    widget: WidgetConfigType;
    iconOnly: boolean;
    onContextMenu?: (e: MouseEvent) => void;
}): JSX.Element => (
    <div onContextMenu={onContextMenu}>
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

ActionWidget.displayName = "ActionWidget";

// ── More dropdown ─────────────────────────────────────────────────────────────

const MoreDropdown = ({
    widgets,
    onClose,
    pos,
    settings,
    wmap,
    ref,
}: {
    widgets: () => { key: string; widget: WidgetConfigType }[];
    onClose: () => void;
    pos: () => { top: number; right: number };
    settings: () => Record<string, any>;
    wmap: () => Record<string, WidgetConfigType>;
    ref?: (el: HTMLDivElement) => void;
}): JSX.Element => {
    const handleItemClick = (widget: WidgetConfigType) => {
        handleWidgetSelect(widget);
        onClose();
    };

    const handleItemContextMenu = (e: MouseEvent, key: string) => {
        e.preventDefault();
        e.stopPropagation();
        const shortName = key.replace("defwidget@", "");
        ContextMenuModel.showContextMenu(
            [{ label: "Pin to bar", click: () => pinWidget(shortName, settings(), wmap()) }],
            e
        );
        onClose();
    };

    return (
        <div
            ref={ref}
            class="action-widget-more-dropdown"
            style={{ position: "fixed", top: `${pos().top}px`, right: `${pos().right}px` }}
        >
            <For each={widgets()}>
                {({ key, widget }) => (
                    <div
                        class="action-widget-more-item"
                        onClick={() => handleItemClick(widget)}
                        onContextMenu={(e) => handleItemContextMenu(e, key)}
                    >
                        <span class="action-widget-more-item-icon" style={{ color: widget.color }}>
                            <i class={makeIconClass(widget.icon, true, { defaultIcon: "browser" })}></i>
                        </span>
                        <span class="action-widget-more-item-label">{widget.label}</span>
                    </div>
                )}
            </For>
        </div>
    );
};

MoreDropdown.displayName = "MoreDropdown";

// ── Main ActionWidgets ────────────────────────────────────────────────────────

const DRAG_THRESHOLD = 5;

const ActionWidgets = (): JSX.Element => {
    const fullConfig = atoms.fullConfigAtom;
    const settings = (): Record<string, any> => fullConfig()?.settings ?? {};
    const wmap = (): Record<string, WidgetConfigType> => fullConfig()?.widgets ?? {};
    const iconOnly = (): boolean => settings()["widget:icononly"] ?? false;
    const pinnedWidgets = () => getPinnedWidgets(settings(), wmap());
    const moreWidgets = () => getMoreWidgets(settings(), wmap());

    // More dropdown state
    const [moreOpen, setMoreOpen] = createSignal(false);
    const [morePos, setMorePos] = createSignal<{ top: number; right: number }>({ top: 0, right: 0 });
    let moreButtonRef!: HTMLDivElement;
    let moreDropdownRef: HTMLDivElement | undefined;

    const openMore = (e: MouseEvent) => {
        if (moreOpen()) {
            setMoreOpen(false);
            return;
        }
        const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
        setMorePos({ top: rect.bottom + 4, right: window.innerWidth - rect.right });
        setMoreOpen(true);
    };

    const closeMore = () => setMoreOpen(false);

    // Close on outside click — ignore clicks inside button or dropdown
    createEffect(() => {
        if (!moreOpen()) return;
        const handler = (e: MouseEvent) => {
            const t = e.target as Node;
            if (moreButtonRef?.contains(t) || moreDropdownRef?.contains(t)) return;
            setMoreOpen(false);
        };
        document.addEventListener("mousedown", handler, true);
        onCleanup(() => document.removeEventListener("mousedown", handler, true));
    });

    // ── Drag-to-reorder (pinned only, saves to widget:pinned) ─────────────────

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
        const current = pinnedWidgets();
        const shortNames = current.map(({ key }) => key.replace("defwidget@", ""));
        const dragShort = dk.replace("defwidget@", "");
        const fromIdx = shortNames.indexOf(dragShort);
        if (fromIdx === -1) return;
        const next = [...shortNames];
        next.splice(fromIdx, 1);
        const adjustedDrop = fromIdx < di ? di - 1 : di;
        next.splice(adjustedDrop, 0, dragShort);
        if (next.join(",") !== shortNames.join(",")) {
            fireAndForget(async () => {
                await RpcApi.SetConfigCommand(TabRpcClient, { "widget:pinned": next } as any);
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

    // ── Context menus ─────────────────────────────────────────────────────────

    const handleBarContextMenu = (e: MouseEvent) => {
        e.preventDefault();
        ContextMenuModel.showContextMenu(
            [
                {
                    label: "Icon Only",
                    type: "checkbox",
                    checked: iconOnly(),
                    click: () => {
                        fireAndForget(async () => {
                            await RpcApi.SetConfigCommand(TabRpcClient, {
                                "widget:icononly": !iconOnly(),
                            } as any);
                        });
                    },
                },
            ],
            e
        );
    };

    const handlePinnedContextMenu = (e: MouseEvent, key: string) => {
        e.preventDefault();
        e.stopPropagation();
        const shortName = key.replace("defwidget@", "");
        ContextMenuModel.showContextMenu(
            [{ label: "Unpin from bar", click: () => unpinWidget(shortName, settings(), wmap()) }],
            e
        );
    };

    // ── Render ────────────────────────────────────────────────────────────────

    return (
        <>
            <div
                ref={containerRef}
                class="action-widgets"
                data-testid="action-widgets"
                onContextMenu={handleBarContextMenu}
            >
                <For each={pinnedWidgets()}>
                    {({ key, widget }, idx) => (
                        <>
                            <Show when={draggingKey() != null && dropIndex() === idx() && draggingKey() !== key}>
                                <div class="action-widget-drop-indicator" />
                            </Show>
                            <div
                                class={`action-widget-slot${draggingKey() === key ? " dragging" : ""}`}
                                data-widget-slot={idx()}
                                onPointerDown={(e) => handlePointerDown(key, e)}
                                onPointerMove={handlePointerMove}
                                onPointerUp={handlePointerUp}
                                onPointerCancel={handlePointerCancel}
                            >
                                <ActionWidget
                                    widget={widget}
                                    iconOnly={iconOnly()}
                                    onContextMenu={(e) => handlePinnedContextMenu(e, key)}
                                />
                            </div>
                        </>
                    )}
                </For>
                <Show when={draggingKey() != null && dropIndex() === pinnedWidgets().length}>
                    <div class="action-widget-drop-indicator" />
                </Show>

                <Show when={moreWidgets().length > 0}>
                    <div
                        ref={moreButtonRef}
                        class="action-widget-more-btn"
                        classList={{ open: moreOpen() }}
                        onClick={openMore}
                    >
                        <i class="fa-solid fa-ellipsis" />
                        <Show when={!iconOnly()}>
                            <span class="action-widget-more-label">more</span>
                        </Show>
                        <i
                            class={`fa-solid ${moreOpen() ? "fa-chevron-up" : "fa-chevron-down"} action-widget-more-chevron`}
                        />
                    </div>
                </Show>
            </div>

            <Portal>
                <Show when={moreOpen()}>
                    <MoreDropdown
                        widgets={moreWidgets}
                        onClose={closeMore}
                        pos={morePos}
                        settings={settings}
                        wmap={wmap}
                        ref={(el) => (moreDropdownRef = el)}
                    />
                </Show>
            </Portal>
        </>
    );
};

ActionWidgets.displayName = "ActionWidgets";

export { ActionWidgets };
