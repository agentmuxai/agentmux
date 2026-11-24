// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The find bar over a browser pane's page (a native window above the DOM) must register as a pane overlay on every
 * platform: the automatic `data-pane-overlay` discovery only runs on Windows, so on macOS and Linux a bar that only
 * carried the attribute would paint under the page and could not be clicked.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ registered: [] as Array<() => HTMLElement | null | undefined> }));

vi.mock("@/app/platform/pane-overlay", () => ({
    usePaneOverlay: (getEl: () => HTMLElement | null | undefined) => {
        h.registered.push(getEl);
    },
}));
vi.mock("@floating-ui/dom", () => ({
    offset: () => ({}),
    autoUpdate: () => () => {},
    computePosition: async () => ({ x: 0, y: 0 }),
}));

import { createSignalAtom } from "@/util/util";
import { Search } from "./search";

afterEach(() => {
    cleanup();
    h.registered = [];
});

describe("the find bar as a pane overlay", () => {
    it("registers its own box, once, while it is open, and not at all while closed", async () => {
        const isOpen = createSignalAtom(false);
        render(() => (
            <Search
                searchValue={createSignalAtom("")}
                resultsIndex={createSignalAtom(0)}
                resultsCount={createSignalAtom(0)}
                isOpen={isOpen}
                anchorRef={{ current: null }}
            />
        ));
        expect(h.registered).toHaveLength(0);
        isOpen._set(true);
        await Promise.resolve();
        expect(h.registered).toHaveLength(1);
        expect(h.registered[0]()?.className).toBe("search-container");
        expect(document.querySelector("[data-pane-overlay]")).toBeNull();
    });
});
