// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A click on an image of an agent's message: two or more local images open the preview that steps through them
 * (in reading order, not the missing ones, not video), one image goes to the Editor as before
 * (SPEC_AGENT_PANE_IMAGE_ARROWS_2026_11_06.md §3.1).
 */

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ fetch: vi.fn(), editor: vi.fn() }));

vi.mock("@/app/element/modal", () => ({
    Modal: (p: { ariaLabel?: string; children?: unknown }) => <div data-stand="Modal" data-aria-label={p.ariaLabel}>{p.children as any}</div>,
}));
vi.mock("@/util/fetchutil", () => ({ fetch: (...a: unknown[]) => h.fetch(...a) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv" }));
vi.mock("@/util/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
vi.mock("@/app/util/editor-open", () => ({ openInEditorPaneOnScreen: (...a: unknown[]) => h.editor(...a) }));

import type { MarkdownNode } from "../../../agent-model/types";
import { AgentMediaProvider } from "../agent-media";
import { MarkdownBlock } from "./MarkdownBlock";

const node = (content: string): MarkdownNode => ({ type: "markdown", id: "m1", content });

function mount(content: string) {
    return render(() => (
        <AgentMediaProvider baseDir={() => "D:/proj"}>
            <MarkdownBlock node={node(content)} />
        </AgentMediaProvider>
    ));
}

const shown = (c: HTMLElement) => c.querySelectorAll<HTMLImageElement>("img.am-media-img");
const modalLabel = (c: HTMLElement) => c.querySelector("[data-stand=Modal]")?.getAttribute("data-aria-label");

beforeEach(() => {
    h.editor.mockReset().mockResolvedValue(true);
    h.fetch.mockReset().mockImplementation(async (url: string) => {
        if (new URL(url).searchParams.get("path")?.endsWith("gone.png")) return { ok: false, status: 404, headers: new Headers() };
        return { ok: true, headers: new Headers({ "Content-Length": "4" }), blob: async () => new Blob(["png!"], { type: "image/png" }) };
    });
    vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: () => "blob:g", revokeObjectURL: () => {} }));
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

describe("a message's images", () => {
    it("open the preview on the clicked one, over the message's images in reading order", async () => {
        const { container } = mount("![one](a.png) text ![two](b.png) ![three](c.png)");
        await waitFor(() => expect(shown(container)).toHaveLength(3));
        fireEvent.click(shown(container)[1]);
        expect(modalLabel(container)).toBe("Image 2 of 3: two");
        expect(h.editor).not.toHaveBeenCalled();
    });

    it("leave out an image that is missing and a video", async () => {
        const { container } = mount("![one](a.png) ![x](gone.png) ![clip](v.mp4) ![two](b.png)");
        await waitFor(() => expect(container.textContent).toContain("[image not found: gone.png]"));
        await waitFor(() => expect(shown(container)).toHaveLength(2));
        fireEvent.click(shown(container)[1]);
        expect(modalLabel(container)).toBe("Image 2 of 2: two");
    });

    it("go to the Editor, as before, when the message has one image", async () => {
        const { container } = mount("![only](a.png) and a clip ![clip](v.mp4)");
        await waitFor(() => expect(shown(container)).toHaveLength(1));
        fireEvent.click(shown(container)[0]);
        expect(modalLabel(container)).toBeUndefined();
        expect(h.editor).toHaveBeenCalledWith("D:/proj/a.png", { followFolder: true });
    });

    it("are buttons for the keyboard, named with their place once focused", async () => {
        const { container } = mount("![one](a.png) ![two](b.png)");
        await waitFor(() => expect(shown(container)).toHaveLength(2));
        const img = shown(container)[1];
        expect(img.getAttribute("role")).toBe("button");
        expect(img.tabIndex).toBe(0);
        fireEvent.focus(img);
        expect(img.getAttribute("aria-label")).toBe("Open image: two, 2 of 2");
        fireEvent.keyDown(img, { key: "Enter" });
        expect(modalLabel(container)).toBe("Image 2 of 2: two");
    });
});
