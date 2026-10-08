// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { Input } from "./input";

afterEach(() => cleanup());

// REPORT_FOCUS_ON_OPEN_AUDIT_2026_10_08.md: Terminal Find, the connection
// switcher and Open from remote all rely on `autoFocus`, which used to be only
// the attribute (page load only), so typing went to the terminal.
describe("Input autoFocus", () => {
    it("takes the caret when it is rendered and selects the text with autoSelect", async () => {
        const { container } = render(() => <Input value="previous query" autoFocus autoSelect />);
        await Promise.resolve();
        const input = container.querySelector("input")!;
        expect(document.activeElement).toBe(input);
        expect([input.selectionStart, input.selectionEnd]).toEqual([0, "previous query".length]);
    });

    it("leaves the caret alone without it", async () => {
        const { container } = render(() => <Input value="x" />);
        await Promise.resolve();
        expect(document.activeElement).not.toBe(container.querySelector("input"));
    });
});
