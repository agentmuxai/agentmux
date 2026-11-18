// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Zoom module — per-pane zoom (the focused block's `term:zoom` metadata)
// and chrome zoom (title bar + status bar, via the `--zoomfactor` CSS var).
//
// This used to be three files — `zoom.win32`, `zoom.linux`,
// `zoom.darwin` — selected by the `.platform` import resolver. Measured
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
//   - Windows and Linux (both Chromium: CEF/WebView2): `calc(100vw /
//     var(--zoomfactor))` in window-header.{win32,linux}.scss. Setting the
//     width from JS breaks Windows.
//   - macOS:   `width: 100%` in window-header.darwin.scss. Do NOT switch
//     it to the `calc(100vw / …)` form — that double-divides on WebKit and
//     the window buttons drift left on zoom.
//
// If a genuine JS-level platform difference ever appears, the right shape is
// a single branch on the runtime platform inside `applyChromeZoomCSS` — not
// three copies of this file.

import { paneTabCapability } from "@/app/store/pane-tab-registry";
import { getAllBlockComponentModelEntries, getBlockComponentModel, getFocusedBlockId } from "@/app/store/block-component-registry";
import { MOS } from "@/app/store/global";
import { fireAndForget } from "@/util/util";
import { isModKey } from "@/util/platformutil";
import { batch, createSignal, onCleanup, onMount } from "solid-js";
import { chromeZoomAtom, setChromeZoomSignal } from "./chrome-zoom";
import { heldPaneZoom, holdPaneZoom, paneZoom, settlePaneZoom } from "./pane-zoom-local";
import { captureZoomAnchor, lastPointer, type ZoomPoint } from "./zoom-anchor";
import { clampZoom, DEFAULT_ZOOM, MIN_ZOOM, writePaneZoom, zoomMaxOf } from "./zoom-factor";
import {
    MAIN_PART,
    splitSubject,
    subjectId,
    zoomLabelKey,
    zoomMetaKey,
    zoomPartSpec,
    zoomSubjectAt,
    zoomSubjectForKeys,
    zoomSubjectsOf,
} from "./zoom-parts";
import { setBlockMeta } from "@/app/store/block-meta";
import { DURATION_LINGER_MS } from "@/app/motion/tokens";

// Zoom constants
const KEYBOARD_STEP = 0.1; // 10% increments for keyboard
export const WHEEL_STEP = 0.05; // 5% increments for scroll wheel
const MICRO_STEP = 0.01; // fine step for skip-to-next-size logic

// A block whose content is drawn at the zoom (a PDF) steps by a ratio of its level instead of the
// additive steps above, so every notch looks the same size at 50% and at 400%.
const stepRatios = new Map<string, number>();
export function setZoomStepRatio(blockId: string, ratio: number): () => void {
    stepRatios.set(blockId, ratio);
    return () => {
        if (stepRatios.get(blockId) === ratio) stepRatios.delete(blockId);
    };
}
/** How much of a wheel notch an event is: a trackpad pinch sends many small ones, a mouse wheel one of about 100. */
export const wheelNotches = (deltaY: number): number => Math.min(1, Math.max(0.1, Math.abs(deltaY) / 100));
/** The ratio one step of `notches` multiplies the zoom by, for a block that steps by ratio. */
export function ratioStepOf(blockId: string, notches: number): number | undefined {
    const ratio = stepRatios.get(blockId);
    return ratio == null ? undefined : Math.pow(ratio, notches);
}

// Zoom indicator visibility (auto-hide after 1.5s)
export const [zoomIndicatorVisibleAtom, setZoomIndicatorVisible] = createSignal<boolean>(false);
export const [zoomIndicatorTextAtom, setZoomIndicatorText] = createSignal<string>("");
/** The subject (`subjectId`, zoom-parts.ts) the amount belongs to, drawn centred on it; null for the window (All panes, the chrome zoom). */
export const [zoomIndicatorTargetAtom, setZoomIndicatorTarget] = createSignal<string | null>(null);
/** The i18n key of the part's name the amount carries ("File tree 130%"); null for a pane drawn as one. */
export const [zoomIndicatorLabelAtom, setZoomIndicatorLabel] = createSignal<string | null>(null);
let zoomIndicatorTimeout: NodeJS.Timeout | null = null;

