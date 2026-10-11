// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const buttonState = vi.fn<() => Promise<boolean>>();

vi.mock("@/store/global", () => ({ getApi: () => ({ getMouseButtonState: buttonState }) }));
vi.mock("@/util/logger", () => ({ Logger: { info: vi.fn(), warn: vi.fn() } }));

import { ESCAPE_AFTER_DRAGEND_MS, endedByEscape, forgetEscapeForTest, isCancelledDragEnd } from "./native-cancel";

const escape = (type: "keydown" | "keyup") => window.dispatchEvent(new KeyboardEvent(type, { key: "Escape" }));

beforeEach(() => {
    vi.useFakeTimers();
    forgetEscapeForTest();
});

afterEach(() => {
    vi.useRealTimers();
    buttonState.mockReset();
});

describe("isCancelledDragEnd", () => {
    it("is a cancel only with no drop effect and the button still down", () => {
        expect(isCancelledDragEnd("none", true)).toBe(true);
        expect(isCancelledDragEnd("none", false)).toBe(false); // a release over nothing
        expect(isCancelledDragEnd("move", true)).toBe(false);
        expect(isCancelledDragEnd("copy", true)).toBe(false);
        expect(isCancelledDragEnd(undefined, true)).toBe(false);
    });
});

describe("endedByEscape", () => {
    it("is a cancel at once when the host reads the button still held", async () => {
        buttonState.mockResolvedValue(true);
        expect(await endedByEscape("none")).toBe(true);
    });

    it("is a release when the button is up and no Escape follows", async () => {
        buttonState.mockResolvedValue(false);
        const result = endedByEscape("none");
        await vi.advanceTimersByTimeAsync(ESCAPE_AFTER_DRAGEND_MS + 20);
        expect(await result).toBe(false);
    });

    // Linux, native Wayland: the button can't be read, and Escape's keyup
    // arrives ~10 ms after the dragend.
    it("is a cancel when Escape's keyup follows the dragend, even if the button can't be read", async () => {
        buttonState.mockRejectedValue(new Error("no X server"));
        const result = endedByEscape("none");
        await vi.advanceTimersByTimeAsync(10);
        escape("keyup");
        await vi.advanceTimersByTimeAsync(10);
        expect(await result).toBe(true);
    });

    it("counts a keyup that lands while the button query is still in flight", async () => {
        let answer!: (down: boolean) => void;
        buttonState.mockReturnValue(new Promise((resolve) => (answer = resolve)));
        const result = endedByEscape("none");
        escape("keyup");
        answer(false);
        expect(await result).toBe(true);
    });

    it("doesn't count an Escape from long before the drag ended", async () => {
        escape("keydown");
        await vi.advanceTimersByTimeAsync(1000);
        buttonState.mockResolvedValue(false);
        const result = endedByEscape("none");
        await vi.advanceTimersByTimeAsync(ESCAPE_AFTER_DRAGEND_MS + 20);
        expect(await result).toBe(false);
    });

    it("doesn't ask when a drop was accepted", async () => {
        expect(await endedByEscape("move")).toBe(false);
        expect(buttonState).not.toHaveBeenCalled();
    });
});
