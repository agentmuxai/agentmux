// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The composer with image attachments (SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_
 * 2026_09_26.md §5): what a send carries, when it waits, and paste.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    ingest: vi.fn(),
    info: vi.fn(),
    container: false,
    splice: vi.fn(),
    copy: vi.fn(),
    attachmentsEnabled: undefined as boolean | undefined,
}));
vi.mock("@/app/store/global", async (orig) => {
    const real = (await orig()) as { getSettingsKeyAtom: (key: string) => () => unknown };
    return {
        ...real,
        getSettingsKeyAtom: (key: string) =>
            key === "attachments:enabled" ? () => hub.attachmentsEnabled : real.getSettingsKeyAtom(key),
    };
});
vi.mock("@/app/drag/file-drop-actions", () => ({
    copyIntoWorkdir: (...a: unknown[]) => hub.copy(...a),
}));
vi.mock("../hooks/useAgentDropAttach", () => ({
    isContainerPane: () => hub.container,
    spliceComposerTokens: (...a: unknown[]) => hub.splice(...a),
}));
vi.mock("@/app/store/rpc-api", async (orig) => {
    const real = (await orig()) as { RpcApi: Record<string, unknown> };
    return {
        ...real,
        RpcApi: {
            ...real.RpcApi,
            AttachmentsIngestCommand: (...a: unknown[]) => hub.ingest(...a),
            AttachmentsInfoCommand: (...a: unknown[]) => hub.info(...a),
            AttachmentsCancelCommand: () => Promise.resolve(),
        },
    };
});

import { AgentFooter } from "./AgentFooter";
import type { AgentViewModel } from "../agent-model";
import { getAttachmentDraft } from "../attachments/attachment-draft";
import { dispatch as dispatchPaneCommand, registerPane, unregisterPane } from "@/app/store/agent-pane-state-store";

afterEach(() => cleanup());

let blockSeq = 0;
function setup(onSendMessage = vi.fn()) {
    const blockId = `att-block-${blockSeq++}`;
    const vm = {
        blockId,
        blockAtom: () => ({ meta: {} }) as any,
        voiceTargetRef: { current: null },
        focusTargetRef: { current: null },
    } as unknown as AgentViewModel;
    render(() => <AgentFooter agentName="Test" viewModel={vm} onSendMessage={onSendMessage} />);
    const ta = screen.getByPlaceholderText(/Send message to/) as HTMLTextAreaElement;
    return { ta, onSendMessage, draft: getAttachmentDraft(blockId), blockId };
}

const ID = "e".repeat(64);
const readyInfo = {
    id: ID,
    name: "",
    mime: "image/png",
    bytes: 1200,
    width: 40,
    height: 30,
    send_mime: "image/png",
    send_bytes: 1200,
    send_width: 40,
    send_height: 30,
    first_frame_only: false,
};

function enter(el: Element) {
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
}

