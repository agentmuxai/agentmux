// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0
//
// Windows-specific zoom module.
// Chrome zoom sets only --zoomfactor. Width compensation is handled
// purely in CSS: calc(100vw / var(--zoomfactor, 1)) in window-header.scss.
// Do NOT set --chrome-header-width from JS — it breaks Windows.

// Per-pane zoom — modifies the focused block's term:zoom metadata.
// Chrome zoom — scales title bar + status bar together via --zoomfactor CSS var.

import { getBlockComponentModel, getFocusedBlockId, WOS } from "@/app/store/global";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { fireAndForget } from "@/util/util";
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

function getBlockZoom(blockId: string): number | null {
    const bcm = getBlockComponentModel(blockId);
    if (!bcm?.viewModel) return null;
    const vt = bcm.viewModel.viewType;
    if (vt !== "term" && vt !== "agent") return null;

    const blockOref = WOS.makeORef("block", blockId);
    const blockData = WOS.getObjectValue<Block>(blockOref);
    return blockData?.meta?.["term:zoom"] ?? 1.0;
}

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

export function zoomBlockIn(blockId: string, step: number = WHEEL_STEP): void {
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return;
    setBlockZoom(blockId, zoom + step);
}

export function zoomBlockOut(blockId: string, step: number = WHEEL_STEP): void {
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return;
    setBlockZoom(blockId, zoom - step);
}

export function zoomIn(store: any, step: number = KEYBOARD_STEP): void {
    const blockId = getFocusedBlockId();
    if (!blockId) return;
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return;
    setBlockZoom(blockId, zoom + step);
}

export function zoomOut(store: any, step: number = KEYBOARD_STEP): void {
    const blockId = getFocusedBlockId();
    if (!blockId) return;
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return;
    setBlockZoom(blockId, zoom - step);
}

export function zoomReset(store: any): void {
    const blockId = getFocusedBlockId();
    if (!blockId) return;
    setBlockZoom(blockId, DEFAULT_ZOOM);
}

// ── Chrome zoom (title bar + status bar) ──────────────────────────

function applyChromeZoomCSS(factor: number): void {
    // Windows: only set --zoomfactor. Width compensation is pure CSS.
    document.documentElement.style.setProperty("--zoomfactor", String(factor));
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

export function initChromeZoom(): void {
    applyChromeZoomCSS(DEFAULT_ZOOM);
}

// ── Shared helpers ────────────────────────────────────────────────

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

export function getZoomPercentage(store: any): string {
    const blockId = getFocusedBlockId();
    if (!blockId) return "100%";
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return "100%";
    return `${Math.round(zoom * 100)}%`;
}

export async function loadZoom(store: any): Promise<void> {}
