// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A picture of a pane, so its floating window can show the pane the moment
 * the window appears instead of the startup splash, until its live content
 * reveals. SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md phase 3.1.
 *
 * Taken on pointerdown on the pane's drag handle, before the drag starts: the
 * drag draws over the pane (its tear-off place, tilelayout.scss), and a picture
 * taken at release would add its capture time to the tear-off. The browser
 * pane's drag catcher prewarms the same way (use-drag-snapshot.ts).
 *
 * The capture is the window's whole viewport, cropped here to the pane. A
 * clipped CDP screenshot (the block-scoped `browserPanes.screenshot`) makes
 * Chromium briefly change the visible page's viewport to render the clip,
 * which flickered the window as the drag began.
 *
 * A native browser pane's page is a separate CEF browser drawn above the DOM
 * placeholder, so the window's capture shows the pane's header and address bar
 * around an empty placeholder. Its picture is that capture with the page's own
 * screenshot (of its whole page: no clip, so no flicker) drawn into the
 * placeholder, so the pane in hand looks like the pane. Before, it was the
 * page alone: the pane lost its header the moment it was picked up.
 *
 * A tab of a pane that has other tabs is pictured alone: the pane as it would stand with that tab only, its
 * pill first in the header and the others not there (`tabStripIn`). A tab dragged out of its pane leaves the
 * others behind, and its picture took them all along (the operator, 2026-11-13: "when a pane tab tears off, its
 * taking all the other pane tabs with it").
 */

import { getApi, MOS } from "@/app/store/global";
import { Logger } from "@/util/logger";
import { isWindows } from "@/util/platformutil";
import { beginPaneCapture } from "./pane-capture-gate";

/** A picture older than this is stale: the pane may have changed since. */
const MAX_AGE_MS = 10_000;

/** How long a tear-off waits for a capture still in flight. */
const TAKE_BUDGET_MS = 60;

/**
 * The largest picture (base64 chars) passed along. It travels in the
 * open_floating_pane_window request, whose body the host caps at 2 MiB
 * (axum's default); a bigger one would fail the tear-off itself.
 */
export const MAX_SNAPSHOT_CHARS = 1_000_000;

let held: {
    blockId: string;
    at: number;
    picture: Promise<string | null>;
    /** A pane's: where the pointer went down in it, and its size, both CSS px. */
    grab?: { x: number; y: number };
    size?: { width: number; height: number };
    /** The live follow is showing this picture: it is what the user has been looking at, however long the drag. */
    following?: boolean;
} | null = null;

export interface CssRect {
    left: number;
    top: number;
    width: number;
    height: number;
}

/** Where the pointer last pressed a pane's drag handle (viewport px): the grab of a picture taken after the press. */
let lastPress: { x: number; y: number } | undefined;

/**
 * A pane's tab strip as its picture needs it to show one tab alone, in CSS px from the pane's own corner: the
 * part of the header the tabs are drawn in, the pictured tab's pill (what is drawn of it), and the header's
 * colour.
 */
export interface TabStripInPane {
    strip: CssRect;
    own: CssRect;
    background: string;
}

const HEADER_SELECTOR = '[data-role="block-header"]';
/** The header's scrolling part, which clips the tabs (blockframe.tsx). */
const HEADER_SCROLL_SELECTOR = ".block-frame-header-scroll";

/** `a` cut to what lies inside `b`; none when nothing does. */
function within(a: CssRect, b: CssRect): CssRect | undefined {
    const left = Math.max(a.left, b.left);
    const top = Math.max(a.top, b.top);
    const width = Math.min(a.left + a.width, b.left + b.width) - left;
    const height = Math.min(a.top + a.height, b.top + b.height) - top;
    return width > 0 && height > 0 ? { left, top, width, height } : undefined;
}

/** A colour that draws nothing: none, `transparent`, or one whose alpha is 0. */
const drawsNothing = (colour: string): boolean => !colour || colour === "transparent" || /,\s*0\)$/.test(colour);

/**
 * The tab strip of pane `el` for a picture of tab `blockId` alone. None when the pane shows no other tab (the
 * picture is already of the tab alone), when the tab's pill is not drawn, or when the header has no colour of
 * its own to draw over the others with: the picture then stays as the window shows the pane.
 */
