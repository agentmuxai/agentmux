// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { isTerminalContext } from "./webgl-context-loss";

/** An element whose `closest(".xterm")` finds an ancestor, or doesn't. */
const element = (insideTerminal: boolean) =>
    ({ closest: (selector: string) => (selector === ".xterm" && insideTerminal ? {} : null) }) as unknown as EventTarget;

describe("isTerminalContext", () => {
    it("is true for a canvas inside a terminal", () => {
        expect(isTerminalContext(element(true))).toBe(true);
    });

    it("is false for a canvas anywhere else", () => {
        expect(isTerminalContext(element(false))).toBe(false);
    });

    it("is false for a target that isn't an element", () => {
        expect(isTerminalContext(null)).toBe(false);
        expect(isTerminalContext({} as EventTarget)).toBe(false);
    });
});
