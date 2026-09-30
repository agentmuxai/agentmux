// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The redock ghost's two stages: a faint preview as soon as a floater is over
 * this window, solid and a real drop target once the dwell arms it.
 * SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md §10.
 */

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { REDOCK_DWELL_MS } from "./floating-pane-constants";

type HoverPayload = { target_label: string | null; cursor_x?: number; cursor_y?: number };

const host = vi.hoisted(() => ({
    onHover: null as null | ((p: HoverPayload) => void),
    stored: [] as unknown[][],
}));
vi.mock("@/store/global", () => ({
    getApi: () => ({
        listen: async (_event: string, cb: (p: HoverPayload) => void) => {
            host.onHover = cb;
            return () => {};
        },
        windows: {
            setFloatingRedockTarget: async (...args: unknown[]) => {
                host.stored.push(args);
            },
        },
    }),
}));
vi.mock("@/util/platformutil", () => ({ isWindows: () => false }));
// Center: the whole leaf.
vi.mock("@/layout/lib/utils", () => ({ determineDropDirection: () => 8 }));

import { installFloatingRedockHoverListener } from "./redock-ghost";

let now = 0;
const ghost = () => document.querySelector<HTMLElement>(".floating-redock-drop-placeholder");
const isPreview = () => ghost()!.classList.contains("floating-redock-drop-placeholder--preview");

/** A hover sample over "main" at (x, y), `dt` ms after the previous one. */
async function hover(dt: number, x = 100, y = 100, target: string | null = "main") {
    now += dt;
    host.onHover!({ target_label: target, cursor_x: x, cursor_y: y });
    await Promise.resolve(); // let fire-and-forget IPCs settle
}

beforeAll(async () => {
    const leaf = document.createElement("div");
    leaf.dataset.blockid = "b1";
    leaf.getBoundingClientRect = () => ({ left: 0, top: 0, width: 400, height: 300, right: 400, bottom: 300, x: 0, y: 0, toJSON() {} }) as DOMRect;
    document.body.appendChild(leaf);
    document.elementFromPoint = () => leaf;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    installFloatingRedockHoverListener();
    await vi.waitFor(() => expect(host.onHover).not.toBeNull());
});

beforeEach(async () => {
    // Leave the window so every test starts disarmed.
    await hover(1000, 0, 0, null);
    host.stored = [];
});
afterEach(() => ghost()?.remove());

describe("redock ghost, two stages", () => {
    it("shows a faint preview on the first sample over this window, without a drop target", async () => {
        await hover(10);
        expect(ghost()).not.toBeNull();
        expect(isPreview()).toBe(true);
        expect(host.stored).toEqual([]);
    });

    it("turns solid and becomes the drop target once the cursor has dwelt", async () => {
        await hover(10);
        await hover(100);
        await hover(REDOCK_DWELL_MS);
        expect(isPreview()).toBe(false);
        expect(host.stored).toEqual([["main", "b1", 8]]);
    });

    it("drops back to the preview, and forgets the target, when the cursor speeds off", async () => {
        await hover(10);
        await hover(100);
        await hover(REDOCK_DWELL_MS);
        await hover(10, 900, 100); // 800 px in 10 ms
        expect(isPreview()).toBe(true);
        expect(host.stored.at(-1)).toEqual(["main", null, null]);
    });

    it("removes the ghost, and forgets any target, when the floater leaves this window", async () => {
        await hover(10);
        await hover(100);
        await hover(REDOCK_DWELL_MS);
        await hover(10, 100, 100, "window-2");
        expect(ghost()).toBeNull();
        expect(host.stored.at(-1)).toEqual(["main", null, null]);
    });

    it("sends no clear while only previewing: nothing was stored", async () => {
        await hover(10);
        await hover(10, 100, 100, null);
        expect(ghost()).toBeNull();
        expect(host.stored).toEqual([]);
    });
});
