// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Media in a message of a container agent: the pane's id goes with every read, so srv answers from the agent's own
 * folders, and a click on the file is opened by srv (`openMessageFile`), never as the path the message wrote. A
 * message of a native agent is unchanged: its click opens the Editor on screen.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    reads: [] as { what: string; path: string; blockId?: string }[],
    opened: [] as [string, string | undefined][],
    editor: [] as string[],
    created: [] as unknown[],
    refuse: undefined as string | undefined,
}));

vi.mock("@/app/element/local-media", async () => {
    const actual = await vi.importActual<typeof import("@/app/element/local-media")>("@/app/element/local-media");
    return {
        ...actual,
        // Both reads say the file is over the inline cap, which leaves a card to click and nothing else to settle.
        fetchMediaBlob: async (path: string, opts: { blockId?: string }) => {
            h.reads.push({ what: "blob", path, blockId: opts?.blockId });
            if (h.refuse) throw new actual.MediaFetchError(403, "Forbidden", h.refuse);
            throw new actual.MediaTooLargeError(300_000_000);
        },
        fetchMediaRange: async (path: string, _s: number, _e: number, opts: { blockId?: string }) => {
            h.reads.push({ what: "range", path, blockId: opts?.blockId });
            return { blob: null, total: 300_000_000 };
        },
    };
});
vi.mock("@/app/element/open-message-file", () => ({ openMessageFile: (path: string, blockId: string | undefined) => h.opened.push([path, blockId]) }));
vi.mock("@/app/util/editor-open", () => ({ openInEditorPaneOnScreen: async (path: string) => (h.editor.push(path), true) }));
vi.mock("@/app/store/block-layout-actions", () => ({ createBlock: async (req: unknown) => (h.created.push(req), "new-block") }));
vi.mock("@/app/store/global", () => ({ openLink: () => {} }));
vi.mock("@/app/element/tip", () => ({ tip: (text: string) => ({ "data-tip": text }) }));
vi.mock("@/element/ui", () => ({
    Button: (p: { class?: string; onClick?: () => void; children?: JSX.Element }) => (
        <span data-stand="Button" class={p.class} onClick={() => p.onClick?.()}>
            {p.children}
        </span>
    ),
}));

import { MarkdownImg, openMediaInEditor } from "./markdown-media";

const imgProps = (src: string) => ({ src, alt: "a picture" }) as JSX.ImgHTMLAttributes<HTMLImageElement>;
const card = (container: HTMLElement) => container.querySelector(".am-media-card") as HTMLElement;

beforeEach(() => {
    h.reads.length = h.opened.length = h.editor.length = h.created.length = 0;
    h.refuse = undefined;
});
afterEach(cleanup);

describe("a container agent's message", () => {
    const media = { baseDir: "/ws", blockId: "pane-1", confined: true };

    it("reads a picture with the pane's id and has srv open it on a click", async () => {
        const { container } = render(() => <MarkdownImg props={imgProps("/workspace/big.png")} media={media} />);
        await vi.waitFor(() => expect(card(container)).not.toBeNull());
        expect(h.reads).toEqual([{ what: "blob", path: "/workspace/big.png", blockId: "pane-1" }]);
        fireEvent.click(card(container));
        expect(h.opened).toEqual([["/workspace/big.png", "pane-1"]]);
        expect(h.editor).toEqual([]);
        expect(h.created).toEqual([]);
    });

    it("reads a video's poster range and its file with the pane's id and has srv open it on a click", async () => {
        const { container } = render(() => <MarkdownImg props={imgProps("/workspace/clip.mp4")} media={media} />);
        await vi.waitFor(() => expect(card(container)).not.toBeNull());
        expect(h.reads).toEqual([{ what: "range", path: "/workspace/clip.mp4", blockId: "pane-1" }]);
        fireEvent.click(card(container));
        expect(h.opened).toEqual([["/workspace/clip.mp4", "pane-1"]]);
        expect(h.editor).toEqual([]);
    });

    it("resolves a relative path against the working directory and still asks for it by the pane", async () => {
        const { container } = render(() => <MarkdownImg props={imgProps("out/big.png")} media={media} />);
        await vi.waitFor(() => expect(card(container)).not.toBeNull());
        expect(h.reads[0]).toEqual({ what: "blob", path: "/ws/out/big.png", blockId: "pane-1" });
    });

    it("a pane not known to be a native agent's is confined: nothing here opens its path", async () => {
        const unknown = { baseDir: "/ws", blockId: "pane-1" };
        const { container } = render(() => <MarkdownImg props={imgProps("/workspace/big.png")} media={{ ...unknown, confined: true }} />);
        await vi.waitFor(() => expect(card(container)).not.toBeNull());
        fireEvent.click(card(container));
        expect(h.editor).toEqual([]);
        expect(h.opened).toHaveLength(1);
    });
});

