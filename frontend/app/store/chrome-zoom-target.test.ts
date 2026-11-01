// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Where a Ctrl+wheel zooms chrome rather than the pane: the bars, anything marked chrome, and not the
 * content of a pane that follows chrome zoom as a whole (the nearest data-zoom-scope wins).
 */

import { beforeEach, describe, expect, it } from "vitest";
import { isOverChrome } from "./chrome-zoom-target";

function html(markup: string): HTMLElement {
    document.body.innerHTML = markup;
    return document.getElementById("t") as HTMLElement;
}

beforeEach(() => {
    document.body.innerHTML = "";
});

describe("isOverChrome", () => {
    it("is true over the window header, the status bar and a pane header", () => {
        expect(isOverChrome(html(`<div class="window-header"><b id="t"></b></div>`))).toBe(true);
        expect(isOverChrome(html(`<div class="status-bar"><b id="t"></b></div>`))).toBe(true);
        expect(isOverChrome(html(`<div class="block-frame-default-header"><b id="t"></b></div>`))).toBe(true);
    });

    it("is true inside anything marked chrome, and false in a pane", () => {
        expect(isOverChrome(html(`<div data-zoom-scope="chrome"><b id="t"></b></div>`))).toBe(true);
        expect(isOverChrome(html(`<div data-blockid="b1"><b id="t"></b></div>`))).toBe(false);
    });

    it("lets content inside a pane drawn at chrome zoom be the pane's own", () => {
        const markup = `<div data-zoom-scope="chrome"><i id="bar"></i><div data-zoom-scope="content"><b id="t"></b></div></div>`;
        expect(isOverChrome(html(markup))).toBe(false);
        expect(isOverChrome(document.getElementById("bar") as HTMLElement)).toBe(true);
    });
});
