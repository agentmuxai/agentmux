// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Window drag follows HostCaps.nativeWindowChrome: a host without native
 * window chrome never gets the host drag listener (a double-click on the
 * drag region never asks the host to maximize), and on Linux/macOS gets no
 * drag-region marker either. The Windows hook always emits the marker, as it
 * did before. docs/specs/SPEC_HOST_API_SEAM_2026_09_26.md §5, slice 4.
 *
 * The listeners attach to `document` and outlive a test, so every no-chrome
 * case runs before any CEF case installs one.
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import { CEF_HOST_CAPS } from "@/app/host/host-caps";
import { makeTestHostApi } from "@/app/host/test-host";

const PLATFORMS = ["linux", "darwin", "win32"] as const;

async function load(platform: (typeof PLATFORMS)[number]) {
    return (await import(`./useWindowDrag.${platform}.ts`)) as typeof import("./useWindowDrag.linux");
}

function doubleClickDragRegion(): void {
    const region = document.createElement("div");
    region.setAttribute("data-drag-region", "true");
    document.body.appendChild(region);
    region.dispatchEvent(new MouseEvent("dblclick", { button: 0, bubbles: true }));
    region.remove();
}

afterEach(() => {
    vi.resetModules();
});

describe.each(PLATFORMS)("useWindowDrag (%s) on a host without native window chrome", (platform) => {
    it("installs no host drag listener", async () => {
        const maximize = vi.fn(() => Promise.resolve());
        window.api = makeTestHostApi({ windows: { maximize } as unknown as WindowHostApi });
        const { useWindowDrag } = await load(platform);
        const { dragProps } = useWindowDrag();
        if (platform !== "win32") expect(dragProps).toEqual({});
        doubleClickDragRegion();
        expect(maximize).not.toHaveBeenCalled();
    });
});

describe.each(["linux", "darwin"] as const)("useWindowDrag (%s) on the CEF host", (platform) => {
    it("marks the drag region and maximizes on double-click", async () => {
        const maximize = vi.fn(() => Promise.resolve());
        window.api = makeTestHostApi({ windows: { maximize } as unknown as WindowHostApi }, CEF_HOST_CAPS);
        const { useWindowDrag } = await load(platform);
        expect(useWindowDrag().dragProps).toEqual({ "data-drag-region": true });
        doubleClickDragRegion();
        expect(maximize).toHaveBeenCalled();
    });
});
