// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A pane's past downloads, for the Downloads menu in the address bar
// (docs/specs/SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md §5). The strip under the address bar shows a download
// while it runs and for a short while after; this list keeps each one that ended (saved, failed or cancelled)
// until the pane closes, so a file saved a while ago can still be found. Only what the host reported goes in
// it, and the paths are the host's own.

import { createSignal, type Accessor } from "solid-js";
import { rememberPast, type DownloadRow } from "./downloads";

export type DownloadHistory = {
    past: Accessor<DownloadRow[]>;
    /** Add a download that has just ended; null (nothing ended) does nothing. */
    record: (row: DownloadRow | null) => void;
    clear: () => void;
    /** The file at `path` couldn't be shown: say so on its line. */
    revealFailed: (path: string) => void;
};

export function createDownloadHistory(): DownloadHistory {
    const [past, setPast] = createSignal<DownloadRow[]>([]);
    let seq = 0;
    return {
        past,
        record: (row) => {
            if (row) setPast((rows) => rememberPast(rows, row, ++seq));
        },
        clear: () => setPast([]),
        revealFailed: (path) =>
            setPast((rows) => rows.map((r) => (r.path === path ? { ...r, reason: "Couldn't open the folder." } : r))),
    };
}
