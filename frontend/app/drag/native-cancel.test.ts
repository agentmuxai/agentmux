// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

const buttonState = vi.fn<() => Promise<boolean>>();
let windows = true;

vi.mock("@/store/global", () => ({ getApi: () => ({ getMouseButtonState: buttonState }) }));
vi.mock("@/util/platformutil", () => ({ isWindows: () => windows }));
vi.mock("@/util/logger", () => ({ Logger: { info: vi.fn(), warn: vi.fn() } }));

import { endedByEscape, isCancelledDragEnd } from "./native-cancel";

afterEach(() => {
    buttonState.mockReset();
    windows = true;
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
    it("asks Windows whether the button is held", async () => {
        buttonState.mockResolvedValue(true);
        expect(await endedByEscape("none")).toBe(true);
        buttonState.mockResolvedValue(false);
        expect(await endedByEscape("none")).toBe(false);
    });

    it("doesn't ask when a drop was accepted", async () => {
        expect(await endedByEscape("move")).toBe(false);
        expect(buttonState).not.toHaveBeenCalled();
    });

    it("leaves other platforms to their own Escape handling", async () => {
        windows = false;
        expect(await endedByEscape("none")).toBe(false);
        expect(buttonState).not.toHaveBeenCalled();
    });

    it("treats a failed query as a release", async () => {
        buttonState.mockRejectedValue(new Error("no host"));
        expect(await endedByEscape("none")).toBe(false);
    });
});
