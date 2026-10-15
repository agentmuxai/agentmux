// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The frontend's view of the window's input owner
// (SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md §3.6): while a browser pane has
// the keyboard the page's document isn't focused, yet the user is in this window.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const host = vi.hoisted(() => ({
    listeners: new Map<string, (payload: unknown) => void>(),
    state: null as InputOwnerReport | null,
}));
vi.mock("@/app/store/global", () => ({
    getApi: () => ({
        listen: (event: string, cb: (payload: unknown) => void) => {
            host.listeners.set(event, cb);
            return Promise.resolve(() => {});
        },
        inputOwnerState: () => Promise.resolve(host.state),
    }),
}));

import {
    inputOwnerReport,
    onInputOwnerChange,
    paneHasTheKeys,
    registerInputOwnerTracking,
    userInThisWindow,
} from "./input-owner";

let n = 0;
const report = (r: Partial<InputOwnerReport>) =>
    host.listeners.get("input-owner-changed")!({ active: true, keys: "app", block_id: null, n: ++n, ...r });

describe("input owner (frontend)", () => {
    let documentFocused = false;
    beforeEach(() => {
        documentFocused = false;
        vi.spyOn(document, "hasFocus").mockImplementation(() => documentFocused);
        host.state = null;
        registerInputOwnerTracking();
    });
    afterEach(() => {
        vi.restoreAllMocks();
    });

    it("the user is in the window while its page has focus", () => {
        documentFocused = true;
        report({ active: false, keys: null });
        expect(userInThisWindow()).toBe(true);
    });

    it("the user is in the window while one of its browser panes has the keys", () => {
        report({ keys: "pane", block_id: "b1" });
        expect(paneHasTheKeys()).toBe(true);
        expect(userInThisWindow()).toBe(true);
    });

    it("not while the window is inactive, even if a pane had the keys last", () => {
        report({ active: false, keys: "pane", block_id: "b1" });
        expect(userInThisWindow()).toBe(false);
    });

    it("not while the keys are elsewhere and the page isn't focused", () => {
        report({ keys: "elsewhere" });
        expect(userInThisWindow()).toBe(false);
        report({ keys: "app" });
        expect(userInThisWindow()).toBe(false);
    });

    it("tells its listeners about each report", () => {
        const seen: (string | null | undefined)[] = [];
        const off = onInputOwnerChange(() => seen.push(inputOwnerReport()?.keys));
        report({ keys: "pane", block_id: "b1" });
        report({ keys: "app" });
        off();
        report({ keys: "pane", block_id: "b2" });
        expect(seen).toEqual(["pane", "app"]);
    });

    it("a report that arrives after a newer one is dropped", () => {
        report({ keys: "pane", block_id: "b1", n: 1000 });
        report({ keys: "app", n: 999 });
        expect(inputOwnerReport()?.keys).toBe("pane");
        report({ keys: "app", n: 1001 });
        expect(inputOwnerReport()?.keys).toBe("app");
        n = 1001;
    });
});
