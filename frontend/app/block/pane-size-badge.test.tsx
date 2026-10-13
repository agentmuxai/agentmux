// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeAll, describe, expect, it } from "vitest";

import { PaneSizeBadge } from "./pane-size-badge";

beforeAll(() => {
    // jsdom has no ResizeObserver; the badge only needs one to exist.
    globalThis.ResizeObserver ??= class {
        observe() {}
        disconnect() {}
        unobserve() {}
    } as unknown as typeof ResizeObserver;
});

afterEach(() => cleanup());

describe("PaneSizeBadge", () => {
    const frame = () => {
        const el = document.createElement("div");
        el.getBoundingClientRect = () => ({ width: 812.4, height: 437.6 }) as DOMRect;
        return el;
    };

    it("shows the frame's size", () => {
        const el = frame();
        const { container } = render(() => <PaneSizeBadge target={() => el} />);
        const badge = container.querySelector(".pane-size-badge")!;
        expect(badge.textContent).toBe("812×438");
    });
});
