// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActionWidgets — Widget bar with pinned widgets + "More" overflow dropdown.
 *
 * Pinned widgets appear directly in the bar. Everything else lives in the More
 * dropdown. Users can pin/unpin via right-click on any widget.
 *
 * Settings keys: see action-widgets-config.ts.
 *
 * Split (see also):
 *   action-widgets-config.ts    — pure/RPC config helpers (pinned/more derivation, pin/unpin)
 *   more-dropdown.tsx           — the "More" overflow dropdown
 *   use-widget-bar-responsive.ts — responsive 3-tier collapse hook
 *   use-widget-drag-reorder.ts  — drag-to-reorder state machine
 */

import { Tooltip } from "@/app/element/tooltip";
import { PopoverMenu, type PopoverMenuItem } from "@/app/element/popover-menu";
import { openMenuAt } from "@/app/element/ui";
import { MENU_ICON } from "@/app/util/menu-icons";
import {
    createPeerRegistry,
    createSubmenuHover,
    type PeerRegistry,
    type SubmenuHoverController,
} from "@/app/util/submenu-hover";
import { atoms } from "@/store/global";
import { isBlank, makeIconClass } from "@/util/util";
import {
    createEffect,
    createSignal,
    For,
    onCleanup,
    Show,
    type Accessor,
    type JSX,
    type Setter,
} from "solid-js";
import {
    getMoreWidgets,
    getPinnedKeys,
    getPinnedWidgets,
    getSavedPinNames,
    openWidget,
    preloadWidget,
    keyOfPinId,
    pinIdOf,
    pinWidget,
    UNHIDEABLE_WIDGET_KEYS,
    unpinWidget,
    widgetIconStyle,
} from "./action-widgets-config";
import { hideFromWidgetBar } from "./widget-bar-hide";
import { buildWidgetOpenActions } from "./action-widgets-menu";
import { MoreDropdown } from "./more-dropdown";
import { closeLogoMenu } from "./logo-menu";
import { PinnedWidgetFlyout } from "./pinned-widget-flyout";
import { useWidgetBarResponsive } from "./use-widget-bar-responsive";
import { useWidgetDragReorder } from "./use-widget-drag-reorder";
import { KanbanNeedsBadge, opensKanban } from "@/app/view/kanban/kanban-needs-badge";
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
    onClick?: () => void;
    onMouseEnter?: () => void;
    onMouseLeave?: (e: MouseEvent) => void;
}): JSX.Element => (
    <div
        onContextMenu={props.onContextMenu}
        onMouseEnter={() => props.onMouseEnter?.()}
        onMouseLeave={(e) => props.onMouseLeave?.(e)}
        onPointerEnter={() => preloadWidget(props.widget)}
    >
        <Tooltip
            content={props.widget.description || props.widget.label}
            placement="bottom"
            divClassName="flex flex-row items-center gap-1 px-2 py-0.5 text-secondary hover:bg-hoverbg hover:text-foreground rounded-sm h-full"
            divOnClick={props.onClick ?? ((e) => openWidget(props.widget, { forceNew: e?.shiftKey }))}
        >
            <div class="widget-icon text-sm" style={widgetIconStyle(props.widget)}>
                <i class={makeIconClass(props.widget.icon, true, { defaultIcon: "window-maximize" })}></i>
                <Show when={opensKanban(props.widget)}>
                    <KanbanNeedsBadge />
                </Show>
            </div>
            <Show when={!props.iconOnly && !isBlank(props.widget.label)}>
                <div class="text-xs whitespace-nowrap">{props.widget.label}</div>
            </Show>
            <Show when={(props.widget.children?.length ?? 0) > 0}>
                <i class="fa-solid fa-chevron-down action-widget-parent-chevron" />
            </Show>
        </Tooltip>
    </div>
);

ActionWidget.displayName = "ActionWidget";

type Settings = Record<string, any>;
type WidgetMap = Record<string, WidgetConfigType>;
type WidgetEntry = { key: string; widget: WidgetConfigType };
type ItemMenuState = { pos: { x: number; y: number }; key: string; shortName: string };
type Responsive = ReturnType<typeof useWidgetBarResponsive>;
type Drag = ReturnType<typeof useWidgetDragReorder>;


