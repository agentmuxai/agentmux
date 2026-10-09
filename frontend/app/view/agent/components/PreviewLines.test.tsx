// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { codeDoc, outputDoc } from "../preview-text/docs";
import { PreviewLines } from "./PreviewLines";

afterEach(() => cleanup());

describe("PreviewLines", () => {
    it("says where the character cap cut, on the cut side", () => {
        const huge = "x".repeat(1_000_100);
        const tail = render(() => <PreviewLines doc={outputDoc(huge, { from: "tail" })} />).container;
        const head = render(() => <PreviewLines doc={codeDoc(huge, { path: "a.min.js" })} />).container;
        const markerFirst = (c: HTMLElement) =>
            c.querySelector(".agent-preview")!.firstElementChild!.classList.contains("agent-preview-truncated");
        expect(tail.querySelector(".agent-preview-truncated")!.textContent).toContain("earlier text cut");
        expect(markerFirst(tail)).toBe(true);
        expect(head.querySelector(".agent-preview-truncated")!.textContent).toContain("text cut here");
        expect(markerFirst(head)).toBe(false);
    });

    it("shows no cut marker when nothing was cut", () => {
        const { container } = render(() => <PreviewLines doc={outputDoc("short", { from: "tail" })} />);
        expect(container.querySelector(".agent-preview-truncated")).toBeNull();
    });

    it("keeps the gutter and diff marker out of the text", () => {
        const { container } = render(() => (
            <PreviewLines doc={codeDoc("     7\tconst a = 1;", { path: "a.ts", gutter: true })} />
        ));
        expect(container.querySelector(".agent-preview-gutter")!.textContent).toBe("7");
        expect(container.querySelector(".agent-preview-text")!.textContent).toBe("const a = 1;");
    });
});
