// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The shared copy-into-working-folder helper and its notices. */

import { beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    cwd: "/work" as string | undefined,
    copy: vi.fn(),
    upload: vi.fn(),
    toast: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({
    MOS: { getObjectValue: () => ({ meta: { "cmd:cwd": hub.cwd } }), makeORef: (t: string, id: string) => `${t}:${id}` },
    pushNotification: (n: unknown) => hub.toast(n),
}));
vi.mock("@/util/dnd", () => ({
    baseName: (p: string) => p.split(/[\\/]/).pop() ?? p,
    copyFilesToDir: (...a: unknown[]) => hub.copy(...a),
}));
vi.mock("../view/agent/attachments/attachment-draft", () => ({
    uploadFileToWorkdir: (...a: unknown[]) => hub.upload(...a),
}));

import { copyIntoWorkdir } from "./file-drop-actions";

beforeEach(() => {
    hub.cwd = "/work";
    for (const f of [hub.copy, hub.upload, hub.toast]) f.mockReset();
});

const titles = () => hub.toast.mock.calls.map((c) => (c[0] as { title: string }).title);

describe("copyIntoWorkdir", () => {
    it("copies paths through the host and mentions them in the composer", async () => {
        hub.copy.mockResolvedValue({
            results: [
                { source: "/a/x.txt", dest: "/work/x.txt" },
                { source: "/a/y.txt", error: "denied" },
            ],
        });
        const splice = vi.fn(() => true);
        const root = document.createElement("div");
        const dests = await copyIntoWorkdir("b1", { paths: ["/a/x.txt", "/a/y.txt"] }, { paneKind: "agent pane", mentionIn: root, splice });
        expect(dests).toEqual(["/work/x.txt"]);
        expect(splice).toHaveBeenCalledWith(root, ["@x.txt"]);
        expect(titles()).toEqual(["Attached x.txt (1 failed)"]);
    });

    it("tells the user to mention files when the composer isn't there", async () => {
        hub.copy.mockResolvedValue({ results: [{ source: "/a/x.txt", dest: "/work/x.txt" }] });
        await copyIntoWorkdir("b1", { paths: ["/a/x.txt"] }, { paneKind: "agent pane", mentionIn: null, splice: vi.fn() });
        expect(hub.toast.mock.calls[0][0]).toMatchObject({
            title: "Copied x.txt",
            message: "Files are in /work. Mention them in your next message.",
        });
    });

    it("uses the bytes transport without host paths, one file at a time", async () => {
        hub.upload.mockResolvedValueOnce("/work/a.txt").mockRejectedValueOnce(new Error("disk full"));
        const files = [new File(["1"], "a.txt"), new File(["2"], "b.txt")];
        const dests = await copyIntoWorkdir("b1", { files }, { paneKind: "terminal pane" });
        expect(hub.upload).toHaveBeenCalledTimes(2);
        expect(dests).toEqual(["/work/a.txt"]);
        expect(hub.toast.mock.calls[0][0]).toMatchObject({
            title: "Copied a.txt to /work (1 failed)",
            message: "b.txt: disk full",
        });
    });

    it("reports a missing working folder and copies nothing", async () => {
        hub.cwd = undefined;
        await copyIntoWorkdir("b1", { paths: ["/a/x"] }, { paneKind: "terminal pane" });
        expect(hub.copy).not.toHaveBeenCalled();
        expect(hub.toast.mock.calls[0][0]).toMatchObject({ message: "No working directory detected for this terminal pane." });
    });

    it("reports a copy where everything failed", async () => {
        hub.copy.mockResolvedValue({ results: [{ source: "/a/x", error: "denied" }] });
        await copyIntoWorkdir("b1", { paths: ["/a/x"] }, { paneKind: "terminal pane" });
        expect(titles()).toEqual(["Copy failed (1 file)"]);
    });
});
