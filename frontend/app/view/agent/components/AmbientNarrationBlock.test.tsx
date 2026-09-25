// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AmbientNarrationBlock — the line reads as the agent's own prose, and ends with
 * a small `ambient` tag. The tag is the whole point of the node being a distinct
 * type (the model did not write the line), so it is asserted, not assumed.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";

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

    it("renders text as plain text, not markdown or HTML", () => {
        const { container } = render(() => (
            <AmbientNarrationBlock node={{ ...node, text: "<b>hi</b> **there**" }} />
        ));
        expect(container.querySelector("b")).toBeNull();
        expect(container.querySelector(".agent-ambient-narration-text")?.textContent).toBe("<b>hi</b> **there**");
    });
});
