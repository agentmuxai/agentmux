// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The floating window's side of `placefloatingpane`: an agent's `resize`, `move` or `place` of a floating pane
 * it opened, or `Pane layout`'s question where the window is (docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md
 * §2.2, §2.3, §3). srv sends it to the floating window itself, which knows its own monitor, work area and host
 * label. It resolves the request the way an open does, sets the window's rect through the host without raising
 * or activating it (`SetWindowPos` with no activate on Windows, `set_bounds` elsewhere), and answers srv with
 * where it is now (`pane.floatplaced`).
 */
import { workAreaOf, type FloaterWindow } from "@/app/store/open-floating-pane";
import { resolveFloaterRect, type Rect } from "@/app/store/floater-placement";
import type { FloaterPlace } from "@/types/rpc/FloaterPlace";
import type { Placed } from "@/types/rpc/Placed";
import type { PlaceFloatingPaneEvent } from "@/types/rpc/PlaceFloatingPaneEvent";

type Deps = {
    /** This window's rect as the host reports it: physical pixels on Windows, DIP elsewhere. */
    getRect: () => Promise<Rect>;
    /** Set this window's rect, in the same units, without activating it. */
    setRect: (rect: Rect) => Promise<unknown>;
    /** `pane.floatplaced`: tell srv where the window is, for the call that waits on it. */
    answer: (args: { request_id: string; placed?: Placed; error?: string }) => Promise<unknown>;
};

/** Host pixels from screen CSS pixels (physical on Windows, DIP elsewhere), and back. */
function toHost(css: number, w: FloaterWindow): number {
    return Math.round(w.isWindows ? css * (w.devicePixelRatio || 1) : css);
}
function fromHost(host: number, w: FloaterWindow): number {
    return w.isWindows ? host / (w.devicePixelRatio || 1) : host;
}

/** What the request resolves against: a resize keeps the position, a move keeps the size. */
function request(place: FloaterPlace, keep: string | undefined, current: Rect, area: Rect): FloaterPlace {
    if (keep === "position") {
        return { ...place, spot: undefined, x: current.x - area.x, y: current.y - area.y, cascade: 0 };
    }
    if (keep === "size") {
        return { ...place, width: current.width, height: current.height, max_share: undefined, cascade: 0 };
    }
    return { ...place, max_share: undefined, cascade: 0 };
}

function placedOf(r: Rect, area: Rect, w: FloaterWindow, extra: { spot?: string; clamped: boolean }): Placed {
    return {
        x: Math.round(r.x - area.x),
        y: Math.round(r.y - area.y),
        width: Math.round(r.width),
        height: Math.round(r.height),
        spot: extra.spot,
        clamped: extra.clamped,
        display_scale: w.devicePixelRatio || 1,
    };
}

/**
 * Apply `data` to this window, or only report its rect when it carries no `place`, and answer srv. A window
 * the host cannot read answers with the reason, never with a made-up rect.
 */
export async function placeFloatingPane(data: PlaceFloatingPaneEvent, w: FloaterWindow, deps: Deps): Promise<void> {
    const answerError = (e: unknown) => deps.answer({ request_id: data.request_id, error: String(e) });
    let host: Rect;
    try {
        host = await deps.getRect();
    } catch (e) {
        await answerError(e);
        throw e;
    }
    if (!(host.width > 0 && host.height > 0)) {
        // The host reports 0 x 0 for a window it cannot find or read.
        await answerError("the window's size could not be read");
        return;
    }
    const area = workAreaOf(w);
    const current: Rect = {
        x: fromHost(host.x, w),
        y: fromHost(host.y, w),
        width: fromHost(host.width, w),
        height: fromHost(host.height, w),
    };
    if (data.place == null) {
        await deps.answer({ request_id: data.request_id, placed: placedOf(current, area, w, { clamped: false }) });
        return;
    }
    const r = resolveFloaterRect(request(data.place, data.keep, current, area), {
        area,
        fallback: { width: current.width, height: current.height },
        anchor: current,
    });
    try {
        await deps.setRect({ x: toHost(r.left, w), y: toHost(r.top, w), width: toHost(r.width, w), height: toHost(r.height, w) });
    } catch (e) {
        await answerError(e);
        throw e;
    }
    await deps.answer({
        request_id: data.request_id,
        placed: placedOf({ x: r.left, y: r.top, width: r.width, height: r.height }, area, w, { spot: r.spot, clamped: r.clamped }),
    });
}
