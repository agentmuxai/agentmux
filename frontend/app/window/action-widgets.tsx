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
 *
 * Split (see also):
 *   action-widgets-config.ts    — pure/RPC config helpers (pinned/more derivation, pin/unpin)
 *   more-dropdown.tsx           — the "More" overflow dropdown
 *   use-widget-bar-responsive.ts — responsive 3-tier collapse hook
 *   use-widget-drag-reorder.ts  — drag-to-reorder state machine
 */

import { Tooltip } from "@/app/element/tooltip";
import { PopoverMenu, type PopoverMenuItem } from "@/app/element/popover-menu";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { TabRpcClient } from "@/app/store/rpc-util";
import { atoms, getApi } from "@/store/global";
import { fireAndForget, isBlank, makeIconClass } from "@/util/util";
import { createEffect, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import {
    getMoreWidgets,
    getPinnedKeys,
    getPinnedWidgets,
    handleWidgetSelect,
    pinWidget,
    unpinWidget,
} from "./action-widgets-config";
import { MoreDropdown } from "./more-dropdown";
import { useWidgetBarResponsive } from "./use-widget-bar-responsive";
import { useWidgetDragReorder } from "./use-widget-drag-reorder";
import "./action-widgets.scss";

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
            divClassName="flex flex-row items-center gap-1 px-2 py-0.5 text-secondary hover:bg-hoverbg hover:text-white rounded-sm h-full"
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

// ── Main ActionWidgets ────────────────────────────────────────────────────────

const ActionWidgets = (): JSX.Element => {
    const fullConfig = atoms.fullConfigAtom;
    const settings = (): Record<string, any> => fullConfig()?.settings ?? {};
    const wmap = (): Record<string, WidgetConfigType> => fullConfig()?.widgets ?? {};
    const iconOnly = (): boolean => settings()["widget:icononly"] ?? false;
    const pinnedWidgets = () => getPinnedWidgets(settings(), wmap());
    const moreWidgets = () => getMoreWidgets(settings(), wmap());

    let containerRef!: HTMLDivElement;
    let moreButtonRef!: HTMLDivElement;
    let moreDropdownRef: HTMLDivElement | undefined;

    // ── Responsive labels — 3-tier collapse ────────────────────────────────
    const {
        showWidgetLabels,
        visiblePinnedWidgets,
        clippedPinnedWidgets,
        clipCount,
        setMirrorRef,
        setIconMirrorRef,
        setIconMirrorMoreRef,
    } = useWidgetBarResponsive({
        containerRef: () => containerRef,
        moreButtonRef: () => moreButtonRef,
        pinnedWidgets,
        moreWidgets,
        iconOnly,
    });

    // ── Drag-to-reorder (pinned only, saves to widget:pinned) ──────────────
    const {
        draggingKey,
        dropIndex,
        handlePointerDown,
        handlePointerMove,
        handlePointerUp,
        handlePointerCancel,
    } = useWidgetDragReorder({
        containerRef: () => containerRef,
        pinnedWidgets,
    });

    // More dropdown state
    const [moreOpen, setMoreOpen] = createSignal(false);

    const openMore = (_e: MouseEvent) => {
        setMoreOpen(!moreOpen());
    };

    const closeMore = () => setMoreOpen(false);

    // Item context menu — rendered independently so the More dropdown stays open
    // while the user interacts with pin/unpin. itemMenuState drives a PopoverMenu
    // in the JSX below; null means closed.
    const [itemMenuState, setItemMenuState] = createSignal<{
        pos: { x: number; y: number };
        shortName: string;
    } | null>(null);

    const handleItemContextMenu = (pos: { x: number; y: number }, key: string) => {
        setItemMenuState({ pos, shortName: key.replace("defwidget@", "") });
    };

    const resolveWidgetDef = (shortName: string) =>
        wmap()[`defwidget@${shortName}`] ?? null;

    const buildItemMenuItems = (shortName: string): PopoverMenuItem[] => {
        const widgetDef = resolveWidgetDef(shortName);
        const blockMeta = widgetDef?.blockdef?.meta as Record<string, unknown> | undefined;
        const view = (blockMeta?.["view"] as string) ?? null;
        return [
            {
                label: "Open in New Window",
                click: () => {
                    closeMore();
                    if (view) fireAndForget(async () => getApi().openNewWindowWithView(view, blockMeta));
                    else fireAndForget(async () => getApi().openNewWindow());
                },
            },
            {
                label: "Open in Floating Pane",
                click: () => {
                    closeMore();
                    if (!view || !blockMeta) return;
                    fireAndForget(async () =>
                        TabRpcClient.rpcCall("pane.open", { view, meta: blockMeta, floating: true }, {})
                    );
                },
            },
            { type: "separator" } as PopoverMenuItem,
            getPinnedKeys(settings(), wmap()).includes(shortName)
                ? { label: "Unpin from bar", click: () => { unpinWidget(shortName, settings(), wmap()); } }
                : { label: "Pin to bar", click: () => { pinWidget(shortName, settings(), wmap()); } },
        ];
    };

    // Close on outside click — ignore clicks inside button, dropdown, or any
    // open popover-menu (e.g. the item context menu rendered by handleItemContextMenu).
    createEffect(() => {
        if (!moreOpen()) return;
        const handler = (e: MouseEvent) => {
            const t = e.target as Node;
            if (moreButtonRef?.contains(t) || moreDropdownRef?.contains(t)) return;
            const el = t instanceof Element ? t : (t as Node).parentElement;
            if (el?.closest(".popover-menu")) return;
            setMoreOpen(false);
        };
        document.addEventListener("mousedown", handler, true);
        onCleanup(() => document.removeEventListener("mousedown", handler, true));
    });

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
    // Right-clicking the gaps between pinned widgets used to open its own
    // one-item ("Icon Only") menu here, separate from — and hidden behind —
    // the unified TitleBarContextMenu the rest of the empty tab-bar space
    // opens. Removed: the bar's own onContextMenu no longer intercepts the
    // event, so it now bubbles up to window-header's handler like any other
    // empty-space right-click, landing on the single shared menu (which
    // already has its own "Show Icons Only" entry).

    const handlePinnedContextMenu = (e: MouseEvent, key: string) => {
        e.preventDefault();
        e.stopPropagation();
        handlePointerCancel();
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
            >
                <For each={visiblePinnedWidgets()}>
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
                                    iconOnly={!showWidgetLabels()}
                                    onContextMenu={(e) => handlePinnedContextMenu(e, key)}
                                />
                            </div>
                        </>
                    )}
                </For>
                <Show when={draggingKey() != null && dropIndex() === visiblePinnedWidgets().length}>
                    <div class="action-widget-drop-indicator" />
                </Show>

                <Show when={moreWidgets().length > 0 || clipCount() > 0}>
                    <div
                        ref={moreButtonRef}
                        class="action-widget-more-btn"
                        classList={{ open: moreOpen() }}
                        onClick={openMore}
                    >
                        <i class="fa-solid fa-ellipsis" />
                        <Show when={showWidgetLabels()}>
                            <span class="action-widget-more-label">more</span>
                        </Show>
                        <i
                            class={`fa-solid ${moreOpen() ? "fa-chevron-up" : "fa-chevron-down"} action-widget-more-chevron`}
                        />
                    </div>
                </Show>
            </div>

            {/* Mirror 1: always-labeled — measures tier 1→2 threshold. */}
            <div ref={setMirrorRef} class="action-widgets action-widgets--measure" aria-hidden="true">
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

            {/* Mirror 2: icon-only — measures tier 2→3 threshold.
                Includes the More button when unpinned widgets exist because
                the tier-2 bar still shows it, so its width counts. */}
            <div ref={setIconMirrorRef} class="action-widgets action-widgets--measure" aria-hidden="true">
                <For each={pinnedWidgets()}>
                    {({ widget }) => (
                        <div class="action-widget-slot">
                            <ActionWidget widget={widget} iconOnly={true} />
                        </div>
                    )}
                </For>
                <Show when={moreWidgets().length > 0}>
                    <div class="action-widget-more-btn">
                        <i class="fa-solid fa-ellipsis" />
                        <i class="fa-solid fa-chevron-down action-widget-more-chevron" />
                    </div>
                </Show>
            </div>

            {/* More button width probe — always mounted so moreBtnW is available
                before the live More button mounts on first tier-3 entry. */}
            <div class="action-widgets action-widgets--measure" aria-hidden="true">
                <div ref={setIconMirrorMoreRef} class="action-widget-more-btn">
                    <i class="fa-solid fa-ellipsis" />
                    <i class="fa-solid fa-chevron-down action-widget-more-chevron" />
                </div>
            </div>

            <Portal>
                <Show when={moreOpen()}>
                    <MoreDropdown
                        widgets={() => [...clippedPinnedWidgets(), ...moreWidgets()]}
                        onClose={closeMore}
                        onItemContextMenu={handleItemContextMenu}
                        anchor={() => moreButtonRef ?? null}
                        settings={settings}
                        wmap={wmap}
                        ref={(el) => (moreDropdownRef = el)}
                    />
                </Show>
            </Portal>

            <Show when={itemMenuState() !== null}>
                <PopoverMenu
                    pos={itemMenuState()!.pos}
                    onClose={() => setItemMenuState(null)}
                    items={buildItemMenuItems(itemMenuState()!.shortName)}
                />
            </Show>
        </>
    );
};

ActionWidgets.displayName = "ActionWidgets";

export { ActionWidgets };
