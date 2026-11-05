// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The fill pass of a transcript scroller (follow-controller.ts `FillOlder`): how many pages of
 * older history one stretch of "too short to scroll" may load, and when a new stretch starts.
 * Pure of the DOM: follow-scroll.ts feeds it the geometry it observes and the effects it runs.
 */

import { FILL_SCREENS, type ScrollGeometry } from "./follow-controller";

/**
 * Pages one fill pass loads before it stops. A pass ends when the content is tall enough, and
 * starts over when the viewport grows; past this the reader asks (`fillStalled`), so a history of
 * rows too small to fill a screen can't keep the loader running.
 */
export const FILL_MAX_PAGES = 8;

export interface FillPassHost {
    /** Page in older history because the content is too short to scroll into. */
    fillOlder?(geo: ScrollGeometry): void;
    /** A pass ended short (`true`: it hit FILL_MAX_PAGES with the content still short) or started again (`false`). */
    fillStalled?(stalled: boolean): void;
}

export interface FillPass {
    /** A FillOlder effect: load a page, or give up when the pass has used its pages. */
    run(geo: ScrollGeometry): void;
    /** A size observation: tall enough ends the pass, a taller viewport moves the target and starts a new one. */
    observed(geo: ScrollGeometry): void;
    /** Begin a new pass (the reader asked for more, or the transcript was cleared). */
    restart(): void;
}

export function createFillPass(host: FillPassHost): FillPass {
    let pages = 0;
    let gaveUp = false;
    let lastClientHeight = 0;
    const setGaveUp = (next: boolean): void => {
        if (next === gaveUp) return;
        gaveUp = next;
        host.fillStalled?.(next);
    };
    const restart = (): void => {
        pages = 0;
        setGaveUp(false);
    };
    return {
        run(geo) {
            if (pages >= FILL_MAX_PAGES) return setGaveUp(true);
            pages++;
            host.fillOlder?.(geo);
        },
        observed(geo) {
            if (geo.scrollHeight >= geo.clientHeight * FILL_SCREENS || geo.clientHeight > lastClientHeight) restart();
            lastClientHeight = geo.clientHeight;
        },
        restart,
    };
}
