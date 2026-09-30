// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { openPendingFiles } from "./pending-open-files";

describe("openPendingFiles — one reuse batch", () => {
    it("opens every file in order and shows the pane once", () => {
        const open = vi.fn();
        const show = vi.fn();
        expect(openPendingFiles(["C:/a.md", "C:/b.md"], open, show)).toEqual(["C:/a.md", "C:/b.md"]);
        expect(open.mock.calls).toEqual([["C:/a.md"], ["C:/b.md"]]);
        expect(show).toHaveBeenCalledTimes(1);
    });

    it("shows the pane after the files are opened", () => {
        const order: string[] = [];
        openPendingFiles(["C:/a.md"], (p) => order.push(`open:${p}`), () => order.push("show"));
        expect(order).toEqual(["open:C:/a.md", "show"]);
    });

    it("skips non-string and empty entries, and doesn't show the pane when nothing was opened", () => {
        const open = vi.fn();
        const show = vi.fn();
        expect(openPendingFiles([null, "", 3, {}], open, show)).toEqual([]);
        expect(open).not.toHaveBeenCalled();
        expect(show).not.toHaveBeenCalled();
    });
});
