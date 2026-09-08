// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Zoom module — per-pane zoom (the focused block's `term:zoom` metadata)
// and chrome zoom (title bar + status bar, via the `--zoomfactor` CSS var).
//
// This used to be three files — `zoom.win32.ts`, `zoom.linux.ts`,
// `zoom.darwin.ts` — selected by the `.platform` import resolver. Measured
// on 2026-09-06 they differed by **nine lines, every one a comment**: the
// JavaScript was identical on all three platforms
// (docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md §2.3). The
// per-platform knowledge those comments carried is real, though, so it is
// kept here rather than lost:
//
// The one platform-sensitive concern is *width compensation* when chrome
// zoom scales the header. JS sets ONLY `--zoomfactor`; how the header's
// width is compensated is a CSS decision that differs per platform, and it
// must stay in CSS — do NOT set `--chrome-header-width` (or any width) from
// JS on any platform:
//
//   - Windows: `calc(100vw / var(--zoomfactor, 1))` in window-header.scss.
//     Setting the width from JS breaks Windows.
//   - macOS:   `width: 100%` in window-header.darwin.scss. Do NOT switch
//     it to the `calc(100vw / …)` form — that double-divides on WebKit and
//     the window buttons drift left on zoom.
//   - Linux:   WebKitGTK does NOT divide flex space by zoom, so no
//     compensation is needed at all; the CSS uses a plain `100vw`.
//
// If a genuine JS-level platform difference ever appears, the right shape is
// a single branch on the runtime platform inside `applyChromeZoomCSS` — not
// three copies of this file.

import { getAllBlockComponentModelEntries } from "@/app/store/block-component-registry";
import { getBlockComponentModel, getFocusedBlockId, WOS } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { fireAndForget } from "@/util/util";
import { createSignal } from "solid-js";

// Zoom constants
export const MIN_ZOOM = 0.5;
export const MAX_ZOOM = 2.0;
export const DEFAULT_ZOOM = 1.0;
export const KEYBOARD_STEP = 0.1; // 10% increments for keyboard
export const WHEEL_STEP = 0.05; // 5% increments for scroll wheel
const MICRO_STEP = 0.01; // fine step for skip-to-next-size logic

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
    return Math.round(factor * 100) / 100; // Round to 0.01 increments
}

// ── Per-pane zoom (terminal blocks) ───────────────────────────────

function getBaseFontSize(blockId: string): number {
    const blockOref = WOS.makeORef("block", blockId);
    const blockData = WOS.getObjectValue<Block>(blockOref);
    const metaFontSize = blockData?.meta?.["term:fontsize"];
    if (typeof metaFontSize === "number" && !isNaN(metaFontSize) && metaFontSize >= 4 && metaFontSize <= 64) {
        return metaFontSize;
    }
    const bcm = getBlockComponentModel(blockId);
    if (bcm?.viewModel?.viewType === "editor") return 13;
    return 15;
}

function computeEffectiveFontSize(baseFontSize: number, zoom: number): number {
    return Math.max(4, Math.min(64, Math.round(baseFontSize * zoom)));
}

function getBlockZoom(blockId: string): number | null {
    const bcm = getBlockComponentModel(blockId);
    if (!bcm?.viewModel) return null;
    const vt = bcm.viewModel.viewType;
    if (vt !== "term" && vt !== "agent" && vt !== "swarm" && vt !== "editor" && vt !== "armory") return null;

    const blockOref = WOS.makeORef("block", blockId);
    const blockData = WOS.getObjectValue<Block>(blockOref);
    return blockData?.meta?.["term:zoom"] ?? 1.0;
}

function setBlockZoom(blockId: string, factor: number, showIndicator: boolean = true): void {
    const newZoom = clampZoom(roundZoom(factor));
    const metaValue = Math.abs(newZoom - 1.0) < 0.01 ? null : newZoom;

    fireAndForget(() =>
        RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", blockId),
            meta: { "term:zoom": metaValue },
        })
    );

    // Suppressed by the all-panes stepper below, which shows one summary
    // toast for the whole gesture instead of one per pane.
    if (showIndicator) {
        showZoomIndicator(`${Math.round(newZoom * 100)}%`);
    }
}

