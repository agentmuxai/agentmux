// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The downloads a browser pane's page started, as the strip under the address
// bar shows them (docs/specs/SPEC_BROWSER_PANE_DOWNLOADS_2026_10_14.md §5).
// Plain data and functions, so the states are tested without a pane. The host
// sends `pane-download-update` to the window that owns the pane; nothing in
// here comes from the page.

import { formatBytes } from "@/util/format-bytes";

export type DownloadState = "running" | "done" | "failed" | "cancelled";

/** The host's `pane-download-update` payload (crates/cef/src/client/downloads.rs). */
export type DownloadUpdate = {
    block_id: string;
    /** null for the "ignored" line, which is about no one download. */
    download_id: number | null;
    name?: string;
    state: DownloadState | "ignored";
    received?: number;
    /** -1 when the server didn't say. */
    total?: number;
    /** Done only: the file the user chose, from the host. */
    path?: string;
    /** Failed, or the "ignored" line: words to show. */
    reason?: string;
};

export type DownloadRow = {
    /** The host's download id, or "ignored" for the flood line. */
    key: string;
    id: number | null;
    name: string;
    state: DownloadState | "ignored";
    received: number;
    total: number;
    path?: string;
    reason?: string;
};

/** How long a finished download's row stays in the strip before it folds away. */
export const DONE_ROW_MS = 20_000;

const IGNORED_KEY = "ignored";

/** The row a host update makes or changes; every row not named stays as it was. */
export function applyDownloadUpdate(rows: DownloadRow[], u: DownloadUpdate): DownloadRow[] {
    if (u.state === "ignored") {
        const row: DownloadRow = {
            key: IGNORED_KEY,
            id: null,
            name: "",
            state: "ignored",
            received: 0,
            total: -1,
            reason: u.reason,
        };
        return rows.some((r) => r.key === IGNORED_KEY)
            ? rows.map((r) => (r.key === IGNORED_KEY ? row : r))
            : [...rows, row];
    }
    if (u.download_id == null) return rows;
    const key = String(u.download_id);
    const prev = rows.find((r) => r.key === key);
    // A late "running" after a final state (events can arrive out of order)
    // must not bring a finished row back to life.
    if (prev && prev.state !== "running" && u.state === "running") return rows;
    const row: DownloadRow = {
        key,
        id: u.download_id,
        name: u.name || prev?.name || "download",
        state: u.state,
        received: u.received ?? prev?.received ?? 0,
        total: u.total ?? prev?.total ?? -1,
        path: u.state === "done" ? u.path : undefined,
        reason: u.state === "failed" ? u.reason : undefined,
    };
    return prev ? rows.map((r) => (r.key === key ? row : r)) : [...rows, row];
}

export function dismissDownload(rows: DownloadRow[], key: string): DownloadRow[] {
    return rows.filter((r) => r.key !== key);
}

/** The progress words: `4.2 MB of 18.0 MB`, or `4.2 MB` when the total is unknown. */
export function progressText(row: DownloadRow): string {
    if (row.total > 0) return `${formatBytes(row.received)} of ${formatBytes(row.total)}`;
    return formatBytes(row.received);
}
