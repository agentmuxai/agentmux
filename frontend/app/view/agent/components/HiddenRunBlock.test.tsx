// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom
// The line for a run of hidden rows: what is in it, what failed, and the click that opens it.

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { HiddenRunNode } from "../types";
import { HiddenRunBlock } from "./HiddenRunBlock";

const run = (over: Partial<HiddenRunNode> = {}): HiddenRunNode => ({
    type: "hidden_run",
    id: "hidden-run:t1",
    counts: [
        { kind: "tool", count: 12 },
        { kind: "thinking", count: 3 },
    ],
    failed: 0,
    open: false,
    hiddenIds: ["t1"],
    ...over,
});

afterEach(cleanup);

describe("HiddenRunBlock", () => {
    it("names what is hidden", () => {
        const { container } = render(() => <HiddenRunBlock node={run()} onToggle={() => {}} />);
        expect(container.textContent).toContain("12 tool calls, 3 thoughts hidden");
    });

    it("says how many tool calls failed", () => {
        const { container } = render(() => <HiddenRunBlock node={run({ failed: 2 })} onToggle={() => {}} />);
        expect(container.querySelector(".hidden-run-failed")?.textContent).toBe(", 2 failed");
    });

    it("opens on a click and reports its state to assistive tech", () => {
        const onToggle = vi.fn();
        const { container } = render(() => <HiddenRunBlock node={run({ open: true })} onToggle={onToggle} />);
        const button = container.querySelector("button")!;
        expect(button.getAttribute("aria-expanded")).toBe("true");
        fireEvent.click(button);
        expect(onToggle).toHaveBeenCalledTimes(1);
    });
});
