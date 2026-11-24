// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { PaneSizeBadge } from "./pane-size-badge";

// jsdom has no ResizeObserver: this one hands its callback to the test.
let notify: ((entries: unknown[]) => void) | null = null;

beforeEach(() => {
    notify = null;
    vi.stubGlobal(
        "ResizeObserver",
        class {
            constructor(cb: (entries: unknown[]) => void) {
                notify = cb;
            }
            observe() {}
            disconnect() {}
            unobserve() {}
        }
    );
});

afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

describe("PaneSizeBadge", () => {
    it("measures nothing when it mounts, and stays hidden until the observer has the size", () => {
        const el = document.createElement("div");
        const rect = vi.fn(() => ({ width: 1, height: 1 }) as DOMRect);
        el.getBoundingClientRect = rect;
        const { container } = render(() => <PaneSizeBadge target={() => el} />);
        const badge = container.querySelector<HTMLElement>(".pane-size-badge")!;
        // Every pane mounts a badge as a drag begins: a read here would lay the page out once per pane.
        expect(rect).not.toHaveBeenCalled();
        expect(badge.style.visibility).toBe("hidden");
        notify!([{ borderBoxSize: [{ inlineSize: 812.4, blockSize: 437.6 }] }]);
        expect(badge.textContent).toBe("812×438");
        expect(badge.style.visibility).toBe("");
    });
});
