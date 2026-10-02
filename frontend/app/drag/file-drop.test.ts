// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window-level file-drop controller
 * (SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §4, §5.3).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({ consume: vi.fn(), peek: vi.fn() }));
vi.mock("@/util/dnd", () => ({
    baseName: (p: string) => p.split(/[\\/]/).pop() ?? p,
    isFileDrag: (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes("Files"),
    consumeDragPaths: () => hub.consume(),
    peekDragPaths: () => hub.peek(),
}));

import {
    beginPathDrag,
    endPathDrag,
    installFileDropController,
    PATHS_MIME,
    paneDropState,
    pathsMatchFiles,
    registerFileDropTarget,
    resetFileDropForTests,
    type FileDropHook,
} from "./file-drop";

const frame = () => new Promise((r) => setTimeout(r, 30));

function pane(id: string): HTMLElement {
    const el = document.createElement("div");
    el.setAttribute("data-role", "pane");
    el.setAttribute("data-blockid", id);
    el.getClientRects = () => [{}] as unknown as DOMRectList; // "visible"
    const inner = document.createElement("span");
    el.appendChild(inner);
    document.body.appendChild(el);
    return inner;
}

interface FakeTransfer {
    types: string[];
    items: { kind: string; type: string }[];
    files: File[];
    dropEffect: string;
}

function fire(type: string, target: EventTarget, dt?: Partial<FakeTransfer>, relatedTarget: EventTarget | null = target) {
    const e = new Event(type, { bubbles: true, cancelable: true }) as DragEvent;
    const transfer: FakeTransfer = {
        types: ["Files"],
        items: [{ kind: "file", type: "" }],
        files: [],
        dropEffect: "",
        ...dt,
    };
    Object.defineProperty(e, "dataTransfer", { value: transfer });
    Object.defineProperty(e, "relatedTarget", { value: relatedTarget });
    target.dispatchEvent(e);
    return { e, transfer };
}

const state = (id: string) => paneDropState(() => id)();

function hook(over: Partial<FileDropHook> & { ok?: boolean } = {}): FileDropHook & { accept: ReturnType<typeof vi.fn>; drop: ReturnType<typeof vi.fn> } {
    const ok = over.ok ?? true;
    return {
        accept: vi.fn(() => (ok ? { ok: true as const, message: "Drop here" } : { ok: false as const, reason: "Nope" })),
        drop: vi.fn(),
        ...over,
    } as never;
}

let uninstall: () => void;

beforeEach(() => {
    hub.consume.mockReset().mockResolvedValue([]);
    hub.peek.mockReset().mockResolvedValue([]);
    uninstall = installFileDropController(window);
});
afterEach(() => {
    uninstall();
    resetFileDropForTests();
    document.body.innerHTML = "";
});

describe("file-drop controller", () => {
    it("arms every accepting pane at the start and targets the one under the cursor", async () => {
        const a = pane("a");
        pane("b");
        pane("c"); // no hook: e.g. a sysinfo pane
        const ha = hook();
        const hb = hook();
        registerFileDropTarget("a", ha);
        registerFileDropTarget("b", hb);

        const { transfer } = fire("dragenter", a);
        await frame();
        expect(state("a")).toEqual({ state: "target", message: "Drop here", icon: undefined });
        expect(state("b")).toEqual({ state: "armed" });
        expect(state("c")).toBeUndefined();
        expect(transfer.dropEffect).toBe("copy");

        // Hovering doesn't re-ask: one verdict per pane per drag.
        for (let i = 0; i < 20; i++) fire("dragover", a);
        expect(ha.accept).toHaveBeenCalledTimes(1);
        expect(hb.accept).toHaveBeenCalledTimes(1);
    });

    it("shows a blocked pane's reason and refuses the drop there", async () => {
        const a = pane("a");
        const b = pane("b");
        registerFileDropTarget("a", hook());
        const hb = hook({ ok: false });
        registerFileDropTarget("b", hb);

        fire("dragenter", a);
        const { transfer } = fire("dragover", b);
        expect(transfer.dropEffect).toBe("none");
        await frame();
        expect(state("b")).toEqual({ state: "blocked", reason: "Nope" });
        expect(state("a")).toEqual({ state: "armed" });

        fire("drop", b, { files: [new File(["x"], "x.txt")] });
        await frame();
        expect(hb.drop).not.toHaveBeenCalled();
        expect(hub.consume).not.toHaveBeenCalled();
        expect(state("a")).toBeUndefined();
    });

    it("a release over another pane in the same frame goes to that pane, not the last target", async () => {
        const a = pane("a");
        const b = pane("b");
        const ha = hook();
        registerFileDropTarget("a", ha);
        registerFileDropTarget("b", hook({ ok: false }));
        fire("dragenter", a);
        fire("drop", b, { files: [new File(["x"], "x.txt")] });
        await frame();
        expect(ha.drop).not.toHaveBeenCalled();
        expect(hub.consume).not.toHaveBeenCalled();
    });

    it("drops with the host paths when they match the dropped files", async () => {
        const a = pane("a");
        const ha = hook();
        registerFileDropTarget("a", ha);
        hub.consume.mockResolvedValue(["C:\\docs\\report.pdf"]);
        const files = [new File(["x"], "report.pdf")];
        fire("dragenter", a);
        fire("drop", a, { files });
        await frame();
        expect(ha.drop).toHaveBeenCalledWith({ paths: ["C:\\docs\\report.pdf"], files });
    });

    it("falls back to the bytes when the stashed paths belong to another drag", async () => {
        const a = pane("a");
        const ha = hook();
        registerFileDropTarget("a", ha);
        hub.consume.mockResolvedValue(["/old/secret.txt"]);
        const files = [new File(["x"], "virtual.txt")];
        fire("drop", a, { files });
        await frame();
        expect(ha.drop).toHaveBeenCalledWith({ paths: [], files });
    });

    it("tells a paths-only hook it got none instead of dropping", async () => {
        const a = pane("a");
        const onNoPaths = vi.fn();
        const ha = hook({ needsPaths: true, onNoPaths });
        registerFileDropTarget("a", ha);
        fire("drop", a, { files: [new File(["x"], "x.txt")] });
        await frame();
        expect(ha.drop).not.toHaveBeenCalled();
        expect(onNoPaths).toHaveBeenCalledWith(1);
    });

    it("re-asks every verdict once the host reports the file names", async () => {
        const a = pane("a");
        const ha = hook();
        registerFileDropTarget("a", ha);
        hub.peek.mockResolvedValue(["/x/photo.png"]);
        fire("dragenter", a);
        await frame();
        expect(ha.accept).toHaveBeenCalledTimes(2);
        expect(ha.accept.mock.calls[1][0]).toMatchObject({ count: 1, names: ["photo.png"] });
    });

    it("ignores drags that carry no files", async () => {
        const a = pane("a");
        const ha = hook();
        registerFileDropTarget("a", ha);
        const { e } = fire("dragover", a, { types: ["application/vnd.pdnd"] });
        await frame();
        expect(e.defaultPrevented).toBe(false);
        expect(ha.accept).not.toHaveBeenCalled();
        expect(state("a")).toBeUndefined();
    });

    it("clears when the drag leaves the window", async () => {
        const a = pane("a");
        registerFileDropTarget("a", hook());
        fire("dragenter", a);
        await frame();
        expect(state("a")).toBeDefined();
        fire("dragleave", a, {}, null);
        await frame();
        expect(state("a")).toBeUndefined();
    });
});

describe("in-app path drags (a Hangar row; SPEC_FILE_BROWSER_PANE_2026_10_01.md §8.2)", () => {
    const start = (paths: string[]) => {
        const data = new Map<string, string>();
        const dt = { setData: (k: string, v: string) => data.set(k, v), effectAllowed: "" } as unknown as DataTransfer;
        beginPathDrag(dt, paths, "hangar");
        return data;
    };
    const pathTransfer = { types: [PATHS_MIME, "text/plain"], items: [], files: [] };

    it("puts the paths on the drag, as data and as text", () => {
        const data = start(["C:\\a\\x.ts", "C:\\a\\y.md"]);
        expect(JSON.parse(data.get(PATHS_MIME)!)).toEqual(["C:\\a\\x.ts", "C:\\a\\y.md"]);
        expect(data.get("text/plain")).toBe("C:\\a\\x.ts\nC:\\a\\y.md");
        endPathDrag();
    });

    it("targets panes with the names, and drops the paths with no file bytes", async () => {
        const a = pane("agent");
        const h = hook();
        registerFileDropTarget("agent", h);
        start(["C:\\a\\x.ts", "C:\\a\\y.md"]);
        const { transfer } = fire("dragenter", a, pathTransfer);
        await frame();
        expect(state("agent")).toEqual({ state: "target", message: "Drop here", icon: undefined });
        expect(h.accept).toHaveBeenCalledWith({ count: 2, types: ["", ""], names: ["x.ts", "y.md"] });
        expect(transfer.dropEffect).toBe("copy");
        fire("drop", a, pathTransfer);
        await frame();
        expect(h.drop).toHaveBeenCalledWith({ paths: ["C:\\a\\x.ts", "C:\\a\\y.md"], files: [] });
        // No OS drop happened: the host's stash is never consulted.
        expect(hub.consume).not.toHaveBeenCalled();
        endPathDrag();
    });

    it("ignores the MIME once the drag has ended (a stale drag from elsewhere)", async () => {
        const a = pane("agent");
        const h = hook();
        registerFileDropTarget("agent", h);
        start(["C:\\a\\x.ts"]);
        endPathDrag();
        fire("dragenter", a, pathTransfer);
        fire("drop", a, pathTransfer);
        await frame();
        expect(h.accept).not.toHaveBeenCalled();
        expect(h.drop).not.toHaveBeenCalled();
    });

    it("doesn't drop where the pane refuses", async () => {
        const a = pane("media");
        const h = hook({ ok: false });
        registerFileDropTarget("media", h);
        start(["C:\\a\\x.ts"]);
        fire("dragenter", a, pathTransfer);
        fire("drop", a, pathTransfer);
        await frame();
        expect(h.drop).not.toHaveBeenCalled();
        endPathDrag();
    });
});

describe("pathsMatchFiles", () => {
    it("compares base names as a set", () => {
        const f = (n: string) => new File([""], n);
        expect(pathsMatchFiles(["/a/x.txt", "C:\\b\\y.md"], [f("y.md"), f("x.txt")])).toBe(true);
        expect(pathsMatchFiles(["/a/x.txt"], [f("z.txt")])).toBe(false);
        expect(pathsMatchFiles([], [f("z.txt")])).toBe(false);
    });
});
