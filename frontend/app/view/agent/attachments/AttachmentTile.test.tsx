// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tiles by kind (SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §5): an icon
 * with the extension for documents, a text preview rendered as text.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://x" }));

import { AttachmentTile, tileTooltip, type TileModel } from "./AttachmentTile";

afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

const tile = (over: Partial<TileModel>): TileModel => ({
    key: "k",
    number: 2,
    name: "report.pdf",
    bytes: 2048,
    status: "ready",
    id: "a".repeat(64),
    kind: "pdf",
    ...over,
});

describe("AttachmentTile", () => {
    it("shows a PDF as an icon with its extension, page count and macro badge", () => {
        const fetchSpy = vi.fn();
        vi.stubGlobal("fetch", fetchSpy);
        const { container } = render(() => (
            <AttachmentTile tile={tile({ pageCount: 12, macros: true })} onOpen={() => {}} />
        ));
        expect(container.querySelector(".fa-file-pdf")).not.toBeNull();
        expect(container.querySelector(".agent-attachment-tile__icon .ext")?.textContent).toBe("PDF");
        expect(container.querySelector(".agent-attachment-tile__pages")?.textContent).toBe("12 p");
        expect(container.querySelector(".agent-attachment-tile__warn")).not.toBeNull();
        expect(container.querySelector("img")).toBeNull();
        // Icon tiles fetch nothing.
        expect(fetchSpy).not.toHaveBeenCalled();
        expect(screen.getByRole("button", { name: /^File 2: report\.pdf/ })).toBeTruthy();
    });

    it("renders a text preview as text, never as markup", async () => {
        const preview = "<img src=x onerror=alert(1)>\nline 2";
        vi.stubGlobal(
            "fetch",
            vi.fn(async (url: string) =>
                url.startsWith("blob:") ? new Response(preview) : new Response(new Blob([preview])),
            ),
        );
        vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: () => "blob:preview", revokeObjectURL: () => {} }));
        const { container } = render(() => (
            <AttachmentTile tile={tile({ name: "notes.md", kind: "text", id: "b".repeat(64) })} onOpen={() => {}} />
        ));
        const page = await vi.waitFor(() => {
            const el = container.querySelector(".agent-attachment-tile__text");
            if (!el) throw new Error("not yet");
            return el;
        });
        expect(page.textContent).toBe(preview);
        expect(page.querySelector("img")).toBeNull();
    });

    it("describes documents in the tooltip", () => {
        expect(tileTooltip(tile({ pageCount: 1, macros: true, textNote: "no text version; save it as .docx" }))).toBe(
            "report.pdf · 2 KB · 1 page · Contains macros · no text version; save it as .docx",
        );
    });
});
