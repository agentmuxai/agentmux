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

import { copyIntoWorkdir, settleWithLimit } from "./file-drop-actions";

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

describe("settleWithLimit", () => {
    it("runs at most `limit` at once and keeps results in order", async () => {
        let running = 0;
        let peak = 0;
        const out = await settleWithLimit([1, 2, 3, 4, 5, 6], 2, async (n) => {
            running++;
            peak = Math.max(peak, running);
            await new Promise((r) => setTimeout(r, 5));
            running--;
            if (n === 4) throw new Error("four");
            return n * 10;
        });
        expect(peak).toBe(2);
        expect(out.map((r) => (r.status === "fulfilled" ? r.value : "x"))).toEqual([10, 20, 30, "x", 50, 60]);
    });

    it("the bytes transport honours dnd:concurrency", async () => {
        let running = 0;
        let peak = 0;
        hub.upload.mockImplementation(async (_b: string, f: File) => {
            running++;
            peak = Math.max(peak, running);
            await new Promise((r) => setTimeout(r, 5));
            running--;
            return `/work/${f.name}`;
        });
        const files = Array.from({ length: 6 }, (_, i) => new File(["x"], `f${i}.txt`));
        const dests = await copyIntoWorkdir("b1", { files }, { paneKind: "terminal pane", concurrency: 3 });
        expect(peak).toBe(3);
        expect(dests).toHaveLength(6);
    });

    it("the bytes transport is unlimited when dnd:concurrency is blank", async () => {
        let running = 0;
        let peak = 0;
        hub.upload.mockImplementation(async (_b: string, f: File) => {
            running++;
            peak = Math.max(peak, running);
            await new Promise((r) => setTimeout(r, 5));
            running--;
            return `/work/${f.name}`;
        });
        const files = Array.from({ length: 9 }, (_, i) => new File(["x"], `f${i}.txt`));
        await copyIntoWorkdir("b1", { files }, { paneKind: "terminal pane" });
        expect(peak).toBe(9);
    });
});
