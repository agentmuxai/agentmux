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

/** The tabs as the Editor holds them now (what `current` answers), which a test moves on while a read is in flight. */
const live = new Map<string, DiskConflictTab & { dirty?: boolean }>();

function setup() {
    const calls: string[] = [];
    const conflicts = new DiskConflicts({
        on: () => ({}),
        current: (id) => live.get(id),
        reload: async (t) => void calls.push(`reload:${t.id}`),
        overwrite: async (t) => void calls.push(`overwrite:${t.id}`),
    });
    return { conflicts, calls };
}

/** A tab with unsaved edits that loaded "one" from disk. */
async function tabOf(over: Partial<DiskConflictTab> = {}): Promise<DiskConflictTab> {
    const tab = { id: "t1", filePath: "/p/settings.json", contentHash: await sha256Hex("one"), ...over };
    live.set(tab.id, { ...tab, dirty: true });
    return tab;
}

describe("DiskConflicts", () => {
    beforeEach(() => {
        disk.content = "one";
        disk.reads = 0;
        live.clear();
    });

    it("the echo of the tab's own save that arrives while the write is in flight is not a conflict once the save has finished", async () => {
        const { conflicts } = setup();
        const tab = await tabOf(); // the event's snapshot: dirty, the hash from before the save
        disk.content = "two"; // the save's own write
        const noting = conflicts.noteChange(tab);
        live.set("t1", { ...tab, contentHash: await sha256Hex("two"), dirty: false }); // the save finished during the read
        await noting;
        expect(conflicts.has("t1")).toBe(false);
    });

    it("a tab that is clean after the read is not asked about, whatever the disk says", async () => {
        const { conflicts } = setup();
        const tab = await tabOf();
        disk.content = "two";
        const noting = conflicts.noteChange(tab);
        live.set("t1", { ...tab, dirty: false });
        await noting;
        expect(conflicts.has("t1")).toBe(false);
    });

    it("a save that goes through clears a question the echo raised while it was in flight", async () => {
        const { conflicts } = setup();
        disk.content = "two";
        await conflicts.noteChange(await tabOf());
        expect(conflicts.has("t1")).toBe(true);
        conflicts.forget("t1"); // the Editor's saveFile, once the write returned
        expect(conflicts.has("t1")).toBe(false);
    });

    it("Keep mine for a save that is waiting saves at once, with no read of the disk first", async () => {
        const { conflicts, calls } = setup();
        const tab = await tabOf();
        disk.content = "two";
        await conflicts.guardSave(tab);
        disk.reads = 0;
        await conflicts.keep(tab);
        expect(disk.reads).toBe(0);
        expect(calls).toEqual(["overwrite:t1"]);
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
