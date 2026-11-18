// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { createDownloadHistory } from "./download-history";
import {
    applyDownloadUpdate,
    justFinished,
    PAST_MAX,
    pastStatus,
    rememberPast,
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

/** Feed updates the way the model does: apply each, ask what it finished, keep what ended. */
function run(updates: DownloadUpdate[]) {
    const history = createDownloadHistory();
    let rows: DownloadRow[] = [];
    for (const u of updates) {
        const next = applyDownloadUpdate(rows, u);
        history.record(justFinished(rows, next, u));
        rows = next;
    }
    return { history, rows };
}

describe("justFinished", () => {
    it("names the row a final update ended, once", () => {
        const before = applyDownloadUpdate([], upd({}));
        const done = upd({ state: "done", path: "C:/Users/me/Downloads/report.pdf" });
        const after = applyDownloadUpdate(before, done);
        expect(justFinished(before, after, done)).toMatchObject({ name: "report.pdf", state: "done", path: "C:/Users/me/Downloads/report.pdf" });
        // The same end again finishes nothing.
        expect(justFinished(after, applyDownloadUpdate(after, done), done)).toBeNull();
    });

    it("finishes nothing on progress, the flood line, or a download the strip never saw", () => {
        const before = applyDownloadUpdate([], upd({}));
        const next = upd({ received: 9_000_000 });
        expect(justFinished(before, applyDownloadUpdate(before, next), next)).toBeNull();
        const flood = upd({ download_id: null, state: "ignored", reason: "This page tried to start more downloads. Ignored." });
        expect(justFinished(before, applyDownloadUpdate(before, flood), flood)).toBeNull();
        // A download whose first update is already its end (a failure before any progress) did finish.
        const failed = upd({ download_id: 5, state: "failed", reason: "The disk is full." });
        expect(justFinished([], applyDownloadUpdate([], failed), failed)).toMatchObject({ state: "failed", reason: "The disk is full." });
    });
});

describe("the past downloads", () => {
    it("keeps each download that ended, newest first, and not the ones still running", () => {
        const { history } = run([
            upd({ download_id: 1, name: "a.zip" }),
            upd({ download_id: 2, name: "b.zip" }),
            upd({ download_id: 1, name: "a.zip", state: "done", path: "/d/a.zip" }),
            upd({ download_id: 2, name: "b.zip", state: "cancelled" }),
            upd({ download_id: 3, name: "c.zip" }),
        ]);
        expect(history.past().map((r) => [r.name, r.state])).toEqual([
            ["b.zip", "cancelled"],
            ["a.zip", "done"],
        ]);
    });

    it("keeps a row after the strip hides it, and survives the host reusing an id", () => {
        const { history } = run([
            upd({ download_id: 1, name: "a.zip", state: "done", path: "/d/a.zip" }),
            upd({ download_id: 1, name: "again.zip", state: "done", path: "/d/again.zip" }),
        ]);
        // The second "1" is a late repeat for the same strip row, so it is not a second past line...
        expect(history.past()).toHaveLength(1);
        // ...but a pane that goes on to a new profile reuses ids, and the strip's row was dismissed by then.
        const fresh = rememberPast(history.past(), { key: "1", id: 1, name: "again.zip", state: "done", received: 1, total: 1, path: "/d/again.zip" }, 2);
        expect(new Set(fresh.map((r) => r.key)).size).toBe(2);
    });

    it("keeps at most PAST_MAX, dropping the oldest", () => {
        let past: DownloadRow[] = [];
        for (let i = 0; i < PAST_MAX + 5; i++) {
            past = rememberPast(past, { key: String(i), id: i, name: `f${i}`, state: "done", received: 1, total: 1, path: `/d/f${i}` }, i);
        }
        expect(past).toHaveLength(PAST_MAX);
        expect(past[0].name).toBe(`f${PAST_MAX + 4}`);
        expect(past.some((r) => r.name === "f0")).toBe(false);
    });

    it("clears the list and marks a file that could not be shown", () => {
        const { history } = run([upd({ state: "done", path: "/d/a.zip" })]);
        history.revealFailed("/d/a.zip");
        expect(history.past()[0].reason).toBe("Couldn't open the folder.");
        history.clear();
        expect(history.past()).toEqual([]);
    });
});

describe("pastStatus", () => {
    const row = (over: Partial<DownloadRow>): DownloadRow => ({ key: "k", id: 1, name: "a", state: "done", received: 4_400_000, total: 18_000_000, ...over });
    it("says the size saved, cancelled, or why it failed", () => {
        expect(pastStatus(row({}))).toBe("17.2 MB");
        expect(pastStatus(row({ total: -1 }))).toBe("4.2 MB");
        expect(pastStatus(row({ state: "cancelled" }))).toBe("cancelled");
        expect(pastStatus(row({ state: "failed", reason: "The disk is full." }))).toBe("The disk is full.");
        expect(pastStatus(row({ state: "failed" }))).toBe("failed");
        expect(pastStatus(row({ reason: "Couldn't open the folder." }))).toBe("Couldn't open the folder.");
    });
});
