// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { recordIpcRoundtrip } from "./index";

afterEach(() => vi.restoreAllMocks());

describe("recordIpcRoundtrip", () => {
    // A slow-IPC warning is a console line, and console lines go to the host
    // over these commands: warning about them would feed the log pipe itself.
    it.each(["fe_log_structured", "fe_log", "fe_log_batch"])("never warns about the log pipe's own %s", (cmd) => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        recordIpcRoundtrip(cmd, 500);
        expect(warn).not.toHaveBeenCalled();
    });

    it("warns about any other command over a frame", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        recordIpcRoundtrip("browser_panes_set_rects", 40);
        expect(warn).toHaveBeenCalledWith(expect.stringContaining("browser_panes_set_rects"));
    });
});
