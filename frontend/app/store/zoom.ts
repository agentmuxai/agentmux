// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

// Per-pane zoom — modifies the focused block's term:zoom metadata.
// Replaces the old global window zoom (Tauri window.set_zoom).

import { getBlockComponentModel, getFocusedBlockId, globalStore, WOS } from "@/app/store/global";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { fireAndForget } from "@/util/util";
import { atom } from "jotai";

// Zoom constants
export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 2.0;
export const DEFAULT_ZOOM = 1.0;
export const KEYBOARD_STEP = 0.25; // 25% increments for keyboard
export const WHEEL_STEP = 0.1; // 10% increments for scroll wheel

// Zoom indicator visibility (auto-hide after 1.5s)
export const zoomIndicatorVisibleAtom = atom<boolean>(false);
export const zoomIndicatorTextAtom = atom<string>("");
let zoomIndicatorTimeout: NodeJS.Timeout | null = null;

function clampZoom(factor: number): number {
    return Math.min(Math.max(factor, MIN_ZOOM), MAX_ZOOM);
}

function roundZoom(factor: number): number {
    return Math.round(factor * 20) / 20; // Round to 0.05 increments
}

/**
 * Get the current term:zoom for the focused block, or null if not a terminal.
 */
function getFocusedBlockZoom(): { blockId: string; zoom: number } | null {
    const blockId = getFocusedBlockId();
    if (!blockId) return null;

    const bcm = getBlockComponentModel(blockId);
    if (!bcm?.viewModel) return null;

    // Only terminal views support per-pane zoom
    if (bcm.viewModel.viewType !== "term") return null;

    const blockOref = WOS.makeORef("block", blockId);
    const blockData = WOS.getObjectValue<Block>(blockOref);
    const currentZoom = blockData?.meta?.["term:zoom"] ?? 1.0;
    return { blockId, zoom: currentZoom };
}

/**
 * Set zoom on the focused terminal pane.
 */
function setPaneZoom(factor: number): void {
    const info = getFocusedBlockZoom();
    if (!info) return;

    const newZoom = clampZoom(roundZoom(factor));
    const metaValue = Math.abs(newZoom - 1.0) < 0.01 ? null : newZoom;

    fireAndForget(() =>
        RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", info.blockId),
            meta: { "term:zoom": metaValue },
        })
    );

    showZoomIndicator(`${Math.round(newZoom * 100)}%`);
}

/**
 * Zoom in the focused terminal pane.
 */
export function zoomIn(store: any, step: number = KEYBOARD_STEP): void {
    const info = getFocusedBlockZoom();
    if (!info) return;
    setPaneZoom(info.zoom + step);
}

/**
 * Zoom out the focused terminal pane.
 */
export function zoomOut(store: any, step: number = KEYBOARD_STEP): void {
    const info = getFocusedBlockZoom();
    if (!info) return;
    setPaneZoom(info.zoom - step);
}

/**
 * Reset zoom on the focused terminal pane to 100%.
 */
export function zoomReset(store: any): void {
    setPaneZoom(DEFAULT_ZOOM);
}

/**
 * Show zoom indicator with auto-hide.
 */
function showZoomIndicator(text: string): void {
    if (zoomIndicatorTimeout) {
        clearTimeout(zoomIndicatorTimeout);
    }
    globalStore.set(zoomIndicatorTextAtom, text);
    globalStore.set(zoomIndicatorVisibleAtom, true);

    zoomIndicatorTimeout = setTimeout(() => {
        globalStore.set(zoomIndicatorVisibleAtom, false);
        zoomIndicatorTimeout = null;
    }, 1500);
}

/**
 * Get zoom as percentage string for the focused pane.
 */
export function getZoomPercentage(store: any): string {
    const info = getFocusedBlockZoom();
    if (!info) return "100%";
    return `${Math.round(info.zoom * 100)}%`;
}

/**
 * Load zoom level from settings on startup.
 * Now a no-op — per-pane zoom is stored in block metadata, not global state.
 */
export async function loadZoom(store: any): Promise<void> {
    // Per-pane zoom is loaded from block metadata automatically via Jotai atoms.
    // No global zoom to restore.
}
