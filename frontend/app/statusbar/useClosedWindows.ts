// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, on, onCleanup, type Accessor } from "solid-js";
import { WindowService, type ClosedWindow } from "@/store/services";

/** How often the open panel asks srv for the closed windows, and how soon it asks again after a window opens or closes. */
const CLOSED_POLL_MS = 2000;
const CLOSED_RECHECK_MS = 600;
const CLOSED_REQUEST_TIMEOUT_MS = 10_000;

/** `next`, with each window that did not change replaced by the object it was, so a row that did not change is not rebuilt. */
export function keepUnchanged(prev: ClosedWindow[], next: ClosedWindow[]): ClosedWindow[] {
    const before = new Map(prev.map((w) => [w.workspaceid, w]));
    const merged = next.map((w) => {
        const was = before.get(w.workspaceid);
        return was && JSON.stringify(was) === JSON.stringify(w) ? was : w;
    });
    return merged.length === prev.length && merged.every((w, i) => w === prev[i]) ? prev : merged;
}

/**
 * The workspace's closed windows (srv `session_closed.rs`), asked again whenever a window opens or closes,
 * and after a reopen or delete. `placeSlots` runs after each answer.
 */
export function useClosedWindows(openCount: () => number, placeSlots: () => void): { closed: Accessor<ClosedWindow[]>; refreshClosed: () => Promise<void> } {
    const [closed, setClosed] = createSignal<ClosedWindow[]>([]);
    let listing = false;
    let askedAgain = false;
    const refreshClosed = (): Promise<void> => {
        // One request at a time. One asked for meanwhile (after a reopen or a remove, say) runs once this
        // one settles, since this one's answer may be from before it.
        if (listing) {
            askedAgain = true;
            return Promise.resolve();
        }
        listing = true;
        let timer: number | undefined;
        // A request that never settles must not stop the ones after it.
        const timeout = new Promise<never>((_, reject) => {
            timer = window.setTimeout(() => reject(new Error("timed out")), CLOSED_REQUEST_TIMEOUT_MS);
        });
        return Promise.race([WindowService.ListClosedWindows(), timeout])
            .then((list) => {
                setClosed((prev) => keepUnchanged(prev, list ?? []));
                placeSlots();
            })
            .catch((e) => console.warn("[InstancePanel] couldn't list the closed windows:", e))
            .finally(() => {
                window.clearTimeout(timer);
                listing = false;
                if (askedAgain) {
                    askedAgain = false;
                    void refreshClosed();
                }
            });
    };
    // A window opening or closing changes the list, but srv may not have recorded a close yet when the
    // host reports it: ask again shortly after.
    createEffect(
        on(openCount, () => {
            void refreshClosed();
            const later = [CLOSED_RECHECK_MS, CLOSED_RECHECK_MS * 3].map((ms) => window.setTimeout(() => void refreshClosed(), ms));
            onCleanup(() => later.forEach((t) => window.clearTimeout(t)));
        })
    );
    // What else changes a closed window's row (its agents resuming or stopping, a rename) has no event of its
    // own, so while the panel is open it is asked for on a short timer, not only when it opens.
    const pollTimer = window.setInterval(() => {
        if (!document.hidden) void refreshClosed();
    }, CLOSED_POLL_MS);
    onCleanup(() => window.clearInterval(pollTimer));
    return { closed, refreshClosed };
}
