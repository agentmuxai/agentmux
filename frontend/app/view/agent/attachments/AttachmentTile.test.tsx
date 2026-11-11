// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tiles by kind (SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §5): an icon
 * with the extension for documents, a text preview rendered as text.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createSignal, Show, Suspense } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/util/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
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
        // The text is read from the fetched file itself, not fetched again from its object URL.
        vi.stubGlobal("fetch", vi.fn(async () => new Response(preview)));
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

    // The pane's view sits under the block's Suspense: a resource read while it loads takes the whole view out of
    // the document, so its scroller reads zero size and loses its place, and a strip that drops and rebuilds its
    // tiles starts the read again, every frame (#47406).
    it("never suspends the view around it while its text loads, and has the text at once when shown again", async () => {
        const preview = ["first line", "second line"].join(String.fromCharCode(10));
        const fetchSpy = vi.fn(async () => new Response(preview));
        vi.stubGlobal("fetch", fetchSpy);
        vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: () => "blob:notes", revokeObjectURL: () => {} }));
        const fallbackShown = vi.fn();
        const Fallback = () => (fallbackShown(), (<div>loading the pane</div>));
        const [shown, setShown] = createSignal(true);
        const notes = tile({ name: "notes.txt", kind: "text", id: "c".repeat(64) });
        const { container } = render(() => (
            <Suspense fallback={<Fallback />}>
                <div class="transcript">
                    <Show when={shown()}>
                        <AttachmentTile tile={notes} onOpen={() => {}} />
                    </Show>
                </div>
            </Suspense>
        ));
        const text = () => container.querySelector(".agent-attachment-tile__text")?.textContent;
        await vi.waitFor(() => expect(text()).toBe(preview));
        expect(fallbackShown).not.toHaveBeenCalled();

        // The strip dropped its tiles and built them again: same file, no second read, no wait.
        setShown(false);
        setShown(true);
        await Promise.resolve();
        expect(text()).toBe(preview);
        expect(fetchSpy).toHaveBeenCalledTimes(1);
        expect(fallbackShown).not.toHaveBeenCalled();
    });

    it("describes documents in the tooltip", () => {
        expect(tileTooltip(tile({ pageCount: 1, macros: true, textNote: "no text version; save it as .docx" }))).toBe(
            "report.pdf · 2.0 KB · 1 page · Contains macros · no text version; save it as .docx",
        );
    });
});
