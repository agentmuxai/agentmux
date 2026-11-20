// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { mountedPaneIds } from "./mounted-panes";

const tab = (html: string): HTMLElement => {
    const el = document.createElement("div");
    el.innerHTML = html;
    return el;
};

describe("mountedPaneIds", () => {
    it("is the panes whose frame is mounted in the tab", () => {
        const el = tab(`<div class="block block-frame-default" data-blockid="a"></div><div class="block block-frame-default" data-blockid="b"></div>`);
        expect([...mountedPaneIds(el)].sort()).toEqual(["a", "b"]);
    });

    it("counts a pane that shows its crash fallback: it is showing, with its own Reload pane and Close", () => {
        const el = tab(`<div class="block block-frame-default" data-blockid="a"></div><div class="block-error-fallback" data-blockid="b" role="alert"></div>`);
        expect([...mountedPaneIds(el)].sort()).toEqual(["a", "b"]);
    });

    it("does not count a drag preview's frame", () => {
        const el = tab(`<div class="block block-frame-default block-preview" data-blockid="a"></div>`);
        expect(mountedPaneIds(el).size).toBe(0);
    });

    it("is empty for no tab", () => {
        expect(mountedPaneIds(undefined).size).toBe(0);
        expect(mountedPaneIds(null).size).toBe(0);
    });
});
