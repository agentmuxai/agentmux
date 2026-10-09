// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Streamed output joined into lines (ChunkPreview → preview-text chunksDoc).
 * The spinner collapser hands back frames without their line break; each one
 * is still a line of its own, not the start of the next.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { ChunkPreview } from "./ChunkPreview";
import { MAX_TOOL_OUTPUT_LINES } from "./output-cap";

afterEach(() => cleanup());

const chunk = (content: string, kind = "stdout") => ({ kind, content });
const lines = (c: HTMLElement) => [...c.querySelectorAll(".agent-preview-line")].map((l) => l.textContent);

describe("ChunkPreview", () => {
    it("joins chunks cut mid-line back into lines", () => {
        const { container } = render(() => <ChunkPreview chunks={[chunk("first li"), chunk("ne\nsecond\n")]} />);
        expect(lines(container)).toEqual(["first line", "second"]);
    });

    it("a frozen spinner frame keeps its own line; the output after it doesn't join it", () => {
        const { container } = render(() => <ChunkPreview chunks={[chunk("⠋"), chunk("⠙"), chunk("Done!")]} />);
        expect(lines(container)).toEqual(["⠙", "Done!"]);
    });

    it("a frozen progress frame keeps its own line", () => {
        const { container } = render(() => (
            <ChunkPreview
                chunks={[
                    chunk("Downloading (12%)"),
                    chunk("Downloading (50%)"),
                    chunk("Downloading (100%)"),
                    chunk("installed\n"),
                ]}
            />
        ));
        expect(lines(container)).toEqual(["Downloading (100%)", "installed"]);
    });

    it("the live spinner is its own line, even after an unfinished one", () => {
        const { container } = render(() => (
            <ChunkPreview chunks={[chunk("building"), chunk("Installing deps... ⠋"), chunk("Installing deps... ⠙")]} />
        ));
        expect(lines(container)).toEqual(["building", "Installing deps... ⠙"]);
    });

    it("hides bashwrap's starting notice", () => {
        const { container } = render(() => (
            <ChunkPreview chunks={[chunk("[bashwrap] starting: 5 chars", "system"), chunk("out\n")]} />
        ));
        expect(lines(container)).toEqual(["out"]);
    });

    it("keeps the latest lines of a long stream and counts the hidden ones in lines, not chunks", () => {
        // 1500 lines, each streamed as two chunks: "line N" then "\n".
        const chunks = Array.from({ length: 1500 }, (_, i) => [chunk(`line ${i}`), chunk("\n")]).flat();
        const { container } = render(() => <ChunkPreview chunks={chunks} />);
        const shown = lines(container);
        expect(shown).toHaveLength(MAX_TOOL_OUTPUT_LINES);
        expect(shown[0]).toBe("line 500");
        expect(shown.at(-1)).toBe("line 1499");
        expect(container.querySelector(".agent-output-hidden-marker")!.textContent).toContain("500");
    });

    it("doesn't split a line at the window's edge", () => {
        // Every line arrives in three pieces, so the window's first chunk is mid-line.
        const chunks = Array.from({ length: 1200 }, (_, i) => [chunk("a"), chunk(`b${i}`), chunk("c\n")]).flat();
        const { container } = render(() => <ChunkPreview chunks={chunks} />);
        const shown = lines(container);
        expect(shown).toHaveLength(MAX_TOOL_OUTPUT_LINES);
        expect(shown.every((l) => /^ab\d+c$/.test(l ?? ""))).toBe(true);
        expect(container.querySelector(".agent-output-hidden-marker")!.textContent).toContain("200");
    });
});
