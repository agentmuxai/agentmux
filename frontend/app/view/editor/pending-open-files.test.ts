// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { openPendingFiles, pendingEntryKey } from "./pending-open-files";

describe("openPendingFiles — one reuse batch", () => {
    it("opens every file in order and shows the pane once", () => {
        const open = vi.fn();
        const show = vi.fn();
        expect(openPendingFiles(["C:/a.md", "C:/b.md"], open, show)).toEqual(["C:/a.md", "C:/b.md"]);
        expect(open.mock.calls).toEqual([
            ["C:/a.md", { followFolder: false }],
            ["C:/b.md", { followFolder: false }],
        ]);
        expect(show).toHaveBeenCalledTimes(1);
    });

    it("shows the pane after the files are opened", () => {
        const order: string[] = [];
        openPendingFiles(["C:/a.md"], (p) => order.push(`open:${p}`), () => order.push("show"));
        expect(order).toEqual(["open:C:/a.md", "show"]);
    });

    it("skips entries that aren't files, and doesn't show the pane when nothing was opened", () => {
        const open = vi.fn();
        const show = vi.fn();
        expect(openPendingFiles([null, "", 3, {}, { path: "" }], open, show)).toEqual([]);
        expect(open).not.toHaveBeenCalled();
        expect(show).not.toHaveBeenCalled();
    });

    it("a media open's entry follows the folder (the retired Media pane's way in)", () => {
        const open = vi.fn();
        const entry = { path: "C:/renders/out.png", follow_folder: true };
        expect(openPendingFiles([entry], open, () => {})).toEqual([entry]);
        expect(open).toHaveBeenCalledWith("C:/renders/out.png", { followFolder: true });
    });

    it("an entry naming the text viewer opens in it (muxsh edit), whatever the file is", () => {
        const open = vi.fn();
        openPendingFiles([{ path: "C:/shots/a.png", viewer: "text" }, { path: "C:/b.png", viewer: "nope" }], open, () => {});
        expect(open.mock.calls).toEqual([
            ["C:/shots/a.png", { followFolder: false, viewer: "text" }],
            ["C:/b.png", { followFolder: false }],
        ]);
        expect(open.mock.calls[1][1]).not.toHaveProperty("viewer");
    });

    it("tells an object entry apart across reads of the queue", () => {
        expect(pendingEntryKey({ path: "C:/a.png", follow_folder: true })).toBe(pendingEntryKey({ path: "C:/a.png", follow_folder: true }));
        expect(pendingEntryKey("C:/a.png")).not.toBe(pendingEntryKey({ path: "C:/a.png", follow_folder: true }));
    });
});
