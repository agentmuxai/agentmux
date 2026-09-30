// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A picture of a pane, so its floating window can show the pane the moment
 * the window appears instead of the startup splash, until its live content
 * reveals. SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md phase 3.1.
 *
 * Taken on pointerdown on the pane's drag handle, before the drag starts: a
 * dragged pane is drawn blurred (tilelayout.scss `.dragging`), and a picture
 * taken at release would add its capture time to the tear-off. The browser
 * pane's drag catcher prewarms the same way (use-drag-snapshot.ts).
 */

import { getApi } from "@/app/store/global";
import { Logger } from "@/util/logger";

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

let held: { blockId: string; at: number; picture: Promise<string | null> } | null = null;

/** Start capturing `blockId` (a base64 JPEG), replacing any earlier picture. */
export function prewarmTearOffSnapshot(blockId: string): void {
    const at = Date.now();
    const picture = getApi()
        .browserPanes.screenshot(blockId, { format: "jpeg", quality: 80 })
        .then(
            (r) => {
                Logger.debug("dnd:cross", "tear-off snapshot captured", { blockId, ms: Date.now() - at });
                return r?.png_base64 || null;
            },
            // Expected for a pane with nothing to capture, e.g. a background
            // pane tab has no element on screen.
            () => null
        );
    held = { blockId, at, picture };
}

/**
 * The picture of `blockId` taken for this drag, if it's ready within the
 * budget. Never starts a capture: that would delay the tear-off itself.
 */
export async function takeTearOffSnapshot(blockId: string): Promise<string | undefined> {
    const h = held;
    held = null;
    if (!h || h.blockId !== blockId || Date.now() - h.at > MAX_AGE_MS) return undefined;
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
}
