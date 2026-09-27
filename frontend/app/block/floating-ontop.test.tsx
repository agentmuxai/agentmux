// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Floating-pane "Always on top" — the header tack.
 * docs/specs/SPEC_FLOATING_PANE_ALWAYS_ON_TOP_2026_09_27.md §4–§6.
 *
 * The block meta `pane:floating_ontop` is the source of truth: the button
 * reflects it and asks the host to change it; a (re)loaded floater re-applies
 * a stored `true`.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ windows: true }));
vi.mock("@/util/platformutil", async (orig) => ({
    ...(await orig<typeof import("@/util/platformutil")>()),
    isWindows: () => h.windows,
}));

import { CEF_HOST_CAPS } from "@/app/host/host-caps";
import { makeTestHostApi } from "@/app/host/test-host";
import * as MOS from "@/app/store/mos";
import {
    canTackFloatingPane,
    FloatingAlwaysOnTopButton,
    isFloatingOnTop,
    useReapplyFloatingOnTop,
} from "./floating-ontop";

const realApi = window.api;
let setOnTop: ReturnType<typeof vi.fn>;
let seq = 0;

/** A fresh block id per test, seeded into the object store. */
function seedBlock(meta: Record<string, unknown>): string {
    const oid = `blk-ontop-${++seq}`;
    MOS.updateMuxObject({
        updatetype: "update",
        otype: "block",
        oid,
        obj: { otype: "block", oid, version: 1, meta } as unknown as Block,
    } as MuxObjUpdate);
    return oid;
}

beforeEach(() => {
    h.windows = true;
    setOnTop = vi.fn(() => Promise.resolve());
    window.api = makeTestHostApi(
        { windows: { setFloatingAlwaysOnTop: setOnTop } as unknown as WindowHostApi },
        CEF_HOST_CAPS
    );
});
afterEach(() => {
    cleanup();
    window.api = realApi;
});

describe("isFloatingOnTop", () => {
    it("is true only for a stored boolean true", () => {
        expect(isFloatingOnTop({ "pane:floating_ontop": true } as MetaType)).toBe(true);
        expect(isFloatingOnTop({ "pane:floating_ontop": false } as MetaType)).toBe(false);
        expect(isFloatingOnTop({ "pane:floating_ontop": "true" } as unknown as MetaType)).toBe(false);
        expect(isFloatingOnTop({} as MetaType)).toBe(false);
        expect(isFloatingOnTop(null)).toBe(false);
    });
});

describe("canTackFloatingPane", () => {
    it("only in a floater, on a host with the capability, on Windows (Phase 1)", () => {
        expect(canTackFloatingPane("floating-1")).toBe(true);
        expect(canTackFloatingPane(null)).toBe(false);
        h.windows = false;
        expect(canTackFloatingPane("floating-1")).toBe(false);
        h.windows = true;
        window.api = makeTestHostApi({}, { ...CEF_HOST_CAPS, floatingAlwaysOnTop: false });
        expect(canTackFloatingPane("floating-1")).toBe(false);
    });
});

describe("FloatingAlwaysOnTopButton", () => {
    it("shows the tacked state in the theme color and asks the host to untack", () => {
        const blockId = seedBlock({ view: "term", "pane:floating_ontop": true });
        const { container } = render(() => <FloatingAlwaysOnTopButton label="floating-7" blockId={blockId} />);
        const btn = container.querySelector("button")!;

        expect(btn.classList.contains("toggle")).toBe(true);
        expect(btn.classList.contains("active")).toBe(true);
        expect(btn.classList.contains("block-frame-ontop")).toBe(true);
        expect(btn.getAttribute("title")).toBe("Always on top (Active)");
        expect(btn.querySelector("i")!.className).toContain("fa-thumbtack");

        btn.click();
        expect(setOnTop).toHaveBeenCalledWith("floating-7", blockId, false);
    });

    it("is off when the meta is absent and tacks on click", () => {
        const blockId = seedBlock({ view: "term" });
        const { container } = render(() => <FloatingAlwaysOnTopButton label="floating-8" blockId={blockId} />);
        const btn = container.querySelector("button")!;

        expect(btn.classList.contains("active")).toBe(false);
        expect(btn.getAttribute("title")).toBe("Always on top");
        btn.click();
        expect(setOnTop).toHaveBeenCalledWith("floating-8", blockId, true);
    });

    it("follows the meta when the host's write-through lands", () => {
        const blockId = seedBlock({ view: "term" });
        const { container } = render(() => <FloatingAlwaysOnTopButton label="floating-9" blockId={blockId} />);
        const btn = container.querySelector("button")!;
        expect(btn.classList.contains("active")).toBe(false);

        MOS.updateMuxObject({
            updatetype: "update",
            otype: "block",
            oid: blockId,
            obj: { otype: "block", oid: blockId, version: 2, meta: { view: "term", "pane:floating_ontop": true } },
        } as unknown as MuxObjUpdate);

        expect(btn.classList.contains("active")).toBe(true);
    });
});

describe("useReapplyFloatingOnTop", () => {
    function mount(label: string, blockId: () => string | undefined) {
        return createRoot((dispose) => {
            useReapplyFloatingOnTop(() => label, blockId);
            return dispose;
        });
    }

    it("re-applies a stored tack to the (fresh) window, once", async () => {
        const blockId = seedBlock({ view: "term", "pane:floating_ontop": true });
        const dispose = mount("floating-3", () => blockId);
        await Promise.resolve();
        expect(setOnTop).toHaveBeenCalledTimes(1);
        expect(setOnTop).toHaveBeenCalledWith("floating-3", blockId, true);

        // A later meta change (the user untacks) is not a reload — no re-apply.
        MOS.updateMuxObject({
            updatetype: "update",
            otype: "block",
            oid: blockId,
            obj: { otype: "block", oid: blockId, version: 2, meta: { view: "term" } },
        } as unknown as MuxObjUpdate);
        await Promise.resolve();
        expect(setOnTop).toHaveBeenCalledTimes(1);
        dispose();
    });

    it("does nothing for an untacked pane", async () => {
        const blockId = seedBlock({ view: "term" });
        const dispose = mount("floating-4", () => blockId);
        await Promise.resolve();
        expect(setOnTop).not.toHaveBeenCalled();
        dispose();
    });

    it("does nothing where the tack isn't available", async () => {
        h.windows = false;
        const blockId = seedBlock({ view: "term", "pane:floating_ontop": true });
        const dispose = mount("floating-5", () => blockId);
        await Promise.resolve();
        expect(setOnTop).not.toHaveBeenCalled();
        dispose();
    });
});
