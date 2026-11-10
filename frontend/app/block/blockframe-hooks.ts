// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What a pane's frame keeps wired up for its block: the connection, and the right-click menu on its body. */

import { createEffect, onCleanup, onMount, type Accessor } from "solid-js";
import { openMenuAt } from "@/app/element/ui";
import { useWidthFit } from "@/app/element/width-fit";
import { createScrollEdges } from "@/app/element/scroll-edges";
import { stripWheelDelta } from "@/app/element/strip-wheel";
import { getBlockComponentModel } from "@/app/store/block-component-registry";
import { chromeZoomAtom } from "@/app/store/chrome-zoom";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getApi } from "@/util/app-api";
import * as util from "@/util/util";
import type { NodeModel } from "@/layout/index";
import { resolveContextMenuRegion } from "./context-menu-region";
import { buildPaneContextMenu, joinMenuGroups, type PaneMenuSection } from "./pane-actions";

/** The menu's "Minimize to Header" for an expanded pane that can minimize, else none. */
export function minimizeToHeader(nodeModel: NodeModel): (() => void) | undefined {
    if (nodeModel.isEphemeral() || nodeModel.isMinimized() || !(nodeModel.canMinimize?.() ?? true) || nodeModel.numLeafs() <= 1) {
        return undefined;
    }
    return () => nodeModel.toggleMinimize({ chip: true });
}

/**
 * A narrow pane's header sheds its tab strip's last sliver, then its view's extra end buttons, then
 * its window controls (the grip's menu has them), so Close stays in the pane: `data-fit` "tabs",
 * "trim", then "min", which mixins.scss's header layout turns into what is hidden. Fitted again when
 * the header or its end controls change size, and when the chrome zoom does.
 */
export function useHeaderFit(getHeader: () => HTMLElement | undefined): void {
    useWidthFit(getHeader, ["tabs", "trim", "min"], (el) => [...el.querySelectorAll(".block-frame-end-icons")], chromeZoomAtom);
}

/**
 * Keeps the connection of a pane whose view lets the frame manage it: the block's component model
 * can open the switch-connection modal while the frame stands, and the connection is ensured once
 * the block has one (never for a drag preview).
 */
export function useBlockConnection(opts: {
    nodeModel: NodeModel;
    manageConnection: boolean | null | undefined;
    changeConnModalAtom: util.SignalAtom<boolean>;
    blockData: Accessor<Block | undefined>;
    preview: boolean | undefined;
}): void {
    const { nodeModel, manageConnection, changeConnModalAtom, blockData } = opts;
    createEffect(() => {
        if (!manageConnection) {
            return;
        }
        const bcm = getBlockComponentModel(nodeModel.blockId);
        if (bcm != null) {
            bcm.openSwitchConnection = () => {
                changeConnModalAtom._set(true);
            };
        }
        onCleanup(() => {
            const bcm = getBlockComponentModel(nodeModel.blockId);
            if (bcm != null) {
                bcm.openSwitchConnection = null;
            }
        });
    });

    createEffect(() => {
        // on mount, if manageConnection, call ConnEnsure
        if (!manageConnection || blockData() == null || opts.preview) {
            return;
        }
        const connName = blockData()?.meta?.connection;
        if (!util.isBlank(connName)) {
            console.log("ensure conn", nodeModel.blockId, connName);
            RpcApi.ConnEnsureCommand(
                TabRpcClient,
                { connname: connName, logblockid: nodeModel.blockId },
                { timeout: 60000 }
            ).catch((e) => {
                console.log("error ensuring connection", nodeModel.blockId, connName, e);
            });
        }
    });
}

type BrowserMenuContext = Parameters<NonNullable<ViewModel["getBodyContextMenuItems"]>>[0];

/**
 * The body's right-click handler, and, for a browser pane, the host's own request for it.
 * `browserCtx` is only ever passed by the synthetic browser-pane path (real DOM contextmenu events
 * never carry it) — see getBodyContextMenuItems's doc comment in custom.d.ts.
 */
