// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { isFloaterHeaderMovePress } from "./floater-header-press";

function header(): { root: HTMLElement; pill: HTMLElement; label: HTMLElement; gap: HTMLElement; close: HTMLElement; grip: HTMLElement; body: HTMLElement } {
    const root = document.createElement("div");
    root.innerHTML = `
        <div data-role="block-header">
            <span class="pane-grip" role="button"></span>
            <div class="pane-tab"><span class="label">notes</span><button class="close"></button></div>
            <div class="gap"></div>
        </div>
        <div class="body"></div>`;
    const q = (s: string) => root.querySelector<HTMLElement>(s)!;
    return { root, pill: q(".pane-tab"), label: q(".label"), gap: q(".gap"), close: q(".close"), grip: q(".pane-grip"), body: q(".body") };
}

describe("isFloaterHeaderMovePress", () => {
    it("moves the window from the empty part of the header and from the grip", () => {
        const h = header();
        expect(isFloaterHeaderMovePress(h.gap)).toBe(true);
        expect(isFloaterHeaderMovePress(h.grip)).toBe(true);
    });

    it("leaves a pane tab pill, and what is inside it, its own press", () => {
        const h = header();
        expect(isFloaterHeaderMovePress(h.pill)).toBe(false);
        expect(isFloaterHeaderMovePress(h.label)).toBe(false);
        expect(isFloaterHeaderMovePress(h.close)).toBe(false);
    });

    it("ignores a press outside the header", () => {
        const h = header();
        expect(isFloaterHeaderMovePress(h.body)).toBe(false);
        expect(isFloaterHeaderMovePress(null)).toBe(false);
    });
});
