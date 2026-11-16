// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A passive overlay (a toast) tells the host it is one, in the clip it sends, so the host leaves the page its
// keyboard and the mouse outside the toast. A menu or a dialog sends no such mark and is treated as before.

import { makeTestHostApi } from "@/app/host/test-host";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

vi.mock("@/app/platform/ipc", () => ({ invokeCommand: vi.fn(() => Promise.resolve()) }));

import { registerPaneOverlay } from "./pane-overlay";
import { __resetPaneRectRegistry, registerPaneRect } from "./pane-rect-registry";

type Sent = { x: number; y: number; w: number; h: number; passive?: boolean }[];

const settle = (ms = 40) => new Promise((r) => setTimeout(r, ms));

function makeEl(left: number, top: number): HTMLElement {
    const el = document.createElement("div");
    el.getBoundingClientRect = () =>
        ({ left, top, width: 50, height: 50, x: left, y: top, right: left + 50, bottom: top + 50, toJSON: () => ({}) }) as DOMRect;
    document.body.appendChild(el);
    return el;
}

let setOverlayClip: ReturnType<typeof vi.fn>;
/** The rect lists the host received, in order. */
const sent = (): Sent[] => setOverlayClip.mock.calls.map((c) => c[0] as Sent);

beforeEach(() => {
    setOverlayClip = vi.fn(() => Promise.resolve());
    window.api = makeTestHostApi({ browserPanes: { setOverlayClip } as unknown as BrowserPaneHostApi });
    registerPaneRect("pane-a", { x: 0, y: 0, w: 400, h: 400 });
});

afterEach(async () => {
    document.body.innerHTML = "";
    await settle();
    __resetPaneRectRegistry();
});

describe("the clip the host receives", () => {
    test("a passive overlay is marked passive; a plain one and a keys one are not", async () => {
        const toast = registerPaneOverlay(makeEl(10, 10), { passive: true });
        const dialog = registerPaneOverlay(makeEl(100, 100));
        const menu = registerPaneOverlay(makeEl(200, 200), { keys: true });
        await settle();
        const last = sent().at(-1)!;
        expect(last).toHaveLength(3);
        expect(last.map((r) => r.passive === true)).toEqual([true, false, false]);
        // Only the mark is added: no `passive: false` for the others.
        expect(last[1]).not.toHaveProperty("passive");
        expect(last[2]).not.toHaveProperty("passive");
        toast.release();
        dialog.release();
        menu.release();
    });

    test("an overlay that asks for keys is never passive, whatever else it asks", async () => {
        const h = registerPaneOverlay(makeEl(10, 10), { keys: true, passive: true });
        await settle();
        expect(sent().at(-1)![0]).not.toHaveProperty("passive");
        h.release();
    });

    test("the passive mark goes with its own rect when the others come and go", async () => {
        const toast = registerPaneOverlay(makeEl(10, 10), { passive: true });
        await settle();
        const dialog = registerPaneOverlay(makeEl(100, 100));
        await settle();
        expect(sent().at(-1)!.map((r) => r.passive === true)).toEqual([true, false]);
        toast.release();
        await settle();
        // The dialog is left alone, and is not marked passive for the toast that went before it.
        expect(sent().at(-1)!.map((r) => r.passive === true)).toEqual([false]);
        dialog.release();
    });

    test("a toast that turns into a plain overlay's rect is a different clip, not a duplicate", async () => {
        const toast = registerPaneOverlay(makeEl(10, 10), { passive: true });
        await settle();
        toast.release();
        const plain = registerPaneOverlay(makeEl(10, 10));
        await settle();
        const calls = sent().filter((c) => c.length > 0);
        expect(calls.at(-1)![0]).not.toHaveProperty("passive");
        expect(calls.some((c) => c[0].passive === true)).toBe(true);
        plain.release();
    });
});
