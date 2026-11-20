// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const disk = vi.hoisted(() => ({ content: "one" as string | null, reads: 0 }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ReadEditorFileCommand: async () => {
            disk.reads++;
            if (disk.content === null) throw new Error("ENOENT");
            return { content: disk.content };
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { DiskConflicts, type DiskConflictTab } from "./editor-disk-conflict";
import { sha256Hex } from "./editor-hash";

function setup() {
    const calls: string[] = [];
    const conflicts = new DiskConflicts({
        on: () => ({}),
        reload: async (t) => void calls.push(`reload:${t.id}`),
        overwrite: async (t) => void calls.push(`overwrite:${t.id}`),
    });
    return { conflicts, calls };
}

/** A tab that loaded "one" from disk. */
async function tabOf(over: Partial<DiskConflictTab> = {}): Promise<DiskConflictTab> {
    return { id: "t1", filePath: "/p/settings.json", contentHash: await sha256Hex("one"), ...over };
}

describe("DiskConflicts", () => {
    beforeEach(() => {
        disk.content = "one";
        disk.reads = 0;
    });

    it("a change the watcher reports that leaves the content as the tab loaded it is not a conflict (the echo of its own save)", async () => {
        const { conflicts } = setup();
        await conflicts.noteChange(await tabOf());
        expect(conflicts.has("t1")).toBe(false);
    });

    it("a file that really changed under unsaved edits asks", async () => {
        const { conflicts } = setup();
        disk.content = "two";
        await conflicts.noteChange(await tabOf());
        expect(conflicts.has("t1")).toBe(true);
        expect(conflicts.has("other")).toBe(false);
    });

    it("a save over an unchanged file goes ahead, and over a changed file does not write", async () => {
        const { conflicts } = setup();
        const tab = await tabOf();
        expect(await conflicts.guardSave(tab)).toBe(true);
        disk.content = "two";
        expect(await conflicts.guardSave(tab)).toBe(false);
        expect(conflicts.has("t1")).toBe(true);
    });

    it("Keep mine after a save asked saves at once, over the other change", async () => {
        const { conflicts, calls } = setup();
        const tab = await tabOf();
        disk.content = "two";
        await conflicts.guardSave(tab);
        await conflicts.keep(tab);
        expect(calls).toEqual(["overwrite:t1"]);
        expect(conflicts.has("t1")).toBe(false);
    });

    it("Keep mine from the watcher's question only dismisses it, and the next save is not asked again", async () => {
        const { conflicts, calls } = setup();
        const tab = await tabOf();
        disk.content = "two";
        await conflicts.noteChange(tab);
        await conflicts.keep(tab);
        expect(calls).toEqual([]);
        expect(conflicts.has("t1")).toBe(false);
        expect(await conflicts.guardSave(tab)).toBe(true);
    });

    it("a file that changes again after Keep mine asks again", async () => {
        const { conflicts } = setup();
        const tab = await tabOf();
        disk.content = "two";
        await conflicts.noteChange(tab);
        await conflicts.keep(tab);
        disk.content = "three";
        expect(await conflicts.guardSave(tab)).toBe(false);
        expect(conflicts.has("t1")).toBe(true);
    });

    it("Reload hands the tab to the host and clears the question, and a save it was holding is dropped", async () => {
        const { conflicts, calls } = setup();
        const tab = await tabOf();
        disk.content = "two";
        await conflicts.guardSave(tab);
        await conflicts.reload(tab);
        expect(calls).toEqual(["reload:t1"]);
        expect(conflicts.has("t1")).toBe(false);
        await conflicts.keep(tab);
        expect(calls).toEqual(["reload:t1"]);
    });

    it("a file that cannot be read now, a scratch buffer and a tab that never loaded are not conflicts, and cost no read when nothing can differ", async () => {
        const { conflicts } = setup();
        disk.content = null;
        expect(await conflicts.guardSave(await tabOf())).toBe(true);
        disk.content = "two";
        disk.reads = 0;
        expect(await conflicts.guardSave(await tabOf({ isScratch: true }))).toBe(true);
        expect(await conflicts.guardSave(await tabOf({ contentHash: "" }))).toBe(true);
        expect(disk.reads).toBe(0);
    });

    it("closing a tab forgets its question", async () => {
        const { conflicts } = setup();
        disk.content = "two";
        await conflicts.noteChange(await tabOf());
        conflicts.forget("t1");
        expect(conflicts.has("t1")).toBe(false);
    });
});
