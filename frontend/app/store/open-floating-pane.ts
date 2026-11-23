// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window's side of `openfloatingpane`: srv made the pane and tore it into a workspace of its own, and
 * this window (the one the open came from) opens the chromeless OS window for it through the host. srv
 * cannot open windows. See docs/specs/SPEC_OPENEDITOR_FLOATING_AND_COLLAPSED_TREE_2026_06_16.md and, for an
 * agent's open with a size and a place, docs/specs/SPEC_AGENT_PANE_PLACEMENT_FLOATING_2026_11_22.md §3.
 */
import { resolveFloaterRect, type Rect } from "@/app/store/floater-placement";
import type { OpenFloatingPaneEvent } from "@/types/rpc/OpenFloatingPaneEvent";
import type { Placed } from "@/types/rpc/Placed";

/** What this needs of the window it runs in, so a test can stand one up. */
export type FloaterWindow = {
    screenX: number;
    screenY: number;
    outerWidth: number;
    outerHeight: number;
    innerWidth: number;
    innerHeight: number;
    devicePixelRatio: number;
    /** Chromium's own: the work area of the monitor the window is on, in CSS pixels. */
    screen: { availLeft?: number; availTop?: number; availWidth: number; availHeight: number };
    isWindows: boolean;
};

type Deps = {
    openFloatingPane: (args: Record<string, unknown>) => Promise<unknown>;
    /** `pane.floatplaced`: tell srv where the pane went, for the open that waits on it. */
    answer: (args: { request_id: string; placed?: Placed; error?: string }) => Promise<unknown>;
};

/** The work area of the monitor this window is on. */
export function workAreaOf(w: FloaterWindow): Rect {
    return { x: w.screen.availLeft ?? 0, y: w.screen.availTop ?? 0, width: w.screen.availWidth, height: w.screen.availHeight };
}

/** Screen CSS pixels as the host takes them: physical on Windows, DIP elsewhere (floating_pane.rs). */
function hostCoord(css: number, w: FloaterWindow): number {
    return Math.round(w.isWindows ? css * (w.devicePixelRatio || 1) : css);
}

/** About half the window, centered: where a floating pane goes when nothing says otherwise. */
function defaultSize(w: FloaterWindow): { width: number; height: number } {
    return {
        width: Math.max(600, Math.min(1200, Math.round(w.innerWidth * 0.5))),
        height: Math.max(400, Math.min(900, Math.round(w.innerHeight * 0.6))),
    };
}

/**
 * Open the floating window for `data`. The person's own open (no `place`) opens where it always did and is
 * not answered. An agent's open is resolved against this monitor's work area, shown without taking the
 * foreground, and answered with where it went.
 */
export async function openFloatingPane(data: OpenFloatingPaneEvent, w: FloaterWindow, deps: Deps): Promise<void> {
    const place = data.place;
    const fallback = defaultSize(w);
    const anchor: Rect = { x: w.screenX, y: w.screenY, width: w.outerWidth, height: w.outerHeight };
    if (place == null) {
        await deps.openFloatingPane({
            pane_id: data.block_id,
            workspace_id: data.workspace_id,
            x: hostCoord(anchor.x + Math.max(0, (anchor.width - fallback.width) / 2), w),
            y: hostCoord(anchor.y + Math.max(0, (anchor.height - fallback.height) / 2), w),
            ...fallback,
        });
        return;
    }
    const area = workAreaOf(w);
    const r = resolveFloaterRect(place, { area, fallback, anchor });
    try {
        await deps.openFloatingPane({
            pane_id: data.block_id,
            workspace_id: data.workspace_id,
            x: hostCoord(r.left, w),
            y: hostCoord(r.top, w),
            width: r.width,
            height: r.height,
            activate: place.activate,
        });
    } catch (e) {
        await deps.answer({ request_id: place.request_id, error: String(e) });
        throw e;
    }
    await deps.answer({
        request_id: place.request_id,
        placed: {
            x: r.left - area.x,
            y: r.top - area.y,
            width: r.width,
            height: r.height,
            spot: r.spot,
            clamped: r.clamped,
            display_scale: w.devicePixelRatio || 1,
        },
    });
}

/** The live window this runs in. */
export function liveFloaterWindow(): FloaterWindow {
    return {
        screenX: window.screenX,
        screenY: window.screenY,
        outerWidth: window.outerWidth,
        outerHeight: window.outerHeight,
        innerWidth: window.innerWidth,
        innerHeight: window.innerHeight,
        devicePixelRatio: window.devicePixelRatio || 1,
        screen: window.screen as FloaterWindow["screen"],
        isWindows: /Windows/i.test(navigator.userAgent),
    };
}
