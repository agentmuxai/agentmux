// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * L1 unit tests for `findScrollContainerRect` — used by PeekOverlay.tsx to
 * cap the hover-to-peek overlay's `max-height` to the space actually
 * available inside the pane, rather than the whole viewport.
 *
 * reagent P2 on PR #2392: the previous version of this file (deleted
 * alongside `pickExpandDirection`/`maxOverlayHeight`, which had migrated
 * off to PeekOverlay.tsx's simplified top-anchored-only positioning) left
 * `findScrollContainerRect` — still very much alive, now depended on by
 * every hover-to-peek surface via PeekOverlay.tsx — with zero direct
 * coverage.
 */

import { afterEach, describe, expect, it } from "vitest";
import { findScrollContainerRect } from "./hover-anchor";

function mockRect(el: HTMLElement, rect: { top: number; bottom: number }): void {
    el.getBoundingClientRect = () =>
        ({ ...rect, left: 0, right: 0, width: 0, height: rect.bottom - rect.top, x: 0, y: rect.top, toJSON() { return this; } }) as DOMRect;
}

describe("findScrollContainerRect", () => {
    afterEach(() => {
        document.body.innerHTML = "";
    });

    it("returns the immediate parent's rect when it scrolls (overflow-y: auto)", () => {
        const parent = document.createElement("div");
        parent.style.overflowY = "auto";
        const child = document.createElement("div");
        parent.appendChild(child);
        document.body.appendChild(parent);
        mockRect(parent, { top: 10, bottom: 500 });

        expect(findScrollContainerRect(child)).toEqual({ top: 10, bottom: 500 });
    });

    it("returns the immediate parent's rect when it clips (overflow-y: hidden)", () => {
        const parent = document.createElement("div");
        parent.style.overflowY = "hidden";
        const child = document.createElement("div");
        parent.appendChild(child);
        document.body.appendChild(parent);
        mockRect(parent, { top: 0, bottom: 300 });

        expect(findScrollContainerRect(child)).toEqual({ top: 0, bottom: 300 });
    });

    it("returns the immediate parent's rect for overflow-y: scroll", () => {
        const parent = document.createElement("div");
        parent.style.overflowY = "scroll";
        const child = document.createElement("div");
        parent.appendChild(child);
        document.body.appendChild(parent);
        mockRect(parent, { top: 20, bottom: 620 });

        expect(findScrollContainerRect(child)).toEqual({ top: 20, bottom: 620 });
    });

    it("walks past non-scrolling ancestors to find the nearest real scroll container", () => {
        const grandparent = document.createElement("div");
        grandparent.style.overflowY = "auto";
        const parent = document.createElement("div"); // default overflow-y: visible
        const child = document.createElement("div");
        grandparent.appendChild(parent);
        parent.appendChild(child);
        document.body.appendChild(grandparent);
        mockRect(grandparent, { top: 5, bottom: 905 });

        expect(findScrollContainerRect(child)).toEqual({ top: 5, bottom: 905 });
    });

    it("falls back to the whole viewport when no ancestor scrolls", () => {
        const parent = document.createElement("div"); // default overflow-y: visible
        const child = document.createElement("div");
        parent.appendChild(child);
        document.body.appendChild(parent);

        expect(findScrollContainerRect(child)).toEqual({ top: 0, bottom: window.innerHeight });
    });

    it("does not treat document.body itself as a scroll container, even if it would match", () => {
        // The loop's `current !== document.body` guard means an
        // overflow-y set directly on <body> is never inspected — only
        // ancestors STRICTLY BETWEEN `el` and `document.body` count.
        document.body.style.overflowY = "auto";
        const child = document.createElement("div");
        document.body.appendChild(child);

        expect(findScrollContainerRect(child)).toEqual({ top: 0, bottom: window.innerHeight });

        document.body.style.overflowY = "";
    });
});
