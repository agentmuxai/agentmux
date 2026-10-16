// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A floating pane's Maximize / Restore button and the placement sync behind it.
 * docs/specs/SPEC_FLOATING_PANE_WINDOW_CONTROLS_REFINEMENTS_2026_10_16.md §3.2, §4.2.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    setMeta: [] as { oref: string; meta: Record<string, unknown> }[],
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetMetaCommand: async (_client: unknown, data: { oref: string; meta: Record<string, unknown> }) => {
            h.setMeta.push(data);
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { CEF_HOST_CAPS } from "@/app/host/host-caps";
import { makeTestHostApi } from "@/app/host/test-host";
import * as MOS from "@/app/store/mos";
import type { MuxObjUpdate } from "@/types/rpc/MuxObjUpdate";
import { FloatingMaximizeButton, isFloatingMaximized, PLACEMENT_RESIZE_SETTLE_MS } from "./floating-maximize";

const realApi = window.api;
let toggle: ReturnType<typeof vi.fn>;
let hostMaximized = false;
let emit: ((payload: unknown) => void) | null = null;
let seq = 0;

function setBlockMeta(oid: string, meta: Record<string, unknown>, version: number): void {
    MOS.updateMuxObject({
        updatetype: "update",
        otype: "block",
        oid,
        obj: { otype: "block", oid, version, meta } as unknown as Block,
    } as MuxObjUpdate);
}

function seedBlock(meta: Record<string, unknown>): string {
    const oid = `blk-max-${++seq}`;
    setBlockMeta(oid, meta, 1);
    return oid;
}

beforeEach(() => {
    h.setMeta = [];
    hostMaximized = false;
    emit = null;
    toggle = vi.fn(() => Promise.resolve());
    window.api = makeTestHostApi(
        {
            windows: {
                toggleFloatingMaximize: toggle,
                isMaximized: async () => hostMaximized,
            } as unknown as WindowHostApi,
            listen: (async (event: string, handler: (p: unknown) => void) => {
                if (event === "floating_placement_changed") emit = handler;
                return () => {
                    emit = null;
                };
            }) as unknown as AppApi["listen"],
        },
        CEF_HOST_CAPS
    );
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    window.api = realApi;
});

describe("isFloatingMaximized", () => {
    it("is true only for the exact stored string", () => {
        expect(isFloatingMaximized({ "pane:floating_placement": "maximized" } as MetaType)).toBe(true);
        expect(isFloatingMaximized({ "pane:floating_placement": "normal" } as MetaType)).toBe(false);
        expect(isFloatingMaximized({} as MetaType)).toBe(false);
        expect(isFloatingMaximized(null)).toBe(false);
    });
});

describe("FloatingMaximizeButton", () => {
    it("shows Maximize for a normal floater and asks the host to toggle", () => {
        const blockId = seedBlock({ view: "term" });
        const { container } = render(() => <FloatingMaximizeButton label="floating-1" blockId={blockId} />);
        const btn = container.querySelector("button")!;
        expect(btn.getAttribute("title")).toBe("Maximize");
        expect(btn.querySelector("i")!.className).toContain("fa-window-maximize");
        expect(btn.classList.contains("block-frame-magnify")).toBe(true);
        btn.click();
        expect(toggle).toHaveBeenCalledWith("floating-1", blockId);
    });

    it("shows Restore, the two-windows glyph, while maximized, and the click still toggles", () => {
        const blockId = seedBlock({ view: "term", "pane:floating_placement": "maximized" });
        const { container } = render(() => <FloatingMaximizeButton label="floating-2" blockId={blockId} />);
        const btn = container.querySelector("button")!;
        expect(btn.getAttribute("title")).toBe("Restore");
        expect(btn.querySelector("i")!.className).toContain("fa-window-restore");
        btn.click();
        expect(toggle).toHaveBeenCalledTimes(1);
    });

    it("follows the meta when it changes", () => {
        const blockId = seedBlock({ view: "term" });
        const { container } = render(() => <FloatingMaximizeButton label="floating-3" blockId={blockId} />);
        const btn = container.querySelector("button")!;
        setBlockMeta(blockId, { view: "term", "pane:floating_placement": "maximized" }, 2);
        expect(btn.getAttribute("title")).toBe("Restore");
        setBlockMeta(blockId, { view: "term", "pane:floating_placement": "normal" }, 3);
        expect(btn.getAttribute("title")).toBe("Maximize");
    });
});

describe("placement sync", () => {
    it("stores the placement the host reports for this floater, and ignores other floaters", async () => {
        const blockId = seedBlock({ view: "term", "pane:floating_placement": "maximized" });
        render(() => <FloatingMaximizeButton label="floating-4" blockId={blockId} />);
        await vi.waitFor(() => expect(emit).not.toBeNull());

        emit!({ label: "floating-other", placement: "normal" });
        await Promise.resolve();
        expect(h.setMeta).toEqual([]);

        emit!({ label: "floating-4", placement: "normal" });
        await vi.waitFor(() =>
            expect(h.setMeta).toEqual([
                { oref: `block:${blockId}`, meta: { "pane:floating_placement": "normal" } },
            ])
        );
    });

    it("writes nothing when the report matches what is stored", async () => {
        const blockId = seedBlock({ view: "term", "pane:floating_placement": "maximized" });
        render(() => <FloatingMaximizeButton label="floating-5" blockId={blockId} />);
        await vi.waitFor(() => expect(emit).not.toBeNull());
        emit!({ label: "floating-5", placement: "maximized" });
        await Promise.resolve();
        await Promise.resolve();
        expect(h.setMeta).toEqual([]);
    });

    it("asks the host after a burst of resizes settles, once, and stores a change the OS made", async () => {
        vi.useFakeTimers();
        const blockId = seedBlock({ view: "term", "pane:floating_placement": "maximized" });
        const isMax = vi.spyOn(window.api.windows, "isMaximized");
        render(() => <FloatingMaximizeButton label="floating-6" blockId={blockId} />);
        hostMaximized = false; // e.g. GNOME un-maximized it on a compositor move

        window.dispatchEvent(new Event("resize"));
        vi.advanceTimersByTime(PLACEMENT_RESIZE_SETTLE_MS / 2);
        window.dispatchEvent(new Event("resize"));
        vi.advanceTimersByTime(PLACEMENT_RESIZE_SETTLE_MS);
        vi.useRealTimers();

        await vi.waitFor(() =>
            expect(h.setMeta).toEqual([
                { oref: `block:${blockId}`, meta: { "pane:floating_placement": "normal" } },
            ])
        );
        expect(isMax).toHaveBeenCalledTimes(1);
        expect(isMax).toHaveBeenCalledWith("floating-6");
    });
});