function stepZoom(
    blockId: string,
    zoom: number,
    step: number,
    direction: 1 | -1,
    showIndicator: boolean = true
): void {
    const baseFontSize = getBaseFontSize(blockId);
    const currentFontSize = computeEffectiveFontSize(baseFontSize, zoom);
    let newZoom = zoom + step * direction;
    const limit = direction === 1 ? MAX_ZOOM : MIN_ZOOM;
    while (
        computeEffectiveFontSize(baseFontSize, newZoom) === currentFontSize &&
        (direction === 1 ? newZoom < limit : newZoom > limit)
    ) {
        newZoom += MICRO_STEP * direction;
    }
    setBlockZoom(blockId, newZoom, showIndicator);
}

export function zoomBlockIn(blockId: string, step: number = WHEEL_STEP): void {
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return;
    stepZoom(blockId, zoom, step, 1);
}

export function zoomBlockOut(blockId: string, step: number = WHEEL_STEP): void {
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return;
    stepZoom(blockId, zoom, step, -1);
}

// ── All-panes zoom (Ctrl+Shift+Scroll) ──────────────────────────────
//
// See docs/specs/SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md.
// Steps every pane in the CURRENT WINDOW's block registry, each relative
// to its own current zoom level — this is a batch of independent
// single-pane zooms, not a new shared value. A pane whose viewType
// getBlockZoom() doesn't recognize (browser, warden, sysinfo, help, ...)
// is silently skipped, the same way a single Ctrl+Scroll over one of
// them already is today — no new filtering logic, this just reuses that
// existing guard for every block in the window instead of one.

function stepAllPanes(step: number, direction: 1 | -1): void {
    let minZoom = Infinity;
    let maxZoom = -Infinity;

    for (const [blockId] of getAllBlockComponentModelEntries()) {
        const zoom = getBlockZoom(blockId);
        if (zoom == null) continue; // not a zoomable pane type — leave untouched

        // Suppress the per-pane indicator: N panes stepping in one gesture
        // would otherwise fire N toasts, each overwriting the last before
        // the user can read it. One summary toast is shown below instead.
        stepZoom(blockId, zoom, step, direction, false);

        const newZoom = getBlockZoom(blockId) ?? DEFAULT_ZOOM;
        minZoom = Math.min(minZoom, newZoom);
        maxZoom = Math.max(maxZoom, newZoom);
    }

    if (minZoom === Infinity) return; // no zoomable pane in this window

    const label =
        Math.abs(minZoom - maxZoom) < 0.01
            ? `All panes: ${Math.round(minZoom * 100)}%`
            : `All panes: ${Math.round(minZoom * 100)}%–${Math.round(maxZoom * 100)}%`;
    showZoomIndicator(label);
}

export function zoomAllPanesIn(step: number = WHEEL_STEP): void {
    stepAllPanes(step, 1);
}

export function zoomAllPanesOut(step: number = WHEEL_STEP): void {
    stepAllPanes(step, -1);
}

export function zoomIn(step: number = KEYBOARD_STEP): void {
    const blockId = getFocusedBlockId();
    if (!blockId) return;
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return;
    stepZoom(blockId, zoom, step, 1);
}

export function zoomOut(step: number = KEYBOARD_STEP): void {
    const blockId = getFocusedBlockId();
    if (!blockId) return;
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return;
    stepZoom(blockId, zoom, step, -1);
}

export function zoomReset(): void {
    const blockId = getFocusedBlockId();
    if (!blockId) return;
    setBlockZoom(blockId, DEFAULT_ZOOM);
}

// ── Chrome zoom (title bar + status bar) ──────────────────────────

function applyChromeZoomCSS(factor: number): void {
    // Only set --zoomfactor. Width compensation is pure CSS on every
    // platform — see the header comment for the per-platform rule.
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

export function showZoomIndicator(text: string): void {
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

export function getZoomPercentage(): string {
    const blockId = getFocusedBlockId();
    if (!blockId) return "100%";
    const zoom = getBlockZoom(blockId);
    if (zoom == null) return "100%";
    return `${Math.round(zoom * 100)}%`;
}

export async function loadZoom(store: any): Promise<void> {}
