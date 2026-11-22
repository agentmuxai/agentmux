// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window's answer to srv's `capture:mask` asks (`capture-mask.ts`; crates/srv/src/sagas/capture_mask.rs):
 * `begin` starts a watch and answers what the window shows; `end` stops it and answers whether anything changed.
 * The answer goes as `capture.mask.report`, which srv takes only from the window.
 */

import { atoms, MOS } from "@/app/store/global";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { CaptureMaskEvent } from "@/types/rpc/CaptureMaskEvent";
import type { WindowCaptureMask } from "@/types/rpc/WindowCaptureMask";
import { OVERLAY_SELECTOR, overlayAnchor } from "./overlay-anchors";
import { CaptureWatch, measure, openListRefusal, PRIVATE_SELECTOR, toMask, type MaskSource, type Measured } from "./capture-mask";

/** A watch not ended by then stops measuring. */
const WATCH_MS = 15_000;

export interface MaskBridgeDeps {
    now(): number;
    windowId(): string | undefined;
    title(): string;
    read(): Measured;
    nextFrame(cb: () => void): void;
    /** Why the window can't give its rectangles now (a pane is being dragged), if it can't. */
    refusal(): string | undefined;
    answer(captureId: string, phase: "begin" | "end", mask: WindowCaptureMask): Promise<void>;
}

/** Answer one ask; `watches` holds the captures between `begin` and `end`. */
export async function answerAsk(ev: CaptureMaskEvent, deps: MaskBridgeDeps, watches: Map<string, CaptureWatch>): Promise<void> {
    const windowId = deps.windowId();
    if (!windowId || Number(ev.expires_at_ms) < deps.now()) return;
    if (ev.phase === "begin") {
        const watch = new CaptureWatch(() => deps.read(), deps.nextFrame, deps.now() + WATCH_MS, deps.now, () => deps.refusal());
        watches.set(ev.capture_id, watch);
        setTimeout(() => watches.delete(ev.capture_id), WATCH_MS);
        await deps.answer(ev.capture_id, "begin", toMask(windowId, deps.title(), watch.begin, { refuse: deps.refusal() }));
        return;
    }
    const watch = watches.get(ev.capture_id);
    watches.delete(ev.capture_id);
    if (!watch) {
        await deps.answer(ev.capture_id, "end", toMask(windowId, deps.title(), deps.read(), { changed: true }));
        return;
    }
    const { measured, changed, refused } = watch.end();
    await deps.answer(ev.capture_id, "end", toMask(windowId, deps.title(), measured, { changed, refuse: refused ?? deps.refusal() }));
}

let dragging = false;

/** An overlay's whole drawn area: its shell and what is inside it, which may be placed outside the shell. */
function unionRect(el: Element): { x: number; y: number; w: number; h: number } {
    const rects = [el, ...el.querySelectorAll("*")].map((e) => e.getBoundingClientRect()).filter((r) => r.width > 0 && r.height > 0);
    if (rects.length === 0) return { x: 0, y: 0, w: 0, h: 0 };
    const x0 = Math.min(...rects.map((r) => r.left));
    const y0 = Math.min(...rects.map((r) => r.top));
    const x1 = Math.max(...rects.map((r) => r.right));
    const y1 = Math.max(...rects.map((r) => r.bottom));
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

function windowSource(): MaskSource {
    const rectOf = (el: Element) => {
        const r = el.getBoundingClientRect();
        return { x: r.x, y: r.y, w: r.width, h: r.height };
    };
    return {
        width: window.innerWidth,
        height: window.innerHeight,
        scale: window.devicePixelRatio || 1,
        panes: () =>
            [...document.querySelectorAll("[data-blockid]")].map((el) => ({ blockId: el.getAttribute("data-blockid") ?? "", rect: rectOf(el) })),
        panels: () => [...document.querySelectorAll(PRIVATE_SELECTOR)].map(rectOf),
        overlays: () =>
            [...document.querySelectorAll(OVERLAY_SELECTOR)].flatMap((el) => {
                const anchor = overlayAnchor(el);
                if (!anchor) return [];
                const blockId = anchor.closest("[data-blockid]")?.getAttribute("data-blockid") ?? null;
                return [{ rect: unionRect(el), blockId, inPanel: anchor.closest(PRIVATE_SELECTOR) != null }];
            }),
        meta: (blockId) => MOS.getObjectValue<Block>(MOS.makeORef("block", blockId))?.meta as Record<string, unknown> | undefined,
    };
}

function windowDeps(): MaskBridgeDeps {
    return {
        now: () => Date.now(),
        windowId: () => atoms.muxWindow()?.oid,
        title: () => document.title,
        read: () => measure(windowSource()),
        nextFrame: (cb) => requestAnimationFrame(() => cb()),
        refusal: () => (dragging ? "a pane is being dragged" : openListRefusal(document.activeElement)),
        async answer(captureId, phase, mask) {
            await RpcApi.CaptureMaskReportCommand(TabRpcClient, { capture_id: captureId, phase, mask });
        },
    };
}

export function startCaptureMasks(deps?: MaskBridgeDeps): void {
    const watches = new Map<string, CaptureWatch>();
    const d = deps ?? windowDeps();
    if (!deps && typeof document !== "undefined") {
        document.addEventListener("dragstart", () => (dragging = true), true);
        for (const end of ["dragend", "drop"]) document.addEventListener(end, () => (dragging = false), true);
    }
    onMuxEvent(WpsEvent.CaptureMask, (ev: CaptureMaskEvent) => void answerAsk(ev, d, watches).catch((e) => console.log("capture mask: answer failed", e)));
}