// The More button gets its own createSubmenuHover controller, registered
// as a peer in the SAME parentPeers registry pinned parents use — so
// hovering onto Messengers closes More and hovering onto More closes
// Messengers, symmetrically. Before this, More was click-only: hovering
// onto a pinned parent still force-closed it (via the peer-close in `renderPinnedSlot`), but
// hovering back onto More never reopened it, only an explicit click did.
const MORE_PEER_KEY = "__more__";

/** What the widget bar and the functions that serve it share. */
interface BarState {
    settings: () => Settings;
    wmap: () => WidgetMap;
    iconOnly: () => boolean;
    pinnedWidgets: () => WidgetEntry[];
    moreWidgets: () => WidgetEntry[];
    containerRef: HTMLDivElement | undefined;
    moreButtonRef: HTMLDivElement | undefined;
    moreDropdownRef: HTMLDivElement | undefined;
    pinnedFlyoutRef: HTMLDivElement | undefined;
    openParentKey: Accessor<string | null>;
    setOpenParentKey: Setter<string | null>;
    parentPeers: PeerRegistry;
    parentHoverControllers: Map<string, SubmenuHoverController>;
    parentSlotRefs: Map<string, HTMLDivElement>;
    // Whether the currently-open pinned parent flyout was confirmed by an
    // explicit click, as opposed to merely being open from hover-intent
    // (SPEC_WIDGET_BAR_HOVER_CLICK_PREMATURE_CLOSE_2026_08_20.md). Only one
    // pinned-parent flyout can be open at a time (peers force-close each
    // other), so a single non-reactive flag alongside `openParentKey` is
    // enough — no per-key map needed. Reset in the controller's own
    // onClose in `registerParentHover`, so every close path (hover-away, outside click,
    // Escape, a peer forcing it shut) clears it for free.
    parentOpenConfirmedByClick: boolean;
    moreOpen: Accessor<boolean>;
    setMoreOpen: Setter<boolean>;
    moreConfirmedByClick: Accessor<boolean>;
    setMoreConfirmedByClick: Setter<boolean>;
    moreHover: SubmenuHoverController;
    unregisterMoreHover: () => void;
    itemMenuState: Accessor<ItemMenuState | null>;
    setItemMenuState: Setter<ItemMenuState | null>;
    contextMenuActiveKey: Accessor<string | null>;
    setContextMenuActiveKey: Setter<string | null>;
    contextMenuCleanup: (() => void) | null;
}