describe("AgentFooter with attachments", () => {
    it("sends ready images with the text, then empties the tray", async () => {
        const { ta, onSendMessage, draft } = setup();
        hub.info.mockResolvedValue({ items: [readyInfo] });
        await draft.restoreRefs([{ id: ID, name: "shot.png" }]);
        expect(screen.getByText(/1 image · /)).toBeTruthy();

        const user = userEvent.setup();
        await user.click(ta);
        await user.type(ta, "look");
        enter(ta);
        expect(onSendMessage).toHaveBeenCalledWith("look", [{ id: ID, name: "shot.png" }]);
        expect(draft.count()).toBe(0);
    });

    it("sends images with no text at all", async () => {
        const { ta, onSendMessage, draft } = setup();
        hub.info.mockResolvedValue({ items: [readyInfo] });
        await draft.restoreRefs([{ id: ID, name: "shot.png" }]);
        ta.focus();
        enter(ta);
        expect(onSendMessage).toHaveBeenCalledWith("", [{ id: ID, name: "shot.png" }]);
    });

    it("keeps the message while the conversation is still loading, and says so", async () => {
        const { ta, onSendMessage, blockId } = setup();
        registerPane(blockId, "agent-1"); // a pane starts InitPending
        try {
            const user = userEvent.setup();
            await user.click(ta);
            await user.type(ta, "hello");
            enter(ta);
            expect(onSendMessage).not.toHaveBeenCalled();
            expect(screen.getByRole("status").textContent).toMatch(/Waiting for the conversation to load/);
            expect(ta.value).toBe("hello");
            // Once the history is in, the same message sends.
            dispatchPaneCommand(blockId, { type: "InitReady", at: Date.now() }, "system");
            enter(ta);
            expect(onSendMessage).toHaveBeenCalledWith("hello");
        } finally {
            unregisterPane(blockId);
        }
    });

    it("waits while an image is still processing, and says so", async () => {
        const { ta, onSendMessage, draft } = setup();
        hub.ingest.mockImplementation(async (_c: unknown, req: { batch_id: string }) => ({
            batch_id: req.batch_id,
            accepted: [{ index: 0, path: "/p/big.png", name: "big.png", bytes: 5000 }],
            rejected: [],
            non_images: [],
            total_bytes: 5000,
            max_files: 128,
            max_total_bytes: 1024 * 1024 * 1024,
        }));
        await draft.ingestPaths(["/p/big.png"]);
        const user = userEvent.setup();
        await user.click(ta);
        await user.type(ta, "hi");
        enter(ta);
        expect(onSendMessage).not.toHaveBeenCalled();
        expect(screen.getByRole("status").textContent).toMatch(/Waiting for 1 image to finish processing/);
        expect(ta.value).toBe("hi");
    });

    it("attaches pasted images without cancelling the paste", () => {
        const { ta, draft } = setup();
        const upload = vi.spyOn(draft, "uploadFiles").mockImplementation(() => {});
        const file = new File([new Uint8Array([1, 2, 3])], "image.png", { type: "image/png" });
        const ev = new Event("paste", { bubbles: true, cancelable: true });
        Object.defineProperty(ev, "clipboardData", { value: { files: [file] } });
        ta.dispatchEvent(ev);
        expect(upload).toHaveBeenCalledWith([file]);
        // The text part of a mixed paste is left to the browser.
        expect(ev.defaultPrevented).toBe(false);
    });

    it("container panes: a paste is copied into the working folder, like a drop", () => {
        const { ta, draft } = setup();
        hub.container = true;
        hub.copy.mockReset().mockResolvedValue([]);
        const tray = vi.spyOn(draft, "uploadFiles");
        const files = ["a.txt", "b.bin"].map((n) => new File([new Uint8Array([1])], n));
        const ev = new Event("paste", { bubbles: true, cancelable: true });
        Object.defineProperty(ev, "clipboardData", { value: { files } });
        try {
            ta.dispatchEvent(ev);
            expect(hub.copy).toHaveBeenCalledTimes(1);
            const [blockId, source, opts] = hub.copy.mock.calls[0];
            expect(blockId).toBe(draft.blockId);
            expect(source).toEqual({ files });
            expect(opts.paneKind).toBe("agent pane");
            expect(opts.mentionIn).toBe(ta.parentElement);
            expect(opts.concurrency).toBeUndefined();
            opts.splice(ta.parentElement, ["@a.txt"]);
            expect(hub.splice).toHaveBeenCalledWith(ta.parentElement, ["@a.txt"]);
            expect(tray).not.toHaveBeenCalled();
        } finally {
            hub.container = false;
        }
    });

    it("attachments:enabled off: a paste is copied into the working folder, like a drop", () => {
        const { ta, draft } = setup();
        hub.attachmentsEnabled = false;
        hub.copy.mockReset().mockResolvedValue([]);
        const tray = vi.spyOn(draft, "uploadFiles");
        const file = new File([new Uint8Array([1, 2, 3])], "image.png", { type: "image/png" });
        const ev = new Event("paste", { bubbles: true, cancelable: true });
        Object.defineProperty(ev, "clipboardData", { value: { files: [file] } });
        try {
            ta.dispatchEvent(ev);
            expect(hub.copy).toHaveBeenCalledTimes(1);
            expect(hub.copy.mock.calls[0][1]).toEqual({ files: [file] });
            expect(tray).not.toHaveBeenCalled();
        } finally {
            hub.attachmentsEnabled = undefined;
        }
    });

    it("keeps the one-argument send for a plain message", async () => {
        const { ta, onSendMessage } = setup();
        const user = userEvent.setup();
        await user.click(ta);
        await user.type(ta, "plain");
        enter(ta);
        expect(onSendMessage).toHaveBeenCalledWith("plain");
    });
});