export function tabStripIn(el: Element, blockId: string): TabStripInPane | undefined {
    const header = el.querySelector(HEADER_SELECTOR);
    if (!header) return undefined;
    const pills = Array.from(header.querySelectorAll<HTMLElement>(".pane-tab[data-tab-id]"));
    const pill = pills.find((p) => p.dataset.tabId === blockId);
    if (!pill || pills.length < 2) return undefined;
    const background = getComputedStyle(header).backgroundColor;
    if (drawsNothing(background)) return undefined;
    const pane = el.getBoundingClientRect();
    const strip = within((pill.closest(HEADER_SCROLL_SELECTOR) ?? header).getBoundingClientRect(), header.getBoundingClientRect());
    const own = strip && within(pill.getBoundingClientRect(), strip);
    if (!strip || !own) return undefined;
    const inPane = (r: CssRect): CssRect => ({ ...r, left: r.left - pane.left, top: r.top - pane.top });
    return { strip: inPane(strip), own: inPane(own), background };
}

/**
 * Draw the pictured tab alone into the pane's picture: the header's colour over the strip, and the tab's own
 * pill (from the window's picture `full`, where the pane's corner is at `crop`) at the strip's start.
 */
function drawTabAlone(
    ctx: OffscreenCanvasRenderingContext2D | null | undefined,
    full: ImageBitmap,
    crop: { x: number; y: number },
    solo: TabStripInPane,
    scale: number
): void {
    if (!ctx) return;
    const px = (v: number) => Math.round(v * scale);
    const strip = { x: px(solo.strip.left), y: px(solo.strip.top), w: px(solo.strip.width), h: px(solo.strip.height) };
    ctx.fillStyle = solo.background;
    ctx.fillRect(strip.x, strip.y, strip.w, strip.h);
    const [w, h] = [Math.min(px(solo.own.width), strip.w), px(solo.own.height)];
    if (w <= 0 || h <= 0) return;
    ctx.drawImage(full, crop.x + px(solo.own.left), crop.y + px(solo.own.top), w, h, strip.x, px(solo.own.top), w, h);
}

/**
 * The pane's rect in the viewport image's pixels: CSS px scaled by the
 * image's own width over the viewport's (that covers the device pixel ratio
 * and the app zoom), clamped to the image. Null if nothing is left.
 */
export function cropRectInImage(
    rect: CssRect,
    viewportCssWidth: number,
    image: { width: number; height: number }
): { x: number; y: number; w: number; h: number } | null {
    if (viewportCssWidth <= 0) return null;
    const scale = image.width / viewportCssWidth;
    const x = Math.max(0, Math.round(rect.left * scale));
    const y = Math.max(0, Math.round(rect.top * scale));
    const w = Math.min(image.width, Math.round((rect.left + rect.width) * scale)) - x;
    const h = Math.min(image.height, Math.round((rect.top + rect.height) * scale)) - y;
    return w > 0 && h > 0 ? { x, y, w, h } : null;
}

function base64ToBlob(b64: string, type: string): Blob {
    const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
    return new Blob([bytes], { type });
}

/**
 * Decode/crop/encode work still running, including work nobody will take (a
 * newer prewarm replaced it, or the take gave up). Tracked so a test can wait
 * for work it didn't await (`settleTearOffSnapshotForTests`). Only the work
 * after the capture RPC answers, which always finishes: a stalled RPC is
 * never tracked, so it can't be retained here forever (Codex P2 on #4110).
 */
const pendingCaptures = new Set<Promise<unknown>>();

function track<T>(capture: Promise<T>): Promise<T> {
    pendingCaptures.add(capture);
    const forget = () => pendingCaptures.delete(capture);
    capture.then(forget, forget);
    return capture;
}

/** Base64 of `bytes`, in chunks so a large picture doesn't overflow the call stack. */
function bytesToBase64(bytes: Uint8Array): string {
    const CHUNK = 0x8000;
    let binary = "";
    for (let i = 0; i < bytes.length; i += CHUNK) {
        binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
    }
    return btoa(binary);
}

/**
 * Not `FileReader.readAsDataURL`: under jsdom that finishes in a later
 * `setImmediate` and can throw "Expected an Uint8Array" from inside jsdom, out
 * of reach of any handler (the intermittent CI failure in
 * docs/reports/REPORT_CI_VITEST_TEAROFF_SNAPSHOT_UNCAUGHT_2026_09_30.md).
 * `arrayBuffer()` is equivalent here, and in Chromium no slower.
 */
