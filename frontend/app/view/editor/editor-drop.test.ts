// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Dropping files onto an editor pane.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.3 ("Later panes").
 */

import { describe, expect, it, vi } from "vitest";
import { createEditorDropHook, editorDropVerdict, MAX_DROPPED_TEXT_BYTES, openEmptyScratch } from "./editor-drop";

const drag = (names?: string[], types: string[] = []) => ({ count: names?.length ?? types.length, types, names });

describe("editor pane file drop", () => {
    it("accepts text and code files, and says how many", () => {
        expect(editorDropVerdict(drag(["main.rs"]))).toEqual({ ok: true, message: "Open main.rs", icon: "fa-file-lines" });
        expect(editorDropVerdict(drag(["a.md", "b.json", "icon.svg"]))).toMatchObject({ ok: true, message: "Open 3 files" });
    });

    it("lets an unknown type through: the drop sniffs its content", () => {
        expect(editorDropVerdict(drag(["LICENSE", "Makefile"]))).toMatchObject({ ok: true });
        expect(editorDropVerdict(drag(undefined, [""]))).toMatchObject({ ok: true });
    });

    it("blocks any known binary", () => {
        expect(editorDropVerdict(drag(["notes.md", "photo.jpg"]))).toEqual({
            ok: false,
            reason: "The editor opens text files; photo.jpg isn't one",
        });
        expect(editorDropVerdict(drag(undefined, ["application/pdf"]))).toMatchObject({ ok: false });
        expect(editorDropVerdict(drag(undefined, ["image/png"]))).toMatchObject({ ok: false });
        expect(editorDropVerdict(drag(undefined, ["text/plain", "application/json"]))).toMatchObject({ ok: true });
    });

    it("opens every dropped path as a tab", async () => {
        const target = { openFile: vi.fn().mockResolvedValue(undefined), openText: vi.fn(), cantOpen: vi.fn() };
        await createEditorDropHook(target).drop({ paths: ["C:/a.rs", "C:/b.md"], files: [] });
        expect(target.openFile.mock.calls).toEqual([["C:/a.rs"], ["C:/b.md"]]);
    });

    it("without paths, opens each file's text in an untitled tab", async () => {
        const target = { openFile: vi.fn(), openText: vi.fn().mockResolvedValue(undefined), cantOpen: vi.fn() };
        const file = new File(["fn main() {}"], "main.rs");
        await createEditorDropHook(target).drop({ paths: [], files: [file] });
        expect(target.openText).toHaveBeenCalledWith("main.rs", "fn main() {}");
        expect(target.openFile).not.toHaveBeenCalled();
    });

    it("skips a binary it only discovers at drop, and reports it", async () => {
        const target = { openFile: vi.fn(), openText: vi.fn(), cantOpen: vi.fn() };
        const bin = new File([new Uint8Array([0x50, 0x4b, 0, 0])], "data");
        const txt = new File(["ok"], "readme");
        await createEditorDropHook(target).drop({ paths: [], files: [bin, txt] });
        expect(target.cantOpen).toHaveBeenCalledWith("data");
        expect(target.openText).toHaveBeenCalledTimes(1);
        expect(target.openText).toHaveBeenCalledWith("readme", "ok");
    });
});

describe("a pathless drop's size", () => {
    it("refuses a file over the editor's read limit without reading it", async () => {
        const target = { openFile: vi.fn(), openText: vi.fn(), cantOpen: vi.fn() };
        const big = new File(["a".repeat(MAX_DROPPED_TEXT_BYTES + 1)], "huge.log");
        const read = vi.spyOn(big, "text");
        const slice = vi.spyOn(big, "slice");
        await createEditorDropHook(target).drop({ paths: [], files: [big] });
        expect(target.cantOpen).toHaveBeenCalledWith("huge.log");
        expect(read).not.toHaveBeenCalled();
        expect(slice).not.toHaveBeenCalled();
        expect(target.openText).not.toHaveBeenCalled();
    });
});

describe("an untitled tab for dropped text", () => {
    it("uses the first empty scratch", async () => {
        const open = vi.fn().mockResolvedValue("t1");
        expect(await openEmptyScratch(open, () => "")).toBe("t1");
        expect(open).toHaveBeenCalledTimes(1);
    });

    it("never takes a reused scratch that still holds notes", async () => {
        const content: Record<string, string> = { old: "my unsaved notes", fresh: "" };
        const open = vi.fn().mockResolvedValueOnce("old").mockResolvedValueOnce("fresh");
        expect(await openEmptyScratch(open, (id) => content[id])).toBe("fresh");
    });

    it("gives up when it can't get an empty one", async () => {
        const open = vi.fn().mockResolvedValue("busy");
        expect(await openEmptyScratch(open, () => "notes", 3)).toBeUndefined();
        expect(open).toHaveBeenCalledTimes(3);
        expect(await openEmptyScratch(vi.fn().mockResolvedValue(undefined), () => "")).toBeUndefined();
    });
});
