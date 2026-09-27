// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AmbientNarrationBlock — the line reads as the agent's own prose, and ends with
 * a small `ambient` tag. The tag is the whole point of the node being a distinct
 * type (the model did not write the line), so it is asserted, not assumed.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AmbientNarrationNode } from "../types";
import { AMBIENT_TAG_LABEL, AMBIENT_TAG_TITLE, AmbientNarrationBlock } from "./AmbientNarrationBlock";

afterEach(() => cleanup());

const node: AmbientNarrationNode = {
    type: "ambient_narration",
    id: "ambient-1-1",
    kind: "background_task",
    text: "Running task dev in the background.",
    timestamp: 1_000,
};

describe("AmbientNarrationBlock", () => {
    it("renders in the agent-prose wrapper, not as a separate labeled row", () => {
        const { container } = render(() => <AmbientNarrationBlock node={node} />);
        const block = container.querySelector(".agent-markdown-block");
        expect(block).not.toBeNull();
        expect(block?.textContent).toContain("Running task dev in the background.");
    });

    it("ends with exactly one ambient tag, inside the same block as the text", () => {
        const { container } = render(() => <AmbientNarrationBlock node={node} />);
        const block = container.querySelector(".agent-markdown-block") as HTMLElement;
        const tags = block.querySelectorAll(".agent-ambient-narration-tag");
        expect(tags).toHaveLength(1);
        expect(tags[0].textContent?.trim()).toBe(AMBIENT_TAG_LABEL);
        expect(tags[0].getAttribute("title")).toBe(AMBIENT_TAG_TITLE);
        // Trailing: nothing follows the tag, and the text precedes it.
        expect(block.lastElementChild).toBe(tags[0]);
        expect(block.textContent?.trim().endsWith(AMBIENT_TAG_LABEL)).toBe(true);
    });

    it("keeps the tag in the copied text, separated from the sentence by a space", () => {
        const { container } = render(() => <AmbientNarrationBlock node={node} />);
        const block = container.querySelector(".agent-markdown-block") as HTMLElement;
        expect(block.textContent).toBe(`${node.text} ${AMBIENT_TAG_LABEL}`);
    });

    // SPEC_PEEK_PANEL_META_ROW_AND_MONO_COMMAND_2026_09_27.md §4.1: the time sits in
    // the shared meta row; the provenance note is its own line, not part of it.
    it("peek: time in the meta row, provenance note on its own line", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => <AmbientNarrationBlock node={node} />);
            fireEvent.mouseEnter(container.querySelector(".agent-markdown-peek-anchor") as HTMLElement);
            vi.advanceTimersByTime(100);
            const meta = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
            expect(meta.length).toBe(1);
            expect(meta[0].querySelector(".agent-node-peek-tooltip-time")).not.toBeNull();
            const note = document.body.querySelector(".agent-node-peek-tooltip-note");
            expect(note?.textContent).toMatch(/ambient narration \(background_task\)/);
            expect(meta[0].contains(note)).toBe(false);
        } finally {
            vi.useRealTimers();
        }
    });

    it("renders text as plain text, not markdown or HTML", () => {
        const { container } = render(() => (
            <AmbientNarrationBlock node={{ ...node, text: "<b>hi</b> **there**" }} />
        ));
        expect(container.querySelector("b")).toBeNull();
        expect(container.querySelector(".agent-ambient-narration-text")?.textContent).toBe("<b>hi</b> **there**");
    });
});