describe("a file srv refuses to show", () => {
    it("says in srv's words why it isn't shown", async () => {
        h.refuse = "This file isn't in a folder the agent shares with its container.";
        const { container } = render(() => <MarkdownImg props={imgProps("/home/me/secret.png")} media={{ baseDir: "/ws", blockId: "pane-1", confined: true }} />);
        await vi.waitFor(() => expect(container.textContent).toContain("image not found"));
        expect(container.textContent).toContain("/home/me/secret.png");
        expect(container.textContent).toContain("isn't in a folder the agent shares");
    });
});

describe("a confined message with no pane to name", () => {
    const media = { baseDir: "/ws", confined: true };

    it("reads no picture and shows none", () => {
        const { container } = render(() => <MarkdownImg props={imgProps("/workspace/big.png")} media={media} />);
        expect(h.reads).toEqual([]);
        expect(container.querySelector(".am-media-card")).toBeNull();
        expect(container.textContent).toContain("not shown here");
    });

    it("probes and reads no video", () => {
        const { container } = render(() => <MarkdownImg props={imgProps("/workspace/clip.mp4")} media={media} />);
        expect(h.reads).toEqual([]);
        expect(container.textContent).toContain("not shown here");
    });

    it("opens nothing on a click anywhere (card, preview, tool-result picture all go through the same open)", () => {
        openMediaInEditor("/workspace/big.png", { confined: true });
        expect(h.editor).toEqual([]);
        expect(h.created).toEqual([]);
        // The helper that opens a message's file is told there is no pane, and refuses itself.
        expect(h.opened).toEqual([["/workspace/big.png", undefined]]);
    });
});

describe("a native agent's message", () => {
    const media = { baseDir: "/ws", blockId: "pane-2", confined: false };

    it("is read with the pane's id too, srv deciding by the pane, and a click opens the Editor on screen as before", async () => {
        const { container } = render(() => <MarkdownImg props={imgProps("/ws/big.png")} media={media} />);
        await vi.waitFor(() => expect(card(container)).not.toBeNull());
        expect(h.reads).toEqual([{ what: "blob", path: "/ws/big.png", blockId: "pane-2" }]);
        fireEvent.click(card(container));
        await vi.waitFor(() => expect(h.editor).toEqual(["/ws/big.png"]));
        expect(h.opened).toEqual([]);
    });

    it("a message with no pane to name (not in an agent pane) reads with no id and opens the Editor as before", async () => {
        const { container } = render(() => <MarkdownImg props={imgProps("/ws/big.png")} media={{ baseDir: "/ws" }} />);
        await vi.waitFor(() => expect(card(container)).not.toBeNull());
        expect(h.reads).toEqual([{ what: "blob", path: "/ws/big.png", blockId: undefined }]);
        fireEvent.click(card(container));
        await vi.waitFor(() => expect(h.editor).toEqual(["/ws/big.png"]));
        expect(h.opened).toEqual([]);
    });
});
