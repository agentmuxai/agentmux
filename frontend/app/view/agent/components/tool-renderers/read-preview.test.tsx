// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a Read shows when it returned something other than plain text: an
 * image, a PDF, a file unchanged since the last Read, or text with the CLI's
 * notes for the model on the end; and an MCP tool's screenshot.
 * docs/analysis/ANALYSIS_READ_TOOL_PREVIEW_2026_10_01.md §6.
 */

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ createBlock: vi.fn(async () => "new-block"), fsOpen: vi.fn<(client: unknown, req: { path: string }) => Promise<object>>(async () => ({})) }));
vi.mock("@/app/store/block-layout-actions", () => ({ createBlock: h.createBlock }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { FsOpenCommand: h.fsOpen } }));

import { ClaudeTranslator } from "../../providers/claude-translator";
import type { ToolNode } from "../../types";
import { ToolBlock } from "../ToolBlock";
import { withoutTrailingNotes } from "./builtins";

afterEach(() => {
    cleanup();
    h.createBlock.mockClear();
});

const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/** The result the translator makes from a tool_result and its structured sibling. */
function translated(content: unknown, structured?: unknown): ToolNode["result"] {
    const t = new ClaudeTranslator();
    const [event] = t.translate({
        type: "user",
        message: { role: "user", content: [{ type: "tool_result", tool_use_id: "r1", content }] },
        ...(structured ? { tool_use_result: structured } : {}),
    });
    return (event as { result: ToolNode["result"] }).result;
}

const node = (over: Partial<ToolNode>): ToolNode => ({
    type: "tool",
    id: "r1",
    tool: "Read",
    toolName: "Read",
    params: { file_path: "C:/shots/a.png" },
    status: "success",
    collapsed: false,
    summary: "",
    ...over,
});

const show = (n: ToolNode) => render(() => <ToolBlock node={n} pinned={true} onTogglePin={() => {}} />);

describe("a Read of an image", () => {
    const result = () =>
        translated([{ type: "image", source: { type: "base64", media_type: "image/png", data: PNG } }], {
            type: "image",
            file: {
                base64: PNG,
                type: "image/png",
                originalSize: 3546,
                dimensions: { originalWidth: 240, originalHeight: 160, displayWidth: 240, displayHeight: 160 },
            },
        });

    it("is kept as the image and its facts, not the raw blocks", () => {
        expect(result()).toEqual({
            content: "",
            images: [{ mediaType: "image/png", data: PNG }],
            file: { kind: "image", size: 3546, width: 240, height: 160 },
        });
    });

    it("shows the image with its type, dimensions and size", () => {
        const { container } = show(node({ result: result() }));
        const img = container.querySelector<HTMLImageElement>("img.agent-tool-image");
        expect(img?.getAttribute("src")).toBe(`data:image/png;base64,${PNG}`);
        expect(img?.style.aspectRatio).toBe("240 / 160");
        expect(container.querySelector(".agent-tool-read-facts")?.textContent).toBe("PNG image · 240 × 160 · 3.5 KB");
        // Not the old `0: {2 keys}` dump.
        expect(container.textContent).not.toContain("keys");
    });

    it("opens the file in a Media pane on click", async () => {
        const { container } = show(node({ result: result() }));
        fireEvent.click(container.querySelector("img.agent-tool-image")!);
        await waitFor(() =>
            expect(h.createBlock).toHaveBeenCalledWith({ meta: { view: "media", "media:path": "C:/shots/a.png" } })
        );
    });

    it("never puts a non-image type in a data URL", () => {
        const r = translated([{ type: "image", source: { type: "base64", media_type: "text/html", data: "PGI+" } }]);
        const { container } = show(node({ result: r }));
        expect(container.querySelector("img")).toBeNull();
    });
});