function roundZoom(factor: number): number {
    return Math.round(factor * 100) / 100; // Round to 0.01 increments
}

// ── Per-pane zoom (terminal blocks) ───────────────────────────────

function getBaseFontSize(subject: string): number {
    const { blockId } = splitSubject(subject);
    const partFontSize = zoomPartSpec(subject)?.baseFontSize;
    if (partFontSize != null) return partFontSize;
    const blockOref = MOS.makeORef("block", blockId);
    const blockData = MOS.getObjectValue<Block>(blockOref);
    const metaFontSize = blockData?.meta?.["term:fontsize"];
    if (typeof metaFontSize === "number" && !isNaN(metaFontSize) && metaFontSize >= 4 && metaFontSize <= 64) {
        return metaFontSize;
    }
    const bcm = getBlockComponentModel(blockId);
    return paneTabCapability(bcm?.viewModel?.viewType, "paneZoom")?.baseFontSize ?? 15;
}

function computeEffectiveFontSize(baseFontSize: number, zoom: number): number {
    return Math.max(4, Math.min(64, Math.round(baseFontSize * zoom)));
}

// "warden" added here per Codex review on PR #3090: warden-view.tsx already
// reads/writes the identical "term:zoom" meta key and applies it as CSS
// zoom exactly like memory/swarm (both already listed) — leaving it out
// was an oversight in this allowlist, not a deliberate exclusion, and it
// made the all-panes batch below silently skip a pane that individual
// Ctrl+Scroll already zooms today via the exact same mechanism.
function getBlockZoom(subject: string): number | null {
    const { blockId } = splitSubject(subject);
    const bcm = getBlockComponentModel(blockId);
    if (!bcm?.viewModel) return null;
    // Which views take part is their `paneZoom` capability (Pane Tab
    // contract Phase 5) — formerly an allowlist here, which is how warden was
    // once left out by accident.
    if (!paneTabCapability(bcm.viewModel.viewType, "paneZoom")) return null;

    const blockOref = MOS.makeORef("block", blockId);
    const blockData = MOS.getObjectValue<Block>(blockOref);
    const stored = (blockData?.meta as Record<string, unknown> | undefined)?.[zoomMetaKey(subject)];
    return heldPaneZoom(subject) ?? (typeof stored === "number" ? stored : 1.0);
}

// Returns the actual clamped/rounded zoom that was written, so a caller
// that needs to know the resulting value (the all-panes stepper below)
// doesn't have to re-read it back — MOS's local cache is NOT updated
// synchronously by this call. setBlockMeta is fire-and-forget;
// the cache only updates later, when the backend pushes a MuxObjUpdate
// event back (global.ts's initGlobalEventSubs → MOS.updateMuxObject). A
// getBlockZoom() call immediately after this one would read the STALE
// pre-write value, not the one just computed here (ReAgent P1, PR #3090).
//
// The level is held locally (`pane-zoom-local.ts`) from this call until the write has settled, so a
// pane that reads `paneZoom` shows it at once and the next step starts from it.
function setBlockZoom(subject: string, factor: number, showIndicator: boolean = true): number {
    let seq = 0;
    const key = zoomMetaKey(subject);
    const { blockId } = splitSubject(subject);
    const newZoom = writePaneZoom((meta: Record<string, number | null>) => {
        seq = holdPaneZoom(subject, meta[key] ?? DEFAULT_ZOOM);
        fireAndForget(async () => {
            try {
                await setBlockMeta(blockId, meta);
                settlePaneZoom(subject, seq, false);
            } catch (e) {
                settlePaneZoom(subject, seq, true);
                throw e;
            }
        });
    }, factor, subject, key);

    // Suppressed by the all-panes stepper below, which shows one summary
    // toast for the whole gesture instead of one per pane.
    if (showIndicator) {
        showZoomIndicator(`${Math.round(newZoom * 100)}%`, subject);
    }
    return newZoom;
}

