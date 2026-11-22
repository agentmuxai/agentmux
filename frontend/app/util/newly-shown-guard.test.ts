// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { guardNewlyShown, NEWLY_SHOWN_GUARD_MS, withinNewlyShownGuard } from "./newly-shown-guard";

afterEach(() => vi.useRealTimers());

describe("newly-shown guard (G6)", () => {
    it("is one constant of 800 ms", () => {
        expect(NEWLY_SHOWN_GUARD_MS).toBe(800);
    });

    it("covers presses from the moment shown until the time is up", () => {
        expect(withinNewlyShownGuard(1000, 1000)).toBe(true);
        expect(withinNewlyShownGuard(1000, 1000 + NEWLY_SHOWN_GUARD_MS - 1)).toBe(true);
        expect(withinNewlyShownGuard(1000, 1000 + NEWLY_SHOWN_GUARD_MS)).toBe(false);
        expect(withinNewlyShownGuard(1000, 999)).toBe(false);
    });

    it("stops a click and Enter on a newly shown element, then lets them through", () => {
        vi.useFakeTimers();
        let t = 0;
        const el = document.createElement("div");
        const button = document.createElement("button");
        el.appendChild(button);
        document.body.appendChild(el);
        const clicks = vi.fn();
        const keys = vi.fn();
        button.addEventListener("click", clicks);
        button.addEventListener("keydown", keys);
        guardNewlyShown(el, () => t);

        t = 100;
        button.click();
        button.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
        button.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }));
        expect(clicks).not.toHaveBeenCalled();
        expect(keys).toHaveBeenCalledTimes(1); // only the "a": typing is not a press

        t = NEWLY_SHOWN_GUARD_MS;
        vi.advanceTimersByTime(NEWLY_SHOWN_GUARD_MS);
        button.click();
        expect(clicks).toHaveBeenCalledTimes(1);
        el.remove();
    });
});