async function blobToBase64(blob: Blob): Promise<string> {
    return bytesToBase64(new Uint8Array(await blob.arrayBuffer()));
}

function ownWindowLabel(): string {
    return new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
}

/** The window's viewport, cropped to `rect`, as a base64 JPEG. `shot` is called once the window has been pictured. */
async function capturePane(rect: CssRect, shot: () => void, solo?: TabStripInPane): Promise<string | null> {
    const { jpeg_base64 } = await getApi().windows.captureViewport(ownWindowLabel(), 80);
    shot();
    return track(cropToPane(jpeg_base64, rect, solo));
}

async function cropToPane(jpeg_base64: string, rect: CssRect, solo?: TabStripInPane): Promise<string | null> {
    const full = await createImageBitmap(base64ToBlob(jpeg_base64, "image/jpeg"));
    try {
        const crop = cropRectInImage(rect, window.innerWidth, full);
        if (!crop) return null;
        const canvas = new OffscreenCanvas(crop.w, crop.h);
        const ctx = canvas.getContext("2d");
        ctx?.drawImage(full, crop.x, crop.y, crop.w, crop.h, 0, 0, crop.w, crop.h);
        if (solo) drawTabAlone(ctx, full, crop, solo, full.width / window.innerWidth);
        return await blobToBase64(await canvas.convertToBlob({ type: "image/jpeg", quality: 0.8 }));
    } finally {
        full.close();
    }
}

/** `inner` (the page's placeholder) in the picture of `pane`, both CSS px of the window, in the picture's pixels. */
export function pageRectInPicture(
    inner: CssRect,
    pane: CssRect,
    scale: number,
    picture: { width: number; height: number }
): { x: number; y: number; w: number; h: number } | null {
    const x = Math.max(0, Math.round((inner.left - pane.left) * scale));
    const y = Math.max(0, Math.round((inner.top - pane.top) * scale));
    const w = Math.min(picture.width, Math.round((inner.left - pane.left + inner.width) * scale)) - x;
    const h = Math.min(picture.height, Math.round((inner.top - pane.top + inner.height) * scale)) - y;
    return w > 0 && h > 0 ? { x, y, w, h } : null;
}

/**
 * A browser pane's picture: the window's capture cropped to the pane, with the page's screenshot drawn where
 * the page stands (`placeholder`). The two are asked for together. Without the window's capture (it failed) the
 * picture is the page alone, as it was; without the page's (asleep, not loaded) it is the pane as the window
 * shows it. `shot` is called once the window has been pictured.
 */
async function captureBrowserPane(
    blockId: string,
    rect: CssRect,
    placeholder: CssRect | null,
    shot: () => void,
    solo?: TabStripInPane
): Promise<string | null> {
    const page = getApi()
        .browserPanes.screenshot(blockId, { format: "jpeg", quality: 80 })
        .then(
            (got) => got?.png_base64 || null,
            () => null
        );
    const frame = await getApi()
        .windows.captureViewport(ownWindowLabel(), 80)
        .then(
            (got) => got.jpeg_base64,
            () => null
        );
    shot();
    const pageShot = await page;
    if (!frame || !placeholder) return pageShot;
    return track(composeBrowserPane(frame, rect, pageShot, placeholder, solo));
}

async function composeBrowserPane(
    frame: string,
    rect: CssRect,
    pageShot: string | null,
    placeholder: CssRect,
    solo?: TabStripInPane
): Promise<string | null> {
    const full = await createImageBitmap(base64ToBlob(frame, "image/jpeg"));
    // A page's picture that does not decode leaves the pane as the window shows it.
    let page: ImageBitmap | null = null;
    try {
        page = pageShot ? await createImageBitmap(base64ToBlob(pageShot, "image/jpeg")) : null;
    } catch {
        page = null;
    }
    try {
        const crop = cropRectInImage(rect, window.innerWidth, full);
        if (!crop) return pageShot;
        const canvas = new OffscreenCanvas(crop.w, crop.h);
        const ctx = canvas.getContext("2d");
        ctx?.drawImage(full, crop.x, crop.y, crop.w, crop.h, 0, 0, crop.w, crop.h);
        const at = page && pageRectInPicture(placeholder, rect, full.width / window.innerWidth, { width: crop.w, height: crop.h });
        if (page && at) ctx?.drawImage(page, 0, 0, page.width, page.height, at.x, at.y, at.w, at.h);
        if (solo) drawTabAlone(ctx, full, crop, solo, full.width / window.innerWidth);
        return await blobToBase64(await canvas.convertToBlob({ type: "image/jpeg", quality: 0.8 }));
    } finally {
        full.close();
        page?.close();
    }
}

