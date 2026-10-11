// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const buttonState = vi.fn<() => Promise<boolean>>();

vi.mock("@/store/global", () => ({ getApi: () => ({ getMouseButtonState: buttonState }) }));
vi.mock("@/util/logger", () => ({ Logger: { info: vi.fn(), warn: vi.fn() } }));

import { ESCAPE_KEY_WAIT_MS, endedByEscape, forgetEscapeForTest, isCancelledDragEnd } from "./native-cancel";

const escape = (type: "keydown" | "keyup") => window.dispatchEvent(new KeyboardEvent(type, { key: "Escape" }));
const unreadable = () => buttonState.mockRejectedValue(new Error("native Wayland"));

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

describe("endedByEscape, where the host reads the button", () => {
    it("is a cancel when the button is still held", async () => {
        buttonState.mockResolvedValue(true);
        expect(await endedByEscape("none")).toBe(true);
    });

    it("is a release at once when the button is up, whatever the keys do", async () => {
        buttonState.mockResolvedValue(false);
        escape("keyup");
        expect(await endedByEscape("none")).toBe(false);
    });

    it("doesn't ask when a drop was accepted", async () => {
        expect(await endedByEscape("move")).toBe(false);
        expect(buttonState).not.toHaveBeenCalled();
    });
});

// Native Wayland: no button state, and the page gets only Escape's keyup,
// when the key is let go.
describe("endedByEscape, where the button can't be read", () => {
    it("is a cancel when Escape is let go soon after the dragend", async () => {
        unreadable();
        const result = endedByEscape("none");
        await vi.advanceTimersByTimeAsync(10);
        escape("keyup");
        await vi.advanceTimersByTimeAsync(10);
        expect(await result).toBe(true);
    });

    it("waits for an Escape held well past the dragend", async () => {
        unreadable();
        const result = endedByEscape("none");
        await vi.advanceTimersByTimeAsync(200);
        escape("keyup");
        await vi.advanceTimersByTimeAsync(10);
        expect(await result).toBe(true);
    });

    it("counts a keyup that lands while the button query is still in flight", async () => {
        let fail!: (e: Error) => void;
        buttonState.mockReturnValue(new Promise((_, reject) => (fail = reject)));
        const result = endedByEscape("none");
        escape("keyup");
        fail(new Error("native Wayland"));
        expect(await result).toBe(true);
    });

    it("is a release when no Escape comes in time", async () => {
        unreadable();
        const result = endedByEscape("none");
        await vi.advanceTimersByTimeAsync(ESCAPE_KEY_WAIT_MS + 20);
        expect(await result).toBe(false);
    });

    it("doesn't count an Escape from long before the drag ended", async () => {
        escape("keydown");
        await vi.advanceTimersByTimeAsync(1000);
        unreadable();
        const result = endedByEscape("none");
        await vi.advanceTimersByTimeAsync(ESCAPE_KEY_WAIT_MS + 20);
        expect(await result).toBe(false);
    });

    it("doesn't count an Escape after the wait, even if the query was slow", async () => {
        let fail!: (e: Error) => void;
        buttonState.mockReturnValue(new Promise((_, reject) => (fail = reject)));
        const result = endedByEscape("none");
        await vi.advanceTimersByTimeAsync(ESCAPE_KEY_WAIT_MS + 50);
        escape("keyup");
        fail(new Error("native Wayland"));
        expect(await result).toBe(false);
    });
});
