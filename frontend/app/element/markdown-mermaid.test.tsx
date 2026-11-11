// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

// Mermaid renders into a scratch container it appends to <body>. Like the real one, this stand-in leaves the
// container behind when the diagram fails to parse, and takes it away when the render succeeds.
vi.mock("mermaid", () => ({
    default: {
        initialize: vi.fn(),
        render: vi.fn(async (id: string, text: string) => {
            const scratch = document.createElement("div");
            scratch.id = `d${id}`;
            scratch.innerHTML = `<svg id="${id}"></svg>`;
            document.body.appendChild(scratch);
            if (text.includes("broken")) throw new Error("Syntax error in text");
            scratch.remove();
            return { svg: `<svg id="${id}"><text>diagram</text></svg>` };
        }),
    },
}));

import { Mermaid, removeMermaidScratch } from "./markdown-mermaid";

const scratchIds = () => [...document.body.children].filter((e) => /^dmermaid-/.test(e.id)).map((e) => e.id);

describe("a failed mermaid diagram leaves nothing in <body>", () => {
    afterEach(() => {
        cleanup();
        document.body.innerHTML = "";
        vi.spyOn(console, "error").mockRestore();
    });

    it("removes the scratch container when the diagram fails to parse, and shows the error", async () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        render(() => <Mermaid chart="broken diagram" />);
        await waitFor(() => expect(screen.getByText("Failed to render diagram")).toBeInTheDocument());
        expect(scratchIds()).toEqual([]);
    });

    it("keeps the diagram when it renders, and leaves nothing beside it", async () => {
        const { container } = render(() => <Mermaid chart="graph TD; A-->B" />);
        await waitFor(() => expect(container.querySelector(".mermaid svg")).not.toBeNull());
        expect(scratchIds()).toEqual([]);
    });
});

describe("removeMermaidScratch", () => {
    afterEach(() => {
        document.body.innerHTML = "";
    });

    it("removes the scratch container and a stray diagram directly in <body>, not one inside a pane", () => {
        const scratch = document.createElement("div");
        scratch.id = "dmermaid-9";
        const stray = document.createElement("svg");
        stray.id = "mermaid-9";
        const pane = document.createElement("div");
        const shown = document.createElement("svg");
        shown.id = "mermaid-4";
        pane.appendChild(shown);
        document.body.append(scratch, stray, pane);

        removeMermaidScratch("mermaid-9");
        removeMermaidScratch("mermaid-4");

        expect(document.getElementById("dmermaid-9")).toBeNull();
        expect(document.getElementById("mermaid-9")).toBeNull();
        expect(document.getElementById("mermaid-4")).toBe(shown);
    });
});