export function useBodyContextMenu(opts: {
    nodeModel: NodeModel;
    viewModel: ViewModel | null | undefined;
    blockData: Accessor<Block | undefined>;
    preview: boolean | undefined;
    frameEl: Accessor<HTMLDivElement | undefined>;
}): (e: MouseEvent, browserCtx?: BrowserMenuContext) => void {
    const { nodeModel, viewModel, blockData, frameEl } = opts;
    const onBodyContextMenu = (e: MouseEvent, browserCtx?: BrowserMenuContext) => {
        if (!blockData() || opts.preview) return;
        e.preventDefault();
        e.stopPropagation();
        // A registered region (context-menu-region.ts) under the click — e.g.
        // the agent shell drawer — can add its own items and drop pane
        // sections that make no sense there. No region (every other click,
        // and the synthetic browser path, which has no DOM target): unchanged.
        const region = resolveContextMenuRegion(e.target, frameEl());
        const omit = new Set<PaneMenuSection>(region?.omit);
        const menu = joinMenuGroups([
            region?.items?.() ?? [],
            omit.has("viewItems") ? [] : (viewModel?.getBodyContextMenuItems?.(browserCtx) ?? []),
            buildPaneContextMenu(
                blockData(),
                {
                    magnified: nodeModel.isMagnified(),
                    onMagnifyToggle: nodeModel.toggleMagnify,
                    onClose: nodeModel.onClose,
                    onMinimizeToHeader: minimizeToHeader(nodeModel),
                    inspectAt: { x: e.clientX, y: e.clientY },
                    omit,
                },
                viewModel
            ),
        ]);
        if (menu.length === 0) return;
        openMenuAt(menu, e);
    };

    // Browser panes: the page is a native overlay, so a right-click on it never fires a DOM
    // `contextmenu`. The host emits `browser-pane-context-menu` with the point in the pane's own
    // coordinates; it is translated to window coordinates and drives the same handler through a
    // synthetic event, so the menu is composed exactly as for every other pane
    // (docs/specs/SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md).
    onMount(() => {
        const unsubPromise = getApi().listen<{
            block_id: string;
            x: number;
            y: number;
            link_url: string;
            image_url?: string;
            selection_text: string;
            is_editable: boolean;
            can_go_back: boolean;
            can_go_forward: boolean;
        }>("browser-pane-context-menu", (payload) => {
            if (payload.block_id !== nodeModel.blockId) return;
            // CEF's xcoord/ycoord are relative to the browser render view,
            // which browser-view.tsx positions against the INNER
            // `.browser-placeholder` div — not `frameEl` (the outer wrapper,
            // which also contains the header/title bar/nav bar above the
            // content). Using frameEl's rect offset the menu downward by
            // that header chrome's height.
            const rect = frameEl()?.querySelector<HTMLElement>(".browser-placeholder")?.getBoundingClientRect();
            if (!rect) return;
            const synthetic = {
                clientX: rect.left + payload.x,
                clientY: rect.top + payload.y,
                preventDefault: () => {},
                stopPropagation: () => {},
            } as unknown as MouseEvent;
            onBodyContextMenu(synthetic, {
                x: payload.x,
                y: payload.y,
                linkUrl: payload.link_url || undefined,
                imageUrl: payload.image_url || undefined,
                selectionText: payload.selection_text || undefined,
                isEditable: payload.is_editable,
                canGoBack: payload.can_go_back,
                canGoForward: payload.can_go_forward,
            });
        });
        onCleanup(() => {
            void unsubPromise.then((unsub) => unsub());
        });
    });
    return onBodyContextMenu;
}

/** The header's one horizontal scroll range (#29325): its element, its edge fades, and the wheel
 *  that scrolls it sideways over the whole header row. A horizontal gesture is the range's own
 *  (its overflow is native); a vertical one over the row must not scroll the page under it.
 *  Called once, in the header's own component. */
export function useHeaderScrollRange(header: () => HTMLElement | undefined): {
    setRange: (el: HTMLDivElement) => void;
    moreLeft: Accessor<boolean>;
    moreRight: Accessor<boolean>;
} {
    let range: HTMLDivElement | undefined;
    const edges = createScrollEdges(() => range);
    onMount(() => {
        const el = header();
        if (!el) return;
        // A real listener, not Solid's onWheel: the delegated one is passive, so preventDefault() would be a no-op.
        const onWheel = (e: WheelEvent) => {
            if (!range || range.scrollWidth <= range.clientWidth) return;
            if (Math.abs(e.deltaX) >= Math.abs(e.deltaY)) return;
            e.preventDefault();
            range.scrollLeft += stripWheelDelta(e.deltaY, e.deltaMode);
        };
        el.addEventListener("wheel", onWheel, { passive: false });
        onCleanup(() => el.removeEventListener("wheel", onWheel));
    });
    // The strip no longer scrolls itself inside the range, so its own reveal cannot bring a
    // newly selected pill into view: do it here, and only when the SELECTED pill changes — a
    // pill's attention, drag or drop class, or a badge mounting beside it, must never pull a
    // range the person scrolled away from back to the tabs.
    onMount(() => {
        let frame = 0;
        let shown: HTMLElement | null = null;
        const revealIfSelectionChanged = () => {
            const active = range?.querySelector<HTMLElement>(".pane-tab--active, .pane-tab--selected");
            if (!active || active === shown) return;
            shown = active;
            active.scrollIntoView({ block: "nearest", inline: "nearest" });
        };
        const schedule = () => {
            if (frame) return;
            frame = requestAnimationFrame(() => {
                frame = 0;
                revealIfSelectionChanged();
            });
        };
        const observer = new MutationObserver(schedule);
        if (range) observer.observe(range, { subtree: true, childList: true, attributes: true, attributeFilter: ["class"] });
        onCleanup(() => {
            observer.disconnect();
            if (frame) cancelAnimationFrame(frame);
        });
    });
    return {
        setRange: (el) => (range = el),
        moreLeft: edges.moreLeft,
        moreRight: edges.moreRight,
    };
}