function createBarState(): BarState {
    const fullConfig = atoms.fullConfigAtom;
    const settings = (): Settings => fullConfig()?.settings ?? {};
    const wmap = (): WidgetMap => fullConfig()?.widgets ?? {};
    const iconOnly = (): boolean => settings()["widget:icononly"] ?? false;
    const pinnedWidgets = () => getPinnedWidgets(settings(), wmap());
    const moreWidgets = () => getMoreWidgets(settings(), wmap());

    // ── Pinned-parent flyout (Case A, SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md §3.3) ──
    // Exactly one pinned parent's flyout can be open at a time — a single
    // scalar (rather than a per-key visible map) is the source of truth for
    // which one, and rendering is keyed directly off it. Each parent row still
    // gets its own createSubmenuHover controller (open-delay + safe-triangle
    // close) registered in a shared peer registry so hovering onto a new
    // parent row force-closes whichever one is currently open, same as any
    // other peer-submenu level in the app.
    const [openParentKey, setOpenParentKey] = createSignal<string | null>(null);
    const parentPeers = createPeerRegistry();
    const parentHoverControllers = new Map<string, SubmenuHoverController>();
    const parentSlotRefs = new Map<string, HTMLDivElement>();


    // More dropdown state
    const [moreOpen, setMoreOpen] = createSignal(false);

    // Whether the current open state was confirmed by an explicit click, as
    // opposed to merely being open from hover-intent — same rationale as
    // `parentOpenConfirmedByClick` in `BarState`
    // (SPEC_WIDGET_BAR_HOVER_CLICK_PREMATURE_CLOSE_2026_08_20.md). Reset in
    // onClose so every close path clears it for free.
    const [moreConfirmedByClick, setMoreConfirmedByClick] = createSignal(false);
    const moreHover = createSubmenuHover({
        onOpen: () => {
            // More opens on hover beside the logo menu button (macOS): close the
            // logo menu so the two menus never overlap.
            closeLogoMenu();
            setMoreOpen(true);
        },
        onClose: () => {
            setMoreOpen(false);
            setMoreConfirmedByClick(false);
        },
    });
    const unregisterMoreHover = parentPeers.register(MORE_PEER_KEY, moreHover);

    // Item context menu — rendered independently so the More dropdown stays open
    // while the user interacts with pin/unpin. itemMenuState drives a PopoverMenu
    // in `renderItemMenu`; null means closed.
    const [itemMenuState, setItemMenuState] = createSignal<{
        pos: { x: number; y: number };
        key: string;
        shortName: string;
    } | null>(null);

    // ── Context menu active state (keeps hover highlight while menu is open) ──
    const [contextMenuActiveKey, setContextMenuActiveKey] = createSignal<string | null>(null);


    return {
        settings,
        wmap,
        iconOnly,
        pinnedWidgets,
        moreWidgets,
        containerRef: undefined,
        moreButtonRef: undefined,
        moreDropdownRef: undefined,
        pinnedFlyoutRef: undefined,
        openParentKey,
        setOpenParentKey,
        parentPeers,
        parentHoverControllers,
        parentSlotRefs,
        parentOpenConfirmedByClick: false,
        moreOpen,
        setMoreOpen,
        moreConfirmedByClick,
        setMoreConfirmedByClick,
        moreHover,
        unregisterMoreHover,
        itemMenuState,
        setItemMenuState,
        contextMenuActiveKey,
        setContextMenuActiveKey,
        contextMenuCleanup: null,
    };
}

function closeParentFlyout(s: BarState, key: string) {
    s.parentHoverControllers.get(key)?.close();
    s.setOpenParentKey((cur) => (cur === key ? null : cur));
}

function registerParentHover(s: BarState, key: string): SubmenuHoverController {
    // Reuse an existing controller for this key rather than always
    // creating+registering a fresh one (reagent P1 on this PR):
    // getPinnedWidgets() allocates new {key, widget} object literals on
    // every fullConfigAtom change, so Solid's reference-diffing <For>
    // tears down and rebuilds EVERY pinned slot — including this one —
    // on any config change, not just when the pinned set actually
    // changes. That includes this PR's own "Pin to bar" RPC round-trip
    // for a child inside this very flyout. Disposing an open controller
    // fires its onClose and force-closes the flyout out from under the
    // user; reusing the same instance across that reference churn keeps
    // it open. See `disposeParentHovers` for
    // where these actually get disposed (component unmount, not per-row
    // teardown).
    const existing = s.parentHoverControllers.get(key);
    if (existing) return existing;
    const hover = createSubmenuHover({
        // Closing any other open peer — including the More button's own
        // controller, registered in `createBarState` under MORE_PEER_KEY — already
        // happens via parentPeers.closeOthers(key) on mouseenter/click
        // (immediately, not gated on this delayed onOpen), so this only
        // needs to update this row's own state.
        onOpen: () => s.setOpenParentKey(key),
        onClose: () => {
            s.setOpenParentKey((cur) => (cur === key ? null : cur));
            s.parentOpenConfirmedByClick = false;
        },
    });
    s.parentHoverControllers.set(key, hover);
    s.parentPeers.register(key, hover);
    return hover;
}

// Component-level teardown for every pinned-parent hover controller —
// deliberately NOT per-<For>-item (see `registerParentHover`).
function disposeParentHovers(s: BarState): void {
    for (const hover of s.parentHoverControllers.values()) hover.dispose();
    s.parentHoverControllers.clear();
}

