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
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { atoms, createBlock, getApi } from "@/store/global";
import { fireAndForget, isBlank, makeIconClass } from "@/util/util";
import { invokeCommand } from "@/app/platform/ipc";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { computeMenuPosition } from "@/app/util/menu-position";
import { autoUpdate } from "@floating-ui/dom";
import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
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

// NOTE: don't destructure props in the signature — Solid props are getters
// that lose reactivity when destructured outside an effect (Solid issue
// #1224). Access via `props.iconOnly` so the `<Show>` below stays reactive
// to upstream `tooNarrow` changes. Without this fix the responsive widget
// bar collapse-to-icons never visually applies even though `tooNarrow`
// computes correctly — diagnosed live via the fe_log pipe.
const ActionWidget = (props: {
    widget: WidgetConfigType;
    iconOnly: boolean;
    onContextMenu?: (e: MouseEvent) => void;
}): JSX.Element => (
    <div onContextMenu={props.onContextMenu}>
        <Tooltip
            content={props.widget.description || props.widget.label}
            placement="bottom"
            divClassName="flex flex-row items-center gap-1 px-2 py-0.5 text-secondary hover:bg-hoverbg hover:text-white cursor-pointer rounded-sm h-full"
            divOnClick={() => handleWidgetSelect(props.widget)}
        >
            <div class="widget-icon text-sm">
                <i class={makeIconClass(props.widget.icon, true, { defaultIcon: "browser" })}></i>
            </div>
            <Show when={!props.iconOnly && !isBlank(props.widget.label)}>
                <div class="text-xs whitespace-nowrap">{props.widget.label}</div>
            </Show>
        </Tooltip>
    </div>
);

ActionWidget.displayName = "ActionWidget";

// ── More dropdown ─────────────────────────────────────────────────────────────

