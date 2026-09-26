// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Windows' legacy JS-driven drag (localStorage agentmux.win32NativeDrag = "0"):
 * double-clicking the title bar maximizes the window that was clicked. It
 * used to send maximize_window with no label, which the host resolves to
 * "main", so a torn-off window's double-click maximized the main window.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import { CEF_HOST_CAPS } from "@/app/host/host-caps";
import { makeTestHostApi } from "@/app/host/test-host";

afterEach(() => {
    localStorage.removeItem("agentmux.win32NativeDrag");
    history.replaceState(null, "", "/");
});

describe("useWindowDrag (win32, legacy JS drag)", () => {
    it("double-click maximizes this window, not main", async () => {
        localStorage.setItem("agentmux.win32NativeDrag", "0");
        history.replaceState(null, "", "/?windowLabel=floater-7");
        const maximize = vi.fn(() => Promise.resolve());
        window.api = makeTestHostApi({ windows: { maximize } as unknown as WindowHostApi }, CEF_HOST_CAPS);

        const { useWindowDrag } = await import("./useWindowDrag.win32");
        useWindowDrag();

        const region = document.createElement("div");
        region.setAttribute("data-drag-region", "true");
        document.body.appendChild(region);
        region.dispatchEvent(new MouseEvent("dblclick", { button: 0, bubbles: true }));
        region.remove();

        expect(maximize).toHaveBeenCalledWith("floater-7");
    });
});