function handleParentSlotClick(s: BarState, key: string) {
    if (s.openParentKey() === key) {
        // Already open. If that's only because hover opened it (the
        // user hasn't actually clicked it yet this session), this
        // click is the user's FIRST real interaction with the trigger
        // — confirm it stays open instead of closing, and arm the
        // toggle so a genuine second click does close it
        // (SPEC_WIDGET_BAR_HOVER_CLICK_PREMATURE_CLOSE_2026_08_20.md).
        if (!s.parentOpenConfirmedByClick) {
            s.parentOpenConfirmedByClick = true;
            return;
        }
        closeParentFlyout(s, key);
        return;
    }
    // closeOthers closes every other registered peer — every other
    // pinned parent AND the More button's own controller (MORE_PEER_KEY,
    // registered in `createBarState`) — symmetrically with how hovering/clicking More
    // closes this row.
    s.parentPeers.closeOthers(key);
    // openNow() (not a direct setOpenParentKey) so the controller's own
    // isOpen flips true — otherwise (reagent P1 on this PR) the mouse is
    // already over the row when a click happens, no later onMouseEnter
    // ever arrives to resync isOpen, and the subsequent onMouseLeave's
    // `if (!isOpen) return` skips starting the close-intent timer
    // entirely — the flyout would stay open until an unrelated close
    // path (outside click, Escape, re-click) fired.
    s.parentHoverControllers.get(key)?.openNow();
    s.parentOpenConfirmedByClick = true;
}

// A group hidden (Settings → Widgets, a file edit) or unpinned while its flyout is open loses
// its slot on the bar; close the flyout too, or it would stay up with the group's children
// still launchable from it.
function watchHiddenGroup(s: BarState): void {
    createEffect(() => {
        const key = s.openParentKey();
        if (key !== null && !s.pinnedWidgets().some((e) => e.key === key)) closeParentFlyout(s, key);
    });
}

// Close on outside click / Escape — mirrors the More dropdown's own
// outside-click effect (`watchMoreOutsidePress`), scoped to whichever parent is open.
function watchParentOutsidePress(s: BarState): void {
    createEffect(() => {
        const key = s.openParentKey();
        if (key === null) return;
        const handler = (e: MouseEvent) => {
            const t = e.target as Node;
            if (s.parentSlotRefs.get(key)?.contains(t) || s.pinnedFlyoutRef?.contains(t)) return;
            const el = t instanceof Element ? t : (t as Node).parentElement;
            if (el?.closest(".popover-menu")) return;
            closeParentFlyout(s, key);
        };
        const onKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") closeParentFlyout(s, key);
        };
        document.addEventListener("mousedown", handler, true);
        document.addEventListener("keydown", onKey);
        onCleanup(() => {
            document.removeEventListener("mousedown", handler, true);
            document.removeEventListener("keydown", onKey);
        });
    });
}

function disposeMoreHover(s: BarState): void {
    s.unregisterMoreHover();
    s.moreHover.dispose();
}

function openMore(s: BarState, _e: MouseEvent) {
    if (s.moreOpen()) {
        // Already open. If that's only because hover opened it, this
        // click is the user's FIRST real interaction with the button —
        // confirm it stays open instead of closing, and arm the toggle
        // so a genuine second click does close it
        // (SPEC_WIDGET_BAR_HOVER_CLICK_PREMATURE_CLOSE_2026_08_20.md).
        if (!s.moreConfirmedByClick()) {
            s.setMoreConfirmedByClick(true);
            return;
        }
        s.moreHover.close();
        return;
    }
    s.parentPeers.closeOthers(MORE_PEER_KEY);
    // openNow(), not a direct setMoreOpen — same click-must-sync-
    // controller-state rationale as `handleParentSlotClick` (reagent
    // P1 on this PR): without it, the next mouse-leave's `if (!isOpen)
    // return` would skip starting the close-intent timer.
    s.moreHover.openNow();
    s.setMoreConfirmedByClick(true);
}

function closeMore(s: BarState) {
    s.moreHover.close();
}