const MoreDropdown = ({
    widgets,
    onClose,
    anchor,
    settings,
    wmap,
    ref,
}: {
    widgets: () => { key: string; widget: WidgetConfigType }[];
    onClose: () => void;
    anchor: () => HTMLElement | null;
    settings: () => Record<string, any>;
    wmap: () => Record<string, WidgetConfigType>;
    ref?: (el: HTMLDivElement) => void;
}): JSX.Element => {
    let overlayEl: HTMLDivElement | undefined;
    // Cut a transparent hole through any browser pane HWND behind this
    // dropdown so DOM renders above the pane in this rect.
    // See `frontend/app/platform/pane-overlay.ts`.
    usePaneOverlay(() => overlayEl);

    // Positioning routes through the shared primitive (Phase 3): anchored to
    // the More button, preferred placement bottom-end (right-aligned, as
    // before). flip/shift/size + the paintable-area boundary replace the old
    // hand-rolled `window.innerWidth - rect.right` clamp.
    const [floatingStyle, setFloatingStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
    });
    let cleanupAutoUpdate: (() => void) | null = null;

    const registerFloating = (el: HTMLDivElement) => {
        overlayEl = el;
        ref?.(el);
        requestAnimationFrame(() => {
            const anchorEl = anchor();
            if (!(anchorEl instanceof Element) || !(el instanceof Element)) return;
            const update = async () => {
                const a = anchor();
                if (!a) return;
                const pos = await computeMenuPosition(
                    { anchor: a, placement: "bottom-end", gutter: 4 },
                    el,
                );
                setFloatingStyle(pos.style);
            };
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = autoUpdate(anchorEl, el, update);
        });
    };

    onCleanup(() => cleanupAutoUpdate?.());

    const handleItemClick = (widget: WidgetConfigType) => {
        handleWidgetSelect(widget);
        onClose();
    };

    const handleItemContextMenu = (e: MouseEvent, key: string) => {
        e.preventDefault();
        e.stopPropagation();
        const shortName = key.replace("defwidget@", "");
        ContextMenuModel.showContextMenu(
            [
                { label: "New Window", click: () => {
                    fireAndForget(async () => {
                        await getApi().openNewWindow();
                    });
                }},
                { type: "separator" },
                { label: "Pin to bar", click: () => pinWidget(shortName, settings(), wmap()) },
            ],
            e
        );
        onClose();
    };

    return (
        <div
            ref={registerFloating}
            class="action-widget-more-dropdown"
            style={floatingStyle()}
            data-pane-overlay
        >
            <For each={widgets()}>
                {({ key, widget }) => (
                    <div
                        class="action-widget-more-item"
                        onClick={() => handleItemClick(widget)}
                        onContextMenu={(e) => handleItemContextMenu(e, key)}
                    >
                        <span class="action-widget-more-item-icon widget-icon">
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

    // ── Responsive labels ─────────────────────────────────────────────────────
    // Labels collapse to icons when the title bar is too narrow to fit the
    // labeled bar while leaving the tab bar a usable strip. `tooNarrow` is
    // derived from a hidden always-labeled mirror (rendered below), so the
    // decision never depends on the visible bar's own collapsed state — there
    // is no measure→collapse→re-expand oscillation.
    const [tooNarrow, setTooNarrow] = createSignal(false);
    const effectiveIconOnly = (): boolean => iconOnly() || tooNarrow();
    let mirrorRef: HTMLDivElement | undefined;

    // Header width always reserved for the tab bar before widget labels are
    // dropped. The collapse point still auto-tracks the widget count via the
    // mirror's measured width; this only floors the tab bar's share.
    const MIN_TAB_WIDTH = 120;

    // More dropdown state
    const [moreOpen, setMoreOpen] = createSignal(false);
    let moreButtonRef!: HTMLDivElement;
    let moreDropdownRef: HTMLDivElement | undefined;

    const openMore = (_e: MouseEvent) => {
        setMoreOpen(!moreOpen());
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

    // Re-evaluate `tooNarrow` whenever the window header resizes or the mirror's
    // labeled width changes (e.g. a widget is pinned/unpinned).
    onMount(() => {
        const header = containerRef?.closest(".window-header") as HTMLElement | null;
        if (!header || !mirrorRef) return;
        const buttons = containerRef?.parentElement?.querySelector(
            ".window-action-buttons"
        ) as HTMLElement | null;
        const measure = () => {
            const labeledW = mirrorRef?.offsetWidth ?? 0;
            const headerW = header.clientWidth;
            if (labeledW === 0 || headerW === 0) return;
            const buttonsW = buttons?.offsetWidth ?? 0;
            setTooNarrow(labeledW + buttonsW + MIN_TAB_WIDTH > headerW);
        };
        const ro = new ResizeObserver(measure);
        ro.observe(header);
        ro.observe(mirrorRef);
        measure();
        onCleanup(() => ro.disconnect());
    });

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

    // ── Context menu active state (keeps hover highlight while menu is open) ──
    const [contextMenuActiveKey, setContextMenuActiveKey] = createSignal<string | null>(null);
    let contextMenuCleanup: (() => void) | null = null;

    function armContextMenuDismiss(key: string) {
        contextMenuCleanup?.();
        setContextMenuActiveKey(key);
        const clear = () => {
            setContextMenuActiveKey(null);
            document.removeEventListener("mousedown", clear);
            document.removeEventListener("keydown", onKey);
            contextMenuCleanup = null;
        };
        const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") clear(); };
        document.addEventListener("mousedown", clear, { once: true });
        document.addEventListener("keydown", onKey);
        contextMenuCleanup = clear;
    }

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
        armContextMenuDismiss(key);
        const shortName = key.replace("defwidget@", "");
        ContextMenuModel.showContextMenu(
            [
                { label: "New Window", click: () => {
                    fireAndForget(async () => {
                        await getApi().openNewWindow();
                    });
                }},
                { type: "separator" },
                { label: "Unpin from bar", click: () => unpinWidget(shortName, settings(), wmap()) },
            ],
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
                data-drag-region="false"
                onContextMenu={handleBarContextMenu}
            >
                <For each={pinnedWidgets()}>
                    {({ key, widget }, idx) => (
                        <>
                            <Show when={draggingKey() != null && dropIndex() === idx() && draggingKey() !== key}>
                                <div class="action-widget-drop-indicator" />
                            </Show>
                            <div
                                class={`action-widget-slot${draggingKey() === key ? " dragging" : ""}${contextMenuActiveKey() === key ? " context-active" : ""}`}
                                data-widget-slot={idx()}
                                onPointerDown={(e) => handlePointerDown(key, e)}
                                onPointerMove={handlePointerMove}
                                onPointerUp={handlePointerUp}
                                onPointerCancel={handlePointerCancel}
                            >
                                <ActionWidget
                                    widget={widget}
                                    iconOnly={effectiveIconOnly()}
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
                        <Show when={!effectiveIconOnly()}>
                            <span class="action-widget-more-label">more</span>
                        </Show>
                        <i
                            class={`fa-solid ${moreOpen() ? "fa-chevron-up" : "fa-chevron-down"} action-widget-more-chevron`}
                        />
                    </div>
                </Show>
            </div>

            {/* Hidden always-labeled mirror. Measured to decide whether the
                labeled bar fits; absolutely positioned so it never affects the
                visible bar's layout, and inert (invisible, no pointer events). */}
            <div ref={mirrorRef} class="action-widgets action-widgets--measure" aria-hidden="true">
                <For each={pinnedWidgets()}>
                    {({ widget }) => (
                        <div class="action-widget-slot">
                            <ActionWidget widget={widget} iconOnly={false} />
                        </div>
                    )}
                </For>
                <Show when={moreWidgets().length > 0}>
                    <div class="action-widget-more-btn">
                        <i class="fa-solid fa-ellipsis" />
                        <span class="action-widget-more-label">more</span>
                        <i class="fa-solid fa-chevron-down action-widget-more-chevron" />
                    </div>
                </Show>
            </div>

            <Portal>
                <Show when={moreOpen()}>
                    <MoreDropdown
                        widgets={moreWidgets}
                        onClose={closeMore}
                        anchor={() => moreButtonRef ?? null}
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
