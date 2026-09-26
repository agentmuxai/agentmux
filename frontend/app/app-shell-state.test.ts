// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Closing the main window deletes its record while the page is still on
 * screen, and the "invalid configuration" startup error used to paint on the
 * way out (SPEC_SHUTDOWN_INVALID_CONFIGURATION_FLASH_2026_09_22 §7–§9). These
 * pin the split: the error is for "never loaded" only; "loaded, now gone" is
 * a teardown and renders no text.
 */

import { describe, expect, it } from "vitest";
import { appShellState } from "./app-shell-state";

describe("appShellState", () => {
    it("renders the app whenever client and window are both present", () => {
        expect(appShellState(true, true, false)).toBe("app");
        expect(appShellState(true, true, true)).toBe("app");
    });

    it("keeps the startup error when the window never loaded", () => {
        expect(appShellState(false, false, false)).toBe("invalid");
        expect(appShellState(true, false, false)).toBe("invalid");
        expect(appShellState(false, true, false)).toBe("invalid");
    });

    it("treats a loaded window losing its client or window as teardown, not an error", () => {
        expect(appShellState(true, false, true)).toBe("unloading");
        expect(appShellState(false, false, true)).toBe("unloading");
        expect(appShellState(false, true, true)).toBe("unloading");
    });
});