/** Scales tried, in order, for a viewport picture too large for the request. */
const SHRINK_SCALES = [0.5, 0.35, 0.25];

/**
 * The whole viewport, for a window-tab tear-off: the new window is the source
 * window's size, so the picture fills it as is. Shrunk step by step if it
 * wouldn't fit the request that carries it; null if nothing fits.
 */
async function captureWindow(): Promise<string | null> {
    const { jpeg_base64 } = await getApi().windows.captureViewport(ownWindowLabel(), 80);
    if (jpeg_base64.length <= MAX_SNAPSHOT_CHARS) return jpeg_base64;
    return track(shrinkToFit(jpeg_base64));
}

async function shrinkToFit(jpeg_base64: string): Promise<string | null> {
    const full = await createImageBitmap(base64ToBlob(jpeg_base64, "image/jpeg"));
    try {
        for (const scale of SHRINK_SCALES) {
            const w = Math.max(1, Math.round(full.width * scale));
            const h = Math.max(1, Math.round(full.height * scale));
            const canvas = new OffscreenCanvas(w, h);
            canvas.getContext("2d")?.drawImage(full, 0, 0, w, h);
            const small = await blobToBase64(await canvas.convertToBlob({ type: "image/jpeg", quality: 0.7 }));
            if (small.length <= MAX_SNAPSHOT_CHARS) return small;
        }
        return null;
    } finally {
        full.close();
    }
}

/**
 * Whether `el` is actually drawn. Hidden window tabs and inactive pane tabs
 * stay laid out (real client rects) and are hidden by `visibility: hidden`
 * (workspace.tsx, pane-leaf-chrome.tsx), `content-visibility: hidden`
 * (window:keepinactivetabslaidout=false) or opacity; checkVisibility()
 * covers all of them. Without it, rects plus computed visibility.
 */
function isRendered(el: Element): boolean {
    if (typeof el.checkVisibility === "function") {
        return el.checkVisibility({
            contentVisibilityAuto: true,
            opacityProperty: true,
            visibilityProperty: true,
            // Pre-121 names for the same checks.
            checkOpacity: true,
            checkVisibilityCSS: true,
        } as CheckVisibilityOptions);
    }
    return el.getClientRects().length > 0 && getComputedStyle(el).visibility === "visible";
}

/** The key a window tab's picture is held under (panes use their block id). */
const windowTabKey = (tabId: string) => `window-tab:${tabId}`;

/**
 * Start capturing the window for a tear-off of window tab `tabId`. Only for
 * the active tab: an inactive one's content isn't on screen.
 */
export function prewarmWindowTabSnapshot(tabId: string): void {
    // A native browser pane on screen would be a grey placeholder in the
    // window's capture (see above): no picture beats a wrong one.
    const browserOnScreen = Array.from(document.querySelectorAll(".browser-placeholder")).some(isRendered);
    if (browserOnScreen) {
        held = null;
        return;
    }
    const at = Date.now();
    const key = windowTabKey(tabId);
    const picture = captureWindow().then(
        (b64) => {
            Logger.debug("dnd", "tear-off window snapshot captured", { tabId, ms: Date.now() - at });
            return b64;
        },
        (e) => {
            Logger.debug("dnd", "tear-off window snapshot failed", { tabId, error: String(e) });
            return null;
        }
    );
    held = { blockId: key, at, picture };
}

/** The picture taken for tearing off window tab `tabId`, if ready in time. */
export function takeWindowTabSnapshot(tabId: string): Promise<string | undefined> {
    return takeTearOffSnapshot(windowTabKey(tabId));
}

/**
 * The picture held for window tab `tabId`, without taking it: the live follow shows it during the drag, and
 * the tear-off at release takes it for the new window. Waits for a capture still in flight, up to `waitMs`.
 */
