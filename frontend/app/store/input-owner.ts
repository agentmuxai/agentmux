// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The frontend's view of the window's input owner
 * (docs/specs/SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md §3.6).
 *
 * `document.hasFocus()` is false whenever one of the window's browser panes has
 * the keyboard: the page's own document isn't focused, though the user is in
 * this window. The host's input owner knows better, and says so with
 * `input-owner-changed` whenever whether the window is active, or who has its
 * keyboard, changes. `userInThisWindow()` combines the two; where the host
 * doesn't track it (macOS and Linux until Phase 4) it is `document.hasFocus()`.
 */

import { getApi } from "@/app/store/global";
import { createSignal } from "solid-js";

const [report, setReport] = createSignal<InputOwnerReport | null>(null);
const listeners = new Set<() => void>();

/** What the host last said about this window's input, or null. Reactive. */
export const inputOwnerReport = report;

/** True while one of this window's browser panes has the keyboard and the
 *  window is active, per the host. Reactive. */
export function paneHasTheKeys(): boolean {
    const r = report();
    return r != null && r.active && r.keys === "pane";
}

/**
 * Whether the user is in this window: its page has focus, or one of its
 * browser panes has the keyboard. Use it where `document.hasFocus()` would
 * read "a pane has the keys" as "the user is elsewhere".
 */
export function userInThisWindow(): boolean {
    if (typeof document === "undefined") return true;
    if (typeof document.hasFocus !== "function" || document.hasFocus()) return true;
    return paneHasTheKeys();
}

/** Call `listener` whenever the host's report changes. Returns the unsubscribe. */
export function onInputOwnerChange(listener: () => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
}

function apply(next: InputOwnerReport | null): void {
    setReport(next);
    for (const listener of listeners) listener();
}

/** Follow the host's reports for this window, starting from its current state. */
export function registerInputOwnerTracking(): void {
    const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
    void getApi().listen<InputOwnerReport>("input-owner-changed", (payload) => apply(payload ?? null));
    getApi()
        .inputOwnerState(label)
        .then((current) => {
            // A report that arrived while the query was out is newer.
            if (report() == null) apply(current);
        })
        .catch(() => {});
}
