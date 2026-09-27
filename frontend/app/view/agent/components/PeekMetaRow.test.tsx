// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PeekMetaRow — the hover peek's single time + token line.
 * SPEC_PEEK_PANEL_META_ROW_AND_MONO_COMMAND_2026_09_27.md §4.1.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";

import { PeekMetaRow } from "./PeekMetaRow";

afterEach(() => cleanup());

const row = (c: HTMLElement) => c.querySelectorAll(".agent-node-peek-tooltip-meta");

describe("PeekMetaRow", () => {
    it("puts time and tokens in one row, time first", () => {
        const { container } = render(() => <PeekMetaRow time="9:14:02 PM · 3m ago" tokens="~1.2k tok (est.)" />);
        expect(row(container).length).toBe(1);
        const spans = row(container)[0].querySelectorAll("span");
        expect([...spans].map((s) => s.className)).toEqual([
            "agent-node-peek-tooltip-time",
            "agent-node-peek-tooltip-tokens",
        ]);
        expect(spans[0].textContent).toBe("9:14:02 PM · 3m ago");
        expect(spans[1].textContent).toBe("~1.2k tok (est.)");
    });

    it("time only", () => {
        const { container } = render(() => <PeekMetaRow time="9:14:02 PM · 3m ago" tokens={null} />);
        expect(row(container).length).toBe(1);
        expect(container.querySelector(".agent-node-peek-tooltip-tokens")).toBeNull();
        expect(container.querySelector(".agent-node-peek-tooltip-time")?.textContent).toBe("9:14:02 PM · 3m ago");
    });

    it("tokens only", () => {
        const { container } = render(() => <PeekMetaRow tokens="~40 tok (est.)" />);
        expect(row(container).length).toBe(1);
        expect(container.querySelector(".agent-node-peek-tooltip-time")).toBeNull();
        expect(container.querySelector(".agent-node-peek-tooltip-tokens")?.textContent).toBe("~40 tok (est.)");
    });

    it("renders nothing when neither is known", () => {
        const { container } = render(() => <PeekMetaRow time={null} tokens={undefined} />);
        expect(row(container).length).toBe(0);
    });
});