export async function peekWindowTabSnapshot(tabId: string, waitMs: number): Promise<string | undefined> {
    return (await peekHeld(windowTabKey(tabId), waitMs))?.picture;
}

/** A pane's picture for the live follow, with the pointer's place in the pane and the pane's size. */
export interface PaneFollowPicture {
    picture: string;
    grab: { x: number; y: number };
    size: { width: number; height: number };
}

/** The picture held for pane `blockId`, without taking it (see `peekWindowTabSnapshot`). */
export async function peekTearOffSnapshot(blockId: string, waitMs: number): Promise<PaneFollowPicture | undefined> {
    const got = await peekHeld(blockId, waitMs);
    return got?.grab && got.size ? { picture: got.picture, grab: got.grab, size: got.size } : undefined;
}

async function peekHeld(key: string, waitMs: number) {
    const h = held;
    if (!h || h.blockId !== key || Date.now() - h.at > MAX_AGE_MS) return undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const budget = new Promise<null>((resolve) => {
        timer = setTimeout(() => resolve(null), waitMs);
    });
    const picture = await Promise.race([h.picture, budget]);
    clearTimeout(timer);
    if (!picture || picture.length > MAX_SNAPSHOT_CHARS) return undefined;
    h.following = true;
    return { picture, grab: h.grab, size: h.size };
}

/**
 * Start capturing `blockId` (a base64 JPEG), replacing any earlier picture. `pointer` is where the press
 * landed (viewport px): the live follow keeps the pane at that grab.
 */
export function prewarmTearOffSnapshot(blockId: string, pointer?: { x: number; y: number }): void {
    const at = Date.now();
    if (pointer) lastPress = pointer;
    // The rect now, before the drag restyles or moves anything.
    const el = document.querySelector(`[data-blockid="${CSS.escape(blockId)}"]`);
    if (!el) {
        held = null;
        return;
    }
    const r = el.getBoundingClientRect();
    const solo = tabStripIn(el, blockId);
    // The block's own view, not the DOM: a pane keeps its inactive tabs
    // mounted, so a background browser tab would match a DOM query.
    const isBrowser = MOS.getObjectValue<Block>(MOS.makeORef("block", blockId))?.meta?.view === "browser";
    // The drag's own drawing waits for the capture: it is a picture of the window as drawn when it runs
    // (pane-capture-gate.ts). The crop and the encoding after it hold nothing back.
    const captured = beginPaneCapture();
    const paneRect = { left: r.left, top: r.top, width: r.width, height: r.height };
    // The page's own placeholder: the one that is drawn (a pane keeps its other tabs' views mounted, hidden).
    const ph = isBrowser ? Array.from(el.querySelectorAll(".browser-placeholder")).find(isRendered)?.getBoundingClientRect() : undefined;
    const capture = isBrowser
        ? captureBrowserPane(blockId, paneRect, ph ? { left: ph.left, top: ph.top, width: ph.width, height: ph.height } : null, captured, solo)
        : capturePane(paneRect, captured, solo);
    const picture = capture.then(
        (b64) => {
            captured();
            Logger.debug("dnd:cross", "tear-off snapshot captured", { blockId, ms: Date.now() - at });
            return b64;
        },
        (e) => {
            captured();
            Logger.debug("dnd:cross", "tear-off snapshot failed", { blockId, error: String(e) });
            return null;
        }
    );
    held = {
        blockId,
        at,
        picture,
        grab: pointer && grabIn(r, pointer, solo),
        size: { width: r.width, height: r.height },
    };
}

/**
 * Where the pointer holds the picture of the pane at `r`. In a picture of one tab alone the tab's pill has
 * moved to the strip's start: a pointer that pressed the pill holds it there too.
 */
function grabIn(r: CssRect, pointer: { x: number; y: number }, solo: TabStripInPane | undefined): { x: number; y: number } {
    const grab = { x: pointer.x - r.left, y: pointer.y - r.top };
    if (!solo) return grab;
    const { own, strip } = solo;
    const onPill = grab.x >= own.left && grab.x <= own.left + own.width && grab.y >= own.top && grab.y <= own.top + own.height;
    return onPill ? { x: grab.x - (own.left - strip.left), y: grab.y } : grab;
}

