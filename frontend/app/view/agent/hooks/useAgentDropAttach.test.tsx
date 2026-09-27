// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's file-drop hook (SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md
 * §5.3): what it promises while hovering, and where a drop goes.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    settings: {} as Record<string, unknown>,
    meta: {} as Record<string, unknown>,
    registered: new Map<string, unknown>(),
    ingest: vi.fn(),
    upload: vi.fn(),
    copy: vi.fn(),
    attachFailed: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({
    getSettingsKeyAtom: (k: string) => () => hub.settings[k],
    MOS: { getObjectValue: () => ({ meta: hub.meta }), makeORef: (t: string, id: string) => `${t}:${id}` },
}));
vi.mock("@/app/drag/file-drop", () => ({
    registerFileDropTarget: (id: string, hook: unknown) => {
        hub.registered.set(id, hook);
        return () => hub.registered.delete(id);
    },
}));
vi.mock("@/app/drag/file-drop-actions", () => ({
    copyIntoWorkdir: (...a: unknown[]) => hub.copy(...a),
    fileCount: (n: number) => `${n} ${n === 1 ? "file" : "files"}`,
    notifyDrop: { attachFailed: (e: unknown) => hub.attachFailed(e) },
    paneWorkdir: () => hub.meta["cmd:cwd"],
}));
vi.mock("../attachments/attachment-draft", () => ({
    getAttachmentDraft: () => ({ ingestPaths: hub.ingest, uploadFiles: hub.upload }),
}));

import type { FileDropHook } from "@/app/drag/file-drop";
import { useAgentDropAttach } from "./useAgentDropAttach";

function mount(): FileDropHook {
    function Harness() {
        useAgentDropAttach({ blockId: "b1", rootRef: () => undefined });
        return <div />;
    }
    render(() => <Harness />);
    return hub.registered.get("b1") as FileDropHook;
}

const drag = { count: 2, types: ["", ""] };
const file = (name: string) => new File(["x"], name);

beforeEach(() => {
    hub.settings = {};
    hub.meta = {};
    hub.registered.clear();
    for (const f of [hub.ingest, hub.upload, hub.copy, hub.attachFailed]) f.mockReset();
    hub.ingest.mockResolvedValue([]);
});
afterEach(() => cleanup());

describe("agent pane file drop", () => {
    it("registers while mounted and unregisters on cleanup", () => {
        mount();
        expect(hub.registered.has("b1")).toBe(true);
        cleanup();
        expect(hub.registered.has("b1")).toBe(false);
    });

    it("attaches to the tray, with or without a working folder", async () => {
        const hook = mount();
        expect(hook.accept(drag)).toEqual({ ok: true, message: "Drop 2 files to attach", icon: "fa-paperclip" });
        await hook.drop({ paths: ["/a/x.pdf", "/a/y.md"], files: [file("x.pdf"), file("y.md")] });
        expect(hub.ingest).toHaveBeenCalledWith(["/a/x.pdf", "/a/y.md"]);
        expect(hub.copy).not.toHaveBeenCalled();
    });

    it("uploads the bytes when the host has no paths", async () => {
        const hook = mount();
        const files = [file("virtual.txt")];
        await hook.drop({ paths: [], files });
        expect(hub.upload).toHaveBeenCalledWith(files);
        expect(hub.ingest).not.toHaveBeenCalled();
    });

    it("copies what an older backend's ingest hands back", async () => {
        hub.ingest.mockResolvedValue(["/a/y.md"]);
        const hook = mount();
        await hook.drop({ paths: ["/a/x.png", "/a/y.md"], files: [file("x.png"), file("y.md")] });
        expect(hub.copy).toHaveBeenCalledWith("b1", { paths: ["/a/y.md"] }, expect.anything());
    });

    it("container agents copy into the working folder, or are blocked without one", async () => {
        hub.meta = { agentMode: "container" };
        const hook = mount();
        expect(hook.accept(drag)).toEqual({ ok: false, reason: "No working folder for this agent" });
        hub.meta = { agentMode: "container", "cmd:cwd": "/work" };
        expect(hook.accept(drag)).toEqual({ ok: true, message: "Copy 2 files to /work", icon: "fa-copy" });
        await hook.drop({ paths: [], files: [file("a.txt")] });
        expect(hub.copy).toHaveBeenCalledWith("b1", { files: [expect.any(File)] }, expect.anything());
    });

    it("says why when file drop is turned off", () => {
        hub.settings["dnd:enabled"] = false;
        expect(mount().accept(drag)).toEqual({ ok: false, reason: "File drop is turned off (dnd:enabled)" });
    });

    it("reports a failed attach", async () => {
        hub.ingest.mockRejectedValue(new Error("srv down"));
        await mount().drop({ paths: ["/a/x"], files: [file("x")] });
        expect(hub.attachFailed).toHaveBeenCalled();
    });
});
