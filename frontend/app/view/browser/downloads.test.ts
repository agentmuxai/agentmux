// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import {
    applyDownloadUpdate,
    dismissDownload,
    progressText,
    type DownloadRow,
    type DownloadUpdate,
} from "./downloads";

const upd = (over: Partial<DownloadUpdate>): DownloadUpdate => ({
    block_id: "blk",
    download_id: 1,
    name: "report.pdf",
    state: "running",
    received: 4_400_000,
    total: 18_000_000,
    ...over,
});

describe("applyDownloadUpdate", () => {
    it("adds a row for a new download and updates it in place", () => {
        let rows = applyDownloadUpdate([], upd({}));
        expect(rows).toHaveLength(1);
        expect(rows[0]).toMatchObject({ key: "1", id: 1, name: "report.pdf", state: "running" });
        rows = applyDownloadUpdate(rows, upd({ received: 9_000_000 }));
        expect(rows).toHaveLength(1);
        expect(rows[0].received).toBe(9_000_000);
        rows = applyDownloadUpdate(rows, upd({ download_id: 2, name: "b.zip" }));
        expect(rows.map((r) => r.key)).toEqual(["1", "2"]);
    });

    it("keeps the path for a done download and the reason for a failed one only", () => {
        const done = applyDownloadUpdate([], upd({ state: "done", path: "C:/Users/me/Downloads/report.pdf" }));
        expect(done[0].path).toBe("C:/Users/me/Downloads/report.pdf");
        const failed = applyDownloadUpdate([], upd({ state: "failed", reason: "The download failed: disk full." }));
        expect(failed[0].reason).toBe("The download failed: disk full.");
        expect(failed[0].path).toBeUndefined();
        const cancelled = applyDownloadUpdate(done, upd({ state: "cancelled" }));
        expect(cancelled[0].path).toBeUndefined();
        expect(cancelled[0].state).toBe("cancelled");
    });

    it("does not bring a finished row back with a late running update", () => {
        const done = applyDownloadUpdate([], upd({ state: "done", path: "/x/report.pdf" }));
        expect(applyDownloadUpdate(done, upd({ state: "running" }))).toBe(done);
    });

    it("shows the flood line once, tied to no download", () => {
        const line = upd({
            download_id: null,
            state: "ignored",
            reason: "This page tried to start more downloads. Ignored.",
        });
        let rows = applyDownloadUpdate([], line);
        rows = applyDownloadUpdate(rows, line);
        expect(rows).toHaveLength(1);
        expect(rows[0]).toMatchObject({ key: "ignored", id: null, state: "ignored" });
        expect(rows[0].reason).toBe("This page tried to start more downloads. Ignored.");
    });

    it("ignores an update with no download id that isn't the flood line", () => {
        const rows: DownloadRow[] = [];
        expect(applyDownloadUpdate(rows, upd({ download_id: null }))).toBe(rows);
    });
});

describe("dismissDownload", () => {
    it("removes only the named row", () => {
        let rows = applyDownloadUpdate([], upd({}));
        rows = applyDownloadUpdate(rows, upd({ download_id: 2 }));
        expect(dismissDownload(rows, "1").map((r) => r.key)).toEqual(["2"]);
        expect(dismissDownload(rows, "nope")).toHaveLength(2);
    });
});

describe("progress words", () => {
    it("says `of` only when the total is known", () => {
        const [row] = applyDownloadUpdate([], upd({ received: 4_404_019, total: 18_874_368 }));
        expect(progressText(row)).toBe("4.2 MB of 18.0 MB");
        const [unknown] = applyDownloadUpdate([], upd({ received: 4_404_019, total: -1 }));
        expect(progressText(unknown)).toBe("4.2 MB");
    });
});
