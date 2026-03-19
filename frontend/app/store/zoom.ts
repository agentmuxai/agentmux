// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

// Per-pane zoom — modifies the focused block's term:zoom metadata.
// Chrome zoom — scales title bar + status bar together via --zoomfactor CSS var.

import { getBlockComponentModel, getFocusedBlockId, WOS } from "@/app/store/global";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { fireAndForget } from "@/util/util";
import { PLATFORM, PlatformLinux, PlatformMacOS } from "@/util/platformutil";
import { createSignal } from "solid-js";

// Zoom constants
export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 2.0;
export const DEFAULT_ZOOM = 1.0;
export const KEYBOARD_STEP = 0.25; // 25% increments for keyboard
export const WHEEL_STEP = 0.1; // 10% increments for scroll wheel

// Zoom indicator visibility (auto-hide after 1.5s)
export const [zoomIndicatorVisibleAtom, setZoomIndicatorVisible] = createSignal<boolean>(false);
export const [zoomIndicatorTextAtom, setZoomIndicatorText] = createSignal<string>("");
let zoomIndicatorTimeout: NodeJS.Timeout | null = null;

// Chrome zoom (title bar + status bar)
export const [chromeZoomAtom, setChromeZoomSignal] = createSignal<number>(DEFAULT_ZOOM);

function clampZoom(factor: number): number {
    return Math.min(Math.max(factor, MIN_ZOOM), MAX_ZOOM);
}

function roundZoom(factor: number): number {
    return Math.round(factor * 20) / 20; // Round to 0.05 increments
}

// ── Per-pane zoom (terminal blocks) ───────────────────────────────

/**
 * Get the current term:zoom for a specific block, or null if not a terminal.
 */
function getBlockZoom(blockId: string): number | null {
    const bcm = getBlockComponentModel(blockId);
    if (!bcm?.viewModel) return null;
    const vt = bcm.viewModel.viewType;
    if (vt !== "term" && vt !== "agent") return null;

    const blockOref = WOS.makeORef("block", blockId);
    const blockData = WOS.getObjectValue<Block>(blockOref);
    return blockData?.meta?.["term:zoom"] ?? 1.0;
}

/**
 * Set zoom on a specific terminal pane.
 */
function setBlockZoom(blockId: string, factor: number): void {
    const newZoom = clampZoom(roundZoom(factor));
    const metaValue = Math.abs(newZoom - 1.0) < 0.01 ? null : newZoom;

    fireAndForget(() =>
        RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", blockId),
            meta: { "term:zoom": metaValue },
        })
    );

    showZoomIndicator(`${Math.round(newZoom * 100)}%`);
}

/**
 * Zoom in a specific block by blockId (for scroll wheel on hovered pane).
 */
export function zoomBlockIn(blockId: string, step: number = WHEEL_STEP): void {
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return;
    setBlockZoom(blockId, zoom + step);
}

/**
 * Zoom out a specific block by blockId (for scroll wheel on hovered pane).
 */
export function zoomBlockOut(blockId: string, step: number = WHEEL_STEP): void {
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return;
    setBlockZoom(blockId, zoom - step);
}

/**
 * Zoom in the focused terminal pane (keyboard shortcut).
 */
export function zoomIn(store: any, step: number = KEYBOARD_STEP): void {
    const blockId = getFocusedBlockId();
    if (!blockId) return;
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return;
    setBlockZoom(blockId, zoom + step);
}

/**
 * Zoom out the focused terminal pane (keyboard shortcut).
 */
export function zoomOut(store: any, step: number = KEYBOARD_STEP): void {
    const blockId = getFocusedBlockId();
    if (!blockId) return;
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return;
    setBlockZoom(blockId, zoom - step);
}

/**
 * Reset zoom on the focused terminal pane to 100%.
 */
export function zoomReset(store: any): void {
    const blockId = getFocusedBlockId();
    if (!blockId) return;
    setBlockZoom(blockId, DEFAULT_ZOOM);
}

// ── Chrome zoom (title bar + status bar) ──────────────────────────

function applyChromeZoomCSS(factor: number): void {
    document.documentElement.style.setProperty("--zoomfactor", String(factor));
    // Platform-specific header width compensation:
    // - Linux: 100vw (WebKitGTK doesn't divide flex space by zoom)
    // - macOS: 100% (avoids sub-pixel rounding with viewport units under zoom)
    // - Windows: NOT set — uses CSS calc(100vw / var(--zoomfactor, 1)) which is
    //   evaluated live by the browser in the zoom context. Setting a JS literal
    //   like calc(100vw / ${factor}) is NOT equivalent and breaks Windows.
    if (PLATFORM === PlatformLinux || factor <= 1) {
        document.documentElement.style.setProperty("--chrome-header-width", "100vw");
    } else if (PLATFORM === PlatformMacOS) {
        document.documentElement.style.setProperty("--chrome-header-width", "100%");
    } else {
        // Windows: remove the JS-set property so CSS fallback kicks in.
        document.documentElement.style.removeProperty("--chrome-header-width");
    }
}

export function chromeZoomIn(step: number = WHEEL_STEP): void {
    setChromeZoom(chromeZoomAtom() + step);
}

export function chromeZoomOut(step: number = WHEEL_STEP): void {
    setChromeZoom(chromeZoomAtom() - step);
}

export function chromeZoomReset(): void {
    setChromeZoom(DEFAULT_ZOOM);
}

function setChromeZoom(factor: number): void {
    const clamped = clampZoom(roundZoom(factor));
    setChromeZoomSignal(clamped);
    applyChromeZoomCSS(clamped);
    showZoomIndicator(`Chrome ${Math.round(clamped * 100)}%`);
}

/**
 * Initialize chrome zoom on startup. Resets Tauri window zoom to 1.0
 * and applies the default chrome zoom CSS.
 */
export function initChromeZoom(): void {
    applyChromeZoomCSS(DEFAULT_ZOOM);
}

// ── Shared helpers ────────────────────────────────────────────────

/**
 * Show zoom indicator with auto-hide.
 */
function showZoomIndicator(text: string): void {
    if (zoomIndicatorTimeout) {
        clearTimeout(zoomIndicatorTimeout);
    }
    setZoomIndicatorText(text);
    setZoomIndicatorVisible(true);

    zoomIndicatorTimeout = setTimeout(() => {
        setZoomIndicatorVisible(false);
        zoomIndicatorTimeout = null;
    }, 1500);
}

/**
 * Get zoom as percentage string for the focused pane.
 */
export function getZoomPercentage(store: any): string {
    const blockId = getFocusedBlockId();
    if (!blockId) return "100%";
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return "100%";
    return `${Math.round(zoom * 100)}%`;
}

/**
 * No-op — per-pane zoom is stored in block metadata, chrome zoom uses CSS vars.
 */
export async function loadZoom(store: any): Promise<void> {}
