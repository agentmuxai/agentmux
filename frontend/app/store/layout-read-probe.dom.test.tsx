// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { createLayoutReadProbe, wrapLayoutReads } from "./layout-read-probe";

describe("wrapLayoutReads on the page's own elements", () => {
    let undo: (() => void) | null = null;
    afterEach(() => {
        undo?.();
        undo = null;
    });

    it("times the browser's layout calls on a real element, and leaves them as they were when undone", () => {
        let t = 0;
        const lines: string[] = [];
        const probe = createLayoutReadProbe({
            now: () => (t += 3),
            log: (l) => lines.push(l),
            stack: () => "Error\n    at caller (http://h/src/c.ts:9:1)",
        });
        probe.setEnabled(true);
        const before = Object.getOwnPropertyDescriptor(Element.prototype, "getBoundingClientRect");
        undo = wrapLayoutReads(probe);

        const div = document.createElement("div");
        div.getBoundingClientRect();
        expect(lines).toEqual([]);

        probe.enter({ data: { command: "eventrecv", data: { event: "x:y" } } });
        expect(div.getBoundingClientRect().width).toBe(0);
        expect(div.offsetWidth).toBe(0);
        div.scrollTop = 4;
        probe.leave();
        expect(lines.map((l) => /api=(\w+)/.exec(l)?.[1])).toEqual(["getBoundingClientRect", "offsetWidth"]);

        undo();
        undo = null;
        expect(Object.getOwnPropertyDescriptor(Element.prototype, "getBoundingClientRect")).toEqual(before);
    });
});