/**
 * The picture of tab `blockId` for the drag that is starting. The press took one of the pane as it stood then,
 * under the tab that was in front: a tab pressed while another was in front has none of its own, so it is
 * taken now, with the press as its grab. Where a press takes pictures (Windows).
 */
export function ensureTabPicture(blockId: string): void {
    if (!isWindows()) return;
    if (held?.blockId === blockId && Date.now() - held.at <= MAX_AGE_MS) return;
    prewarmTearOffSnapshot(blockId, lastPress);
}

/**
 * A picture of pane `blockId` as it stands on screen now (a base64 JPEG), with the pane's size in CSS px: what
 * the landing place of a dragged tab shows (layout/lib/paneLandingPicture.ts). The picture taken at the press
 * can be of a tab that had only just been shown, still empty. The drag's own drawing waits for it as it does
 * for the press's (pane-capture-gate.ts), so the pane is pictured where it stood. Nothing held is touched: the
 * press's picture stays the one a tear-off opens on. None when the pane is not drawn, or cannot be pictured.
 */
export async function picturePaneNow(blockId: string): Promise<{ picture: string; size: { width: number; height: number } } | undefined> {
    const el = document.querySelector(`[data-blockid="${CSS.escape(blockId)}"]`);
    const r = el?.getBoundingClientRect();
    if (!el || !r || !(r.width > 0) || !(r.height > 0) || !isRendered(el)) return undefined;
    const isBrowser = MOS.getObjectValue<Block>(MOS.makeORef("block", blockId))?.meta?.view === "browser";
    const solo = tabStripIn(el, blockId);
    const captured = beginPaneCapture();
    const paneRect = { left: r.left, top: r.top, width: r.width, height: r.height };
    const ph = isBrowser ? Array.from(el.querySelectorAll(".browser-placeholder")).find(isRendered)?.getBoundingClientRect() : undefined;
    try {
        const picture = await (isBrowser
            ? captureBrowserPane(blockId, paneRect, ph ? { left: ph.left, top: ph.top, width: ph.width, height: ph.height } : null, captured, solo)
            : capturePane(paneRect, captured, solo));
        return picture ? { picture, size: { width: r.width, height: r.height } } : undefined;
    } catch (e) {
        Logger.debug("dnd:cross", "landing picture failed", { blockId, error: String(e) });
        return undefined;
    } finally {
        captured();
    }
}

/**
 * The picture of `blockId` taken for this drag, if it's ready within the
 * budget. Never starts a capture: that would delay the tear-off itself.
 */
export async function takeTearOffSnapshot(blockId: string): Promise<string | undefined> {
    const h = held;
    held = null;
    // A picture the live follow has shown goes to the floater at any age: the floater opens on exactly
    // what the user saw, and a drag held for a while must not land on the loading logo instead.
    if (!h || h.blockId !== blockId || (!h.following && Date.now() - h.at > MAX_AGE_MS)) return undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const budget = new Promise<null>((resolve) => {
        timer = setTimeout(() => resolve(null), TAKE_BUDGET_MS);
    });
    const picture = await Promise.race([h.picture, budget]);
    clearTimeout(timer);
    if (picture && picture.length > MAX_SNAPSHOT_CHARS) {
        Logger.info("dnd:cross", "tear-off snapshot too large, dropped", { blockId, chars: picture.length });
        return undefined;
    }
    return picture ?? undefined;
}

/**
 * The pane a pointerdown may be about to drag out of its window: a
 * pragmatic draggable (a pane header or a Pane Tab pill) inside a pane.
 */
export function paneDragCandidate(target: EventTarget | null): string | null {
    if (!(target instanceof Element) || !target.closest('[draggable="true"]')) return null;
    return target.closest("[data-blockid]")?.getAttribute("data-blockid") ?? null;
}

/** Test hook. */
export function resetTearOffSnapshotForTests(): void {
    held = null;
    lastPress = undefined;
}

/**
 * Waits for every decode/encode still running, including ones no test
 * awaited. Lets a task pass first, so a capture whose RPC has already
 * answered gets to register its work before the set is checked.
 */
export async function settleTearOffSnapshotForTests(): Promise<void> {
    do {
        await new Promise((resolve) => setTimeout(resolve, 0));
        await Promise.allSettled([...pendingCaptures]);
    } while (pendingCaptures.size > 0);
}