describe("a Read of a PDF", () => {
    const result = () =>
        translated(
            [
                { type: "text", text: "PDF file read: C:/docs/a.pdf (708.3KB)" },
                { type: "document", source: { type: "base64", media_type: "application/pdf", data: "JVBERi0xLjQ=" } },
            ],
            { type: "pdf", file: { filePath: "C:/docs/a.pdf", base64: "JVBERi0xLjQ=", originalSize: 725359 } }
        );

    it("drops the document's base64", () => {
        const r = result();
        expect(JSON.stringify(r)).not.toContain("JVBERi0xLjQ=");
        expect(r).toMatchObject({ content: "PDF file read: C:/docs/a.pdf (708.3KB)", file: { kind: "pdf", size: 725359 } });
    });

    it("shows one line with the size and the pages asked for, with Open and Show in folder", () => {
        const open = vi.fn();
        (window as unknown as { api: unknown }).api = { revealInFileExplorer: open };
        const { container } = show(node({ params: { file_path: "C:/docs/a.pdf", pages: "1-5" }, result: result() }));
        expect(container.querySelector(".agent-tool-read-facts span")?.textContent).toBe("PDF · pages 1-5 · 708.4 KB");
        expect(container.querySelector(".agent-highlighted-code")).toBeNull();
        const buttons = [...container.querySelectorAll(".agent-tool-read-open")];
        expect(buttons.map((b) => b.textContent)).toEqual(["Open", "Show in folder"]);
        fireEvent.click(buttons[1]);
        expect(open).toHaveBeenCalledWith("C:/docs/a.pdf");
        fireEvent.click(buttons[0]);
        expect(h.fsOpen.mock.lastCall?.[1]).toEqual({ path: "C:/docs/a.pdf" });
        delete (window as unknown as { api?: unknown }).api;
    });
});

describe("a Read of a file unchanged since the last Read", () => {
    it("is one muted line, not the note highlighted as code", () => {
        const r = translated("Wasted call — file unchanged since your last Read. Refer to that earlier tool_result instead.", {
            type: "file_unchanged",
            file: { filePath: "C:/a.ts" },
        });
        expect(r).toMatchObject({ file: { kind: "unchanged" } });
        const { container } = show(node({ params: { file_path: "C:/a.ts" }, result: r }));
        expect(container.querySelector(".agent-tool-read-facts")?.textContent).toBe("unchanged since the last Read");
        expect(container.textContent).not.toContain("Wasted call");
        expect(container.querySelector(".agent-highlighted-code")).toBeNull();
    });
});

describe("the CLI's notes on the end of a Read", () => {
    it("are taken off the preview, so the gutter survives", () => {
        const text =
            "     1\tconst a = 1;\n     2\tconst b = 2;\n\n<system-reminder>\n[Truncated: PARTIAL view — showing lines 1-2 of 9 total]\n</system-reminder>\n";
        expect(withoutTrailingNotes(text)).toBe("     1\tconst a = 1;\n     2\tconst b = 2;");
        const { container } = show(node({ params: { file_path: "C:/a.ts" }, result: { content: text } }));
        const code = container.querySelector(".agent-tool-read-content")?.textContent ?? "";
        expect(code).not.toContain("system-reminder");
        expect(code).not.toContain("\t");
        // The range line still says the read was cut short.
        expect(container.querySelector(".agent-tool-read-range")?.textContent).toBe("lines 1–2 of 9 · cut off at the token cap");
    });

    it("leaves a tag the file itself mentions", () => {
        const text = "     1\tthe <system-reminder>tag</system-reminder> in a doc\n     2\tmore";
        expect(withoutTrailingNotes(text)).toBe(text);
    });
});

describe("an MCP tool's screenshot", () => {
    it("shows the image above the result's text", () => {
        const r = translated([
            { type: "image", source: { type: "base64", media_type: "image/png", data: PNG } },
            { type: "text", text: "captured window 3" },
        ]);
        expect(r).toEqual({ content: "captured window 3", images: [{ mediaType: "image/png", data: PNG }] });
        const { container } = show(node({ tool: "Other", toolName: "mcp__agentmux__UIScreenshot", params: {}, result: r }));
        const img = container.querySelector<HTMLImageElement>("img.agent-tool-image")!;
        expect(img).not.toBeNull();
        expect(container.textContent).toContain("captured window 3");
        // No file to open: a click shows it at full size instead.
        fireEvent.click(img);
        expect(img.classList.contains("agent-tool-image-full")).toBe(true);
        expect(h.createBlock).not.toHaveBeenCalled();
    });
});