// A right-click that opens the item popover makes the browser fire a
// synthetic mouseleave on whichever dropdown/flyout sits underneath —
// the popover renders at the cursor position, and Chromium re-runs
// hit-testing against the new topmost element even though the cursor
// itself never moved. Left unguarded, that spurious leave reaches the
// hover controller's onSubmenuLeave and starts a close-intent, so More
// (or a pinned parent's flyout) would collapse the moment you right-
// click an item inside it — losing the "menu stays open while you work
// with pin/unpin" behavior the outside-click handler already protects
// on the click side (its own `.closest(".popover-menu")` check). Every
// onSubmenuLeave call site (`renderMoreDropdown`, `renderParentFlyout`) is gated on this so hover-driven close
// is suspended for as long as the item popover is open.
function isItemMenuOpen(s: BarState): boolean {
    return s.itemMenuState() !== null;
}

function handleItemContextMenu(s: BarState, pos: { x: number; y: number }, key: string) {
    s.setItemMenuState({ pos, key, shortName: pinIdOf(key) });
}

function resolveWidgetDef(s: BarState, shortName: string) {
    return s.wmap()[keyOfPinId(shortName)] ?? null;
}

/** "Hide from widget bar": off the bar, out of More and the pane "+" picker until it's
 *  shown again in Settings → Widgets. Settings itself can't be hidden. */
function hideItem(s: BarState, key: string, group: boolean) {
    return UNHIDEABLE_WIDGET_KEYS.has(key)
        ? []
        : [{ label: group ? "Hide group from widget bar" : "Hide from widget bar", icon: MENU_ICON.hide, click: () => hideFromWidgetBar(key, s.settings(), s.wmap()) }];
}

function buildItemMenuItems(s: BarState, key: string, shortName: string): PopoverMenuItem[] {
    const widgetDef = resolveWidgetDef(s, shortName);
    // A parent row's right-click is scoped to the group as a whole (§3.5)
    // — no "Open in New Window"/"Open in Floating Pane" (no single pane a
    // parent represents), and worded "group" instead of the leaf phrasing.
    // Same underlying pinWidget/unpinWidget call either way.
    if ((widgetDef?.children?.length ?? 0) > 0) {
        return [
            getPinnedKeys(s.settings(), s.wmap()).includes(shortName)
                ? { label: "Unpin group from bar", icon: MENU_ICON.unpin, click: () => { unpinWidget(shortName, s.settings(), s.wmap()); } }
                : { label: "Pin group to bar", icon: MENU_ICON.pin, click: () => { pinWidget(shortName, s.settings(), s.wmap()); } },
            ...hideItem(s, key, true),
        ];
    }
    const items: PopoverMenuItem[] = buildWidgetOpenActions(shortName, s.wmap()).map((action) => ({
        label: action.label,
        icon: action.icon,
        click: () => {
            closeMore(s);
            action.run();
        },
    }));
    // Pin/Unpin here works for a grouped child too — e.g. right-clicking
    // "Discord" inside the Messengers flyout and choosing "Pin to bar"
    // PROMOTES it out of the group onto the bar as a standalone widget;
    // "Unpin from bar" reverses that, and it goes back to living only
    // inside Messengers (see getEffectiveGroupedChildKeys). This used to
    // be a silent no-op (reagent P2 on this PR) because getPinnedKeys()
    // unconditionally stripped grouped children regardless of
    // widget:pinned — fixed at the source, so the ordinary Pin/Unpin
    // item below is correct for a child exactly as it is for any other
    // widget, no special-casing needed here.
    items.push({ type: "separator" } as PopoverMenuItem);
    items.push(
        getPinnedKeys(s.settings(), s.wmap()).includes(shortName)
            ? { label: "Unpin from bar", icon: MENU_ICON.unpin, click: () => { unpinWidget(shortName, s.settings(), s.wmap()); } }
            : { label: "Pin to bar", icon: MENU_ICON.pin, click: () => { pinWidget(shortName, s.settings(), s.wmap()); } }
    );
    items.push(...hideItem(s, key, false));
    return items;
}