function stepZoom(
    subject: string,
    zoom: number,
    step: number,
    direction: 1 | -1,
    showIndicator: boolean = true,
    baseFontSizeOverride?: number,
    anchor: ZoomPoint | null = null,
    notches: number = 1
): number {
    const baseFontSize = baseFontSizeOverride ?? getBaseFontSize(subject);
    const currentFontSize = computeEffectiveFontSize(baseFontSize, zoom);
    const ratio = ratioStepOf(subject, notches);
    let newZoom = ratio == null ? zoom + step * direction : direction === 1 ? zoom * ratio : zoom / ratio;
    const limit = direction === 1 ? zoomMaxOf(subject) : MIN_ZOOM;
    // A block that steps by ratio is not drawn in font-size units, so no step is skipped for landing on the same size.
    while (
        ratio == null &&
        computeEffectiveFontSize(baseFontSize, newZoom) === currentFontSize &&
        (direction === 1 ? newZoom < limit : newZoom > limit)
    ) {
        newZoom += MICRO_STEP * direction;
    }
    // Note what is under the pointer before the level changes, so the pane can put it back. A step
    // that lands on the same level (the limit) moves nothing, so there is nothing to hold.
    if (Math.abs(clampZoom(roundZoom(newZoom), subject) - zoom) >= 0.005) captureZoomAnchor(subject, anchor);
    return setBlockZoom(subject, newZoom, showIndicator);
}

/** `anchor`: where the pointer is, so the content under it stays under it. */
export function zoomBlockIn(subject: string, step: number = WHEEL_STEP, anchor: ZoomPoint | null = null, notches?: number): void {
    const zoom = getBlockZoom(subject);
    if (zoom == null) return;
    stepZoom(subject, zoom, step, 1, true, undefined, anchor, notches);
}

export function zoomBlockOut(subject: string, step: number = WHEEL_STEP, anchor: ZoomPoint | null = null, notches?: number): void {
    const zoom = getBlockZoom(subject);
    if (zoom == null) return;
    stepZoom(subject, zoom, step, -1, true, undefined, anchor, notches);
}

/** A block's zoom set to `level`, with nothing held: a fit button, which places the content itself. */
export function zoomBlockTo(subject: string, level: number): void {
    setBlockZoom(subject, level);
}

/**
 * One Ctrl+scroll step on one block, by its own `term:zoom`: the shared wheel
 * step, the font-size skipping and the indicator. No capability check, so it
 * also zooms a headless sub-block (the agent pane's shell) that isn't a pane;
 * such a block passes the font size it draws at (`baseFontSize`), which its
 * pane capability can't tell the stepper.
 */
export function zoomBlockByWheel(
    subject: string,
    deltaY: number,
    baseFontSize?: number,
    anchor: ZoomPoint | null = null
): void {
    const meta = MOS.getObjectValue<Block>(MOS.makeORef("block", splitSubject(subject).blockId))?.meta;
    const level = paneZoom(subject, meta, zoomMetaKey(subject));
    stepZoom(subject, level, WHEEL_STEP, deltaY > 0 ? -1 : 1, true, baseFontSize, anchor, wheelNotches(deltaY));
}

/**
 * Ctrl+scroll zoom for content that handles the wheel itself and would stop it
 * before the window's handler (`AppZoomHandler`, app.tsx) sees it: xterm,
 * CodeMirror. Capture phase, so it runs first; Ctrl+Shift+scroll is left to the
 * all-panes handler. Every other pane needs nothing: the window handler zooms
 * the pane under the pointer. SPEC_SYSINFO_TEXT_ZOOM_DENSITY_AND_ONE_ZOOM_PATH_2026_10_18.md §3.
 */
