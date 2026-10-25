// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SystemNoteBlock: an AgentMux note is its own quiet container, text only, and
 * never carries the human's turn class (SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md, P0).
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import type { SystemNoteNode } from "../types";
import { SystemNoteBlock } from "./SystemNoteBlock";

afterEach(() => cleanup());

const node = (over: Partial<SystemNoteNode> = {}): SystemNoteNode => ({
    type: "system_note",
    id: "note_1",
    text: "AgentMux was upgraded from 0.60.0 to 0.60.1 at 14:17.",
    ...over,
});

describe("SystemNoteBlock", () => {
    it("shows the note under an AgentMux label, in its own container", () => {
        const { container } = render(() => <SystemNoteBlock node={node()} />);
        const root = container.querySelector(".agent-system-note")!;
        expect(root.querySelector(".agent-system-note__label")?.textContent).toContain("AgentMux");
        expect(root.querySelector(".agent-system-note__text")?.textContent).toBe(
            "AgentMux was upgraded from 0.60.0 to 0.60.1 at 14:17."
        );
    });

    it("is never the human's turn", () => {
        const { container } = render(() => <SystemNoteBlock node={node()} />);
        expect(container.querySelector(".agent-user-message")).toBeNull();
        expect(container.querySelector(".agent-system-note")?.className).not.toContain("user");
    });

    it("says when the note arrived late", () => {
        const { container } = render(() => <SystemNoteBlock node={node({ delay: "sent 14:30, delivered 14:41" })} />);
        expect(container.querySelector(".agent-system-note__delay")?.textContent).toBe("sent 14:30, delivered 14:41");
        cleanup();
        expect(render(() => <SystemNoteBlock node={node()} />).container.querySelector(".agent-system-note__delay")).toBeNull();
    });

    it("reads the note as text, never as markup", () => {
        const { container } = render(() => <SystemNoteBlock node={node({ text: "<b>bold</b> <img src=x onerror=alert(1)>" })} />);
        expect(container.querySelector(".agent-system-note__text b")).toBeNull();
        expect(container.querySelector("img")).toBeNull();
        expect(container.querySelector(".agent-system-note__text")?.textContent).toContain("<b>bold</b>");
    });
});