// Close on outside click — ignore clicks inside button, dropdown, or any
// open popover-menu (e.g. the item context menu rendered by handleItemContextMenu).
function watchMoreOutsidePress(s: BarState): void {
    createEffect(() => {
        if (!s.moreOpen()) return;
        const handler = (e: MouseEvent) => {
            const t = e.target as Node;
            if (s.moreButtonRef?.contains(t) || s.moreDropdownRef?.contains(t)) return;
            const el = t instanceof Element ? t : (t as Node).parentElement;
            if (el?.closest(".popover-menu")) return;
            s.moreHover.close();
        };
        document.addEventListener("mousedown", handler, true);
        onCleanup(() => document.removeEventListener("mousedown", handler, true));
    });
}

function armContextMenuDismiss(s: BarState, key: string) {
    s.contextMenuCleanup?.();
    s.setContextMenuActiveKey(key);
    const clear = () => {
        s.setContextMenuActiveKey(null);
        document.removeEventListener("mousedown", clear);
        document.removeEventListener("keydown", onKey);
        s.contextMenuCleanup = null;
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") clear(); };
    document.addEventListener("mousedown", clear, { once: true });
    document.addEventListener("keydown", onKey);
    s.contextMenuCleanup = clear;
}

// ── Context menus ─────────────────────────────────────────────────────────
// Right-clicking the gaps between pinned widgets used to open its own
// one-item ("Icon Only") menu here, separate from — and hidden behind —
// the unified TitleBarContextMenu the rest of the empty tab-bar space
// opens. Removed: the bar's own onContextMenu no longer intercepts the
// event, so it now bubbles up to window-header's handler like any other
// empty-space right-click, landing on the single shared menu (which
// already has its own "Show Icons Only" entry).
function handlePinnedContextMenu(s: BarState, drag: Drag, e: MouseEvent, key: string) {
    e.preventDefault();
    e.stopPropagation();
    drag.handlePointerCancel();
    armContextMenuDismiss(s, key);
    const shortName = pinIdOf(key);
    const isParent = (s.wmap()[key]?.children?.length ?? 0) > 0;
    // A parent's right-click is scoped to group-level actions (§3.5) —
    // no "Open in New Window" (no single pane a parent represents), and
    // "Unpin group from bar" instead of the leaf "Unpin from bar" so the
    // whole-group-leaves-the-bar mental model reads differently from a
    // single pane's shortcut. Unpinning uses the same underlying
    // unpinWidget() call either way — a parent is just another key in
    // `widget:pinned`.
    if (isParent) {
        closeParentFlyout(s, key);
        openMenuAt(
            [{ label: "Unpin group from bar", icon: MENU_ICON.unpin, click: () => unpinWidget(shortName, s.settings(), s.wmap()) }, ...hideItem(s, key, true)],
            e
        );
        return;
    }
    const actions = buildWidgetOpenActions(shortName, s.wmap());
    openMenuAt(
        [
            ...actions.map((action) => ({ label: action.label, icon: action.icon, click: action.run })),
            { type: "separator" },
            { label: "Unpin from bar", icon: MENU_ICON.unpin, click: () => unpinWidget(shortName, s.settings(), s.wmap()) },
            ...hideItem(s, key, false),
        ],
        e
    );
}

function renderPinnedSlot(
    s: BarState,
    responsive: Responsive,
    drag: Drag,
    { key, widget }: WidgetEntry,
    idx: Accessor<number>,
): JSX.Element {
    const isParent = (widget.children?.length ?? 0) > 0;
    const hover = isParent ? registerParentHover(s, key) : null;
    return (
        <>
            <Show when={drag.draggingKey() != null && drag.dropIndex() === idx() && drag.draggingKey() !== key}>
                <div class="action-widget-drop-indicator" />
            </Show>
            <div
                ref={(el) => {
                    if (!isParent) return;
                    s.parentSlotRefs.set(key, el);
                    onCleanup(() => s.parentSlotRefs.delete(key));
                }}
                class={`action-widget-slot${drag.draggingKey() === key ? " dragging" : ""}${s.contextMenuActiveKey() === key ? " context-active" : ""}`}
                data-widget-slot={idx()}
                onPointerDown={(e) => drag.handlePointerDown(key, e)}
                onPointerMove={drag.handlePointerMove}
                onPointerUp={drag.handlePointerUp}
                onPointerCancel={drag.handlePointerCancel}
            >
                <ActionWidget
                    widget={widget}
                    iconOnly={!responsive.showWidgetLabels()}
                    onContextMenu={(e) => handlePinnedContextMenu(s, drag, e, key)}
                    onClick={isParent ? () => handleParentSlotClick(s, key) : undefined}
                    onMouseEnter={
                        hover
                            ? () => {
                                  s.parentPeers.closeOthers(key);
                                  hover.onTriggerEnter();
                              }
                            : undefined
                    }
                    onMouseLeave={hover ? (e) => hover.onTriggerLeave(e) : undefined}
                />
            </div>
        </>
    );
}

function renderMoreButton(s: BarState, responsive: Responsive): JSX.Element {
    return (
        <Show when={s.moreWidgets().length > 0 || responsive.clipCount() > 0}>
            <div
                ref={(el) => (s.moreButtonRef = el)}
                class="action-widget-more-btn"
                classList={{ open: s.moreOpen() }}
                onClick={(e) => openMore(s, e)}
                onMouseEnter={() => {
                    s.parentPeers.closeOthers(MORE_PEER_KEY);
                    s.moreHover.onTriggerEnter();
                }}
                onMouseLeave={(e) => s.moreHover.onTriggerLeave(e)}
            >
                <i class="fa-solid fa-ellipsis" />
                <Show when={responsive.showWidgetLabels()}>
                    <span class="action-widget-more-label">more</span>
                </Show>
                <i
                    class={`fa-solid ${s.moreOpen() ? "fa-chevron-up" : "fa-chevron-down"} action-widget-more-chevron`}
                />
            </div>
        </Show>
    );
}

function renderMeasureMirrors(s: BarState, responsive: Responsive): JSX.Element {
    return (
        <>
            {/* Mirror 1: always-labeled — measures tier 1→2 threshold. */}
            <div ref={responsive.setMirrorRef} class="action-widgets action-widgets--measure" aria-hidden="true">
                <For each={s.pinnedWidgets()}>
                    {({ widget }) => (
                        <div class="action-widget-slot">
                            <ActionWidget widget={widget} iconOnly={false} />
                        </div>
                    )}
                </For>
                <Show when={s.moreWidgets().length > 0}>
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
            <div ref={responsive.setIconMirrorRef} class="action-widgets action-widgets--measure" aria-hidden="true">
                <For each={s.pinnedWidgets()}>
                    {({ widget }) => (
                        <div class="action-widget-slot">
                            <ActionWidget widget={widget} iconOnly={true} />
                        </div>
                    )}
                </For>
                <Show when={s.moreWidgets().length > 0}>
                    <div class="action-widget-more-btn">
                        <i class="fa-solid fa-ellipsis" />
                        <i class="fa-solid fa-chevron-down action-widget-more-chevron" />
                    </div>
                </Show>
            </div>

            {/* More button width probe — always mounted so moreBtnW is available
                before the live More button mounts on first tier-3 entry. */}
            <div class="action-widgets action-widgets--measure" aria-hidden="true">
                <div ref={responsive.setIconMirrorMoreRef} class="action-widget-more-btn">
                    <i class="fa-solid fa-ellipsis" />
                    <i class="fa-solid fa-chevron-down action-widget-more-chevron" />
                </div>
            </div>
        </>
    );
}

function renderMoreDropdown(s: BarState, responsive: Responsive): JSX.Element {
    return (
        <Show when={s.moreOpen()}>
            <MoreDropdown
                widgets={() => [...responsive.clippedPinnedWidgets(), ...s.moreWidgets()]}
                onClose={() => closeMore(s)}
                onItemContextMenu={(pos, key) => handleItemContextMenu(s, pos, key)}
                anchor={() => s.moreButtonRef ?? null}
                settings={s.settings}
                wmap={s.wmap}
                ref={(el) => (s.moreDropdownRef = el)}
                onSubmenuEnter={() => s.moreHover.onSubmenuEnter()}
                onSubmenuLeave={(e) => {
                    if (isItemMenuOpen(s)) return;
                    s.moreHover.onSubmenuLeave(e);
                }}
                setSubmenuEl={(el) => s.moreHover.setSubmenuEl(el)}
                isItemMenuOpen={() => isItemMenuOpen(s)}
            />
        </Show>
    );
}

function renderParentFlyout(s: BarState): JSX.Element {
    return (
        <Show when={s.openParentKey()}>
            {(keyAccessor) => {
                // Capture a plain string once instead of re-invoking
                // the Show's accessor from these callbacks — several
                // of them (onClose, and PinnedWidgetFlyout's own
                // onCleanup calling setSubmenuEl(null)) can still fire
                // during/after this <Show> branch unmounts, and
                // calling a stale keyed-Show accessor post-unmount
                // throws ("Attempting to access a stale value from
                // <Show>") — reproduced live via muxlog while testing
                // this PR's own click-to-close path.
                const key = keyAccessor();
                return (
                    <PinnedWidgetFlyout
                        widget={s.wmap()[key]}
                        wmap={s.wmap}
                        settings={s.settings}
                        onClose={() => closeParentFlyout(s, key)}
                        onItemContextMenu={(pos, key) => handleItemContextMenu(s, pos, key)}
                        anchor={() => s.parentSlotRefs.get(key) ?? null}
                        onSubmenuEnter={() => s.parentHoverControllers.get(key)?.onSubmenuEnter()}
                        onSubmenuLeave={(e) => {
                            if (isItemMenuOpen(s)) return;
                            s.parentHoverControllers.get(key)?.onSubmenuLeave(e);
                        }}
                        setSubmenuEl={(el) => s.parentHoverControllers.get(key)?.setSubmenuEl(el)}
                        ref={(el) => (s.pinnedFlyoutRef = el)}
                    />
                );
            }}
        </Show>
    );
}

function renderItemMenu(s: BarState): JSX.Element {
    return (
        <Show when={s.itemMenuState() !== null}>
            <PopoverMenu
                pos={s.itemMenuState()!.pos}
                onClose={() => s.setItemMenuState(null)}
                items={buildItemMenuItems(s, s.itemMenuState()!.key, s.itemMenuState()!.shortName)}
            />
        </Show>
    );
}

// ── Main ActionWidgets ────────────────────────────────────────────────────────

const ActionWidgets = (): JSX.Element => {
    const s = createBarState();

    // ── Responsive labels — 3-tier collapse ────────────────────────────────
    const responsive = useWidgetBarResponsive({
        containerRef: () => s.containerRef,
        moreButtonRef: () => s.moreButtonRef,
        pinnedWidgets: s.pinnedWidgets,
        moreWidgets: s.moreWidgets,
        iconOnly: s.iconOnly,
    });

    // ── Drag-to-reorder (pinned only, saves to widget:pinned) ──────────────
    const drag = useWidgetDragReorder({
        containerRef: () => s.containerRef,
        pinnedWidgets: s.pinnedWidgets,
        savedPins: () => getSavedPinNames(s.settings(), s.wmap()),
    });

    onCleanup(() => disposeParentHovers(s));
    watchHiddenGroup(s);
    watchParentOutsidePress(s);
    onCleanup(() => disposeMoreHover(s));
    watchMoreOutsidePress(s);

    // ── Render ────────────────────────────────────────────────────────────────

    return (
        <>
            <div
                ref={(el) => (s.containerRef = el)}
                class="action-widgets"
                data-testid="action-widgets"
                data-drag-region="false"
            >
                <For each={responsive.visiblePinnedWidgets()}>
                    {(entry, idx) => renderPinnedSlot(s, responsive, drag, entry, idx)}
                </For>
                <Show when={drag.draggingKey() != null && drag.dropIndex() === responsive.visiblePinnedWidgets().length}>
                    <div class="action-widget-drop-indicator" />
                </Show>

                {renderMoreButton(s, responsive)}
            </div>

            {renderMeasureMirrors(s, responsive)}

            {renderMoreDropdown(s, responsive)}

            {renderParentFlyout(s)}

            {renderItemMenu(s)}
        </>
    );
};

ActionWidgets.displayName = "ActionWidgets";

export { ActionWidgets };