export function usePaneZoomWheel(
    el: () => HTMLElement | undefined,
    blockId: () => string | null | undefined,
    baseFontSize?: number,
    /** False for a wheel the pane leaves to the window's handler (chrome zoom over its own chrome). */
    applies?: (ev: WheelEvent) => boolean
): void {
    onMount(() => {
        const target = el();
        if (!target) return;
        const onWheel = (ev: WheelEvent) => {
            if (!isModKey(ev) || ev.shiftKey) return;
            if (applies && !applies(ev)) return;
            const id = blockId();
            if (!id) return;
            // Also stops CEF's own Ctrl+scroll page zoom.
            ev.preventDefault();
            ev.stopPropagation();
            // The part under the pointer, for a pane whose parts zoom apart.
            zoomBlockByWheel(zoomSubjectAt(ev.target, id), ev.deltaY, baseFontSize, { clientX: ev.clientX, clientY: ev.clientY });
        };
        target.addEventListener("wheel", onWheel, { passive: false, capture: true });
        onCleanup(() => target.removeEventListener("wheel", onWheel, { capture: true }));
    });
}

// ── All-panes zoom (Ctrl+Shift+Scroll) ──────────────────────────────
//
// See docs/specs/SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md.
// Steps every pane in the CURRENT WINDOW's block registry, each relative
// to its own current zoom level — this is a batch of independent
// single-pane zooms, not a new shared value. A pane whose viewType
// getBlockZoom() doesn't recognize (browser, sysinfo, help, ...)
// is silently skipped, the same way a single Ctrl+Scroll over one of
// them already is today — no new filtering logic, this just reuses that
// existing guard for every block in the window instead of one.

function allSubjects(): string[] {
    return [...getAllBlockComponentModelEntries()].flatMap(([blockId]) => zoomSubjectsOf(blockId));
}

function stepAllPanes(step: number, direction: 1 | -1, anchor: ZoomPoint | null = null): void {
    let minZoom = Infinity;
    let maxZoom = -Infinity;

    // Every level of every pane: a pane with parts steps each part by its own level.
    for (const subject of allSubjects()) {
        const zoom = getBlockZoom(subject);
        if (zoom == null) continue; // not a zoomable pane type — leave untouched

        // Suppress the per-pane indicator: N panes stepping in one gesture
        // would otherwise fire N toasts, each overwriting the last before
        // the user can read it. One summary toast is shown below instead.
        //
        // Uses stepZoom's OWN return value, not a getBlockZoom() re-read —
        // the write it just fired is an async setBlockMeta, and
        // MOS's local cache isn't updated until the backend pushes a
        // MuxObjUpdate event back. Re-reading here would see the STALE
        // pre-step value on every call (ReAgent P1, PR #3090).
        // Each pane takes the pointer if it is over it, else its own centre.
        const newZoom = stepZoom(subject, zoom, step, direction, false, undefined, anchor);
        minZoom = Math.min(minZoom, newZoom);
        maxZoom = Math.max(maxZoom, newZoom);
    }

    if (minZoom === Infinity) return; // no zoomable pane in this window

    // Panes that stepped from different levels have no one figure: say what was done, not a range of unrelated numbers.
    const label =
        Math.abs(minZoom - maxZoom) < 0.01
            ? `All panes: ${Math.round(minZoom * 100)}%`
            : `All panes zoomed ${direction > 0 ? "in" : "out"}`;
    showZoomIndicator(label);
}

/** Every zoomable pane in the window back to 100%, with one summary toast. */
export function zoomAllPanesReset(): void {
    let any = false;
    for (const subject of allSubjects()) {
        if (getBlockZoom(subject) == null) continue;
        setBlockZoom(subject, DEFAULT_ZOOM, false);
        any = true;
    }
    if (any) showZoomIndicator("All panes: 100%");
}

export function zoomAllPanesIn(step: number = WHEEL_STEP, anchor: ZoomPoint | null = null): void {
    stepAllPanes(step, 1, anchor);
}

export function zoomAllPanesOut(step: number = WHEEL_STEP, anchor: ZoomPoint | null = null): void {
    stepAllPanes(step, -1, anchor);
}

/**
 * The subject a key or command zooms: `partId`'s level in the selected pane when named, else the part that
 * last had the keyboard in it (a pane with no parts: the pane).
 */
