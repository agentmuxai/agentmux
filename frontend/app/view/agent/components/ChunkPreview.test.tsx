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
});