function keyboardSubject(partId?: string): string | null {
    const blockId = getFocusedBlockId();
    if (!blockId) return null;
    return partId ? subjectId(blockId, partId === MAIN_PART ? null : partId) : zoomSubjectForKeys(blockId);
}

function stepByKeyboard(direction: 1 | -1, step: number, partId?: string): void {
    const subject = keyboardSubject(partId);
    if (!subject) return;
    const zoom = getBlockZoom(subject);
    if (zoom == null) return;
    // The keyboard has no pointer: the last one seen, if it is over the pane, else the pane's centre.
    stepZoom(subject, zoom, step, direction, true, undefined, lastPointer());
}

/** `partId`: a part of the selected pane by name (the palette's "Zoom in file tree"); none: the part with the keyboard. */
export function zoomIn(step: number = KEYBOARD_STEP, partId?: string): void {
    stepByKeyboard(1, step, partId);
}

export function zoomOut(step: number = KEYBOARD_STEP, partId?: string): void {
    stepByKeyboard(-1, step, partId);
}

export function zoomReset(partId?: string): void {
    const subject = keyboardSubject(partId);
    if (subject) setBlockZoom(subject, DEFAULT_ZOOM);
}

/** Every part of the selected pane back to 100% ("Reset zoom in file tree and editor"). */
export function zoomPaneReset(): void {
    const blockId = getFocusedBlockId();
    if (!blockId) return;
    for (const subject of zoomSubjectsOf(blockId)) setBlockZoom(subject, DEFAULT_ZOOM, subject === blockId);
}

// ── Chrome zoom (title bar + status bar) ──────────────────────────

// Kept in sessionStorage, so a reload of the window (a recycle, a version-skew
// reload) keeps it, and an app restart starts at 100% as before
// (docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §2).
const CHROME_ZOOM_KEY = "agentmux:chrome-zoom";

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

function setChromeZoom(factor: number): void {
    const clamped = clampZoom(roundZoom(factor));
    setChromeZoomSignal(clamped);
    applyChromeZoomCSS(clamped);
    try {
        sessionStorage.setItem(CHROME_ZOOM_KEY, String(clamped));
    } catch {
        // Unavailable: the zoom still applies to this page.
    }
    showZoomIndicator(`Chrome ${Math.round(clamped * 100)}%`);
}

/**
 * The chrome zoom this window had before a reload, or the default. `fresh`
 * (a pool boot) forgets the stored value: a pool renderer keeps its
 * sessionStorage when a window is demoted back to it, and the next window
 * promoted from it is a different one.
 */
export function initChromeZoom(opts?: { fresh?: boolean }): void {
    let stored = NaN;
    try {
        if (opts?.fresh) sessionStorage.removeItem(CHROME_ZOOM_KEY);
        else stored = Number(sessionStorage.getItem(CHROME_ZOOM_KEY) ?? NaN);
    } catch {
        // Unavailable: start at the default.
    }
    const factor = Number.isFinite(stored) ? clampZoom(stored) : DEFAULT_ZOOM;
    setChromeZoomSignal(factor);
    applyChromeZoomCSS(factor);
}

// ── Shared helpers ────────────────────────────────────────────────

/**
 * Show the amount for `subject`, centred on it (the pane, or the part of it that zoomed, named in the text
 * while the pane has parts on screen); with no subject it is the window's, centred on the window.
 */
export function showZoomIndicator(text: string, subject: string | null = null): void {
    if (zoomIndicatorTimeout) {
        clearTimeout(zoomIndicatorTimeout);
    }
    // One update, so a second pane's amount moves the box once, from where it was to the new pane.
    batch(() => {
        setZoomIndicatorTarget(subject);
        setZoomIndicatorLabel(subject == null ? null : zoomLabelKey(subject));
        setZoomIndicatorText(text);
        setZoomIndicatorVisible(true);
    });

    zoomIndicatorTimeout = setTimeout(() => {
        setZoomIndicatorVisible(false);
        zoomIndicatorTimeout = null;
    }, DURATION_LINGER_MS);
}
