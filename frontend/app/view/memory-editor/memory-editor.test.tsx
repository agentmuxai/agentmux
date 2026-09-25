// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The shared memory-editor pieces: the pinned two-region layout and its
// remembered split, the keyboard contract, the client-side line diff, and
// the draft model's base/conflict rules.
// docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.4.

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { handleMemoryEditorKeyDown } from "./editor-keys";
import { lineDiff } from "./line-diff";
import { MemoryDraftModel } from "./memory-draft-model";
import { loadStoredSplit, PinnedEditorLayout, splitStorageKey } from "./PinnedEditorLayout";

afterEach(() => cleanup());
beforeEach(() => localStorage.clear());

describe("PinnedEditorLayout", () => {
    const mount = (surface: string) =>
        render(() => (
            <PinnedEditorLayout surface={surface} top={<button>Save</button>} bottom={<textarea aria-label="ed" />} />
        ));

    test("renders the top region, the drag handle, then the bottom region, in that order", () => {
        mount("s1");
        const root = document.querySelector(".memory-pinned-layout")!;
        const kids = Array.from(root.children).map((c) => c.className);
        expect(kids).toEqual(["memory-pinned-top", "memory-pinned-handle", "memory-pinned-bottom"]);
        expect(screen.getByTestId("memory-pinned-top").textContent).toBe("Save");
        expect(screen.getByTestId("memory-pinned-bottom").querySelector("textarea")).not.toBeNull();
        // Content-sized until the user drags.
        expect(screen.getByTestId("memory-pinned-top").style.height).toBe("");
    });

    test("dragging the handle sets the split as a fraction and remembers it per surface", () => {
        mount("s-drag");
        const root = document.querySelector<HTMLElement>(".memory-pinned-layout")!;
        vi.spyOn(root, "getBoundingClientRect").mockReturnValue({ top: 100, height: 400 } as DOMRect);
        const handle = screen.getByRole("separator");

        fireEvent.pointerDown(handle, { button: 0, clientY: 200 });
        fireEvent.pointerMove(window, { clientY: 300 });
        fireEvent.pointerUp(window);

        expect(screen.getByTestId("memory-pinned-top").style.height).toBe("50%");
        expect(localStorage.getItem(splitStorageKey("s-drag"))).toBe("0.5");
        expect(loadStoredSplit("some-other-surface")).toBeNull();
    });

    test("a remembered split is restored on the next mount of the same surface only", () => {
        localStorage.setItem(splitStorageKey("s-a"), "0.3");
        mount("s-a");
        expect(screen.getByTestId("memory-pinned-top").style.height).toBe("30%");
        cleanup();
        mount("s-b");
        expect(screen.getByTestId("memory-pinned-top").style.height).toBe("");
    });

    test("arrow keys on the handle resize it; double-click resets to content-sized", () => {
        localStorage.setItem(splitStorageKey("s-k"), "0.5");
        mount("s-k");
        const handle = screen.getByRole("separator");
        fireEvent.keyDown(handle, { key: "ArrowUp" });
        expect(screen.getByTestId("memory-pinned-top").style.height).toBe("45%");
        expect(localStorage.getItem(splitStorageKey("s-k"))).toBe("0.45");

        fireEvent.dblClick(handle);
        expect(screen.getByTestId("memory-pinned-top").style.height).toBe("");
        expect(localStorage.getItem(splitStorageKey("s-k"))).toBeNull();
    });

    test("ignores a corrupt or out-of-range stored split", () => {
        localStorage.setItem(splitStorageKey("bad"), "7");
        expect(loadStoredSplit("bad")).toBeNull();
        localStorage.setItem(splitStorageKey("bad"), "nope");
        expect(loadStoredSplit("bad")).toBeNull();
    });
});

describe("handleMemoryEditorKeyDown", () => {
    const key = (init: KeyboardEventInit) => new KeyboardEvent("keydown", { cancelable: true, ...init });
    const handlers = (over: Partial<Parameters<typeof handleMemoryEditorKeyDown>[1]> = {}) => ({
        isEditing: () => true,
        isDirty: () => false,
        onSave: vi.fn(),
        onCancel: vi.fn(),
        confirm: vi.fn(() => true),
        ...over,
    });

    test("Ctrl+S and Cmd+S save and swallow the browser's own save", () => {
        for (const init of [{ key: "s", ctrlKey: true }, { key: "s", metaKey: true }]) {
            const h = handlers();
            const e = key(init);
            handleMemoryEditorKeyDown(e, h);
            expect(h.onSave).toHaveBeenCalledTimes(1);
            expect(e.defaultPrevented).toBe(true);
        }
    });

    test("plain s, Ctrl+Shift+S and Alt combos are not saves", () => {
        for (const init of [{ key: "s" }, { key: "s", ctrlKey: true, shiftKey: true }, { key: "s", ctrlKey: true, altKey: true }]) {
            const h = handlers();
            handleMemoryEditorKeyDown(key(init), h);
            expect(h.onSave).not.toHaveBeenCalled();
        }
    });

    test("Esc cancels a clean draft without asking, and asks first for a dirty one", () => {
        const clean = handlers();
        handleMemoryEditorKeyDown(key({ key: "Escape" }), clean);
        expect(clean.confirm).not.toHaveBeenCalled();
        expect(clean.onCancel).toHaveBeenCalledTimes(1);

        const refused = handlers({ isDirty: () => true, confirm: vi.fn(() => false) });
        handleMemoryEditorKeyDown(key({ key: "Escape" }), refused);
        expect(refused.confirm).toHaveBeenCalledTimes(1);
        expect(refused.onCancel).not.toHaveBeenCalled();
    });

    test("does nothing when no editor is open", () => {
        const h = handlers({ isEditing: () => false });
        const e = key({ key: "s", ctrlKey: true });
        handleMemoryEditorKeyDown(e, h);
        handleMemoryEditorKeyDown(key({ key: "Escape" }), h);
        expect(h.onSave).not.toHaveBeenCalled();
        expect(h.onCancel).not.toHaveBeenCalled();
        expect(e.defaultPrevented).toBe(false);
    });
});

describe("lineDiff", () => {
    // Same output as the backend's line_diff for the same inputs
    // (native_memory_handlers.rs), so one renderer serves both.
    test("matches the backend's format", () => {
        expect(lineDiff("a\nb\n", "a\nc\n")).toBe("  a\n- b\n+ c\n");
        expect(lineDiff("", "x")).toBe("+ x\n");
        expect(lineDiff("x\r\ny", "x\ny")).toBe("  x\n  y\n");
        expect(lineDiff("same", "same")).toBe("  same\n");
    });
});

describe("MemoryDraftModel", () => {
    // A trivially checkable hash: the tests are about WHEN the model compares,
    // not about SHA-256 itself (covered against the server's own hash in the
    // component tests).
    const make = (save = vi.fn(async () => undefined)) => {
        let model!: MemoryDraftModel<string>;
        let dispose!: () => void;
        createRoot((d) => {
            dispose = d;
            model = new MemoryDraftModel<string>({
                hash: async (v) => `h(${v})`,
                equals: (a, b) => a === b,
                save,
            });
        });
        return { model, save, dispose };
    };

    test("a save carries the base's hash; a new entry has no base", async () => {
        const { model, save, dispose } = make();
        model.startEdit("base");
        model.setDraft("edited");
        expect(await model.save()).toBe(true);
        expect(save).toHaveBeenCalledWith("edited", "h(base)");

        model.startNew("");
        model.setDraft("brand new");
        await model.save();
        expect(save).toHaveBeenLastCalledWith("brand new", null);
        dispose();
    });

    test("typing during an in-flight save is kept, and the next save is based on what was saved", async () => {
        let finish!: () => void;
        const save = vi.fn(() => new Promise<undefined>((r) => (finish = () => r(undefined))));
        const { model, dispose } = make(save);
        model.startEdit("base");
        model.setDraft("first");
        const pending = model.save();
        await vi.waitFor(() => expect(save).toHaveBeenCalled()); // the save is in flight
        model.setDraft("first, then more"); // typed after pressing Save
        finish();
        expect(await pending).toBe(true);
        expect(model.editingAtom()).toBe(true);
        expect(model.draftAtom()).toBe("first, then more");
        expect(model.baseAtom()).toBe("first");

        save.mockImplementation(async () => undefined);
        await model.save();
        expect(save).toHaveBeenLastCalledWith("first, then more", "h(first)");
        expect(model.editingAtom()).toBe(false);
        dispose();
    });

    test("an external change raises the banner only for a dirty draft whose base hash moved", async () => {
        const { model, dispose } = make();
        model.startEdit("base");
        model.setDraft("edited");

        await model.observeExternal("base"); // same hash as the base
        expect(model.conflictAtom()).toBeNull();

        await model.observeExternal("moved");
        expect(model.conflictAtom()).toEqual({ reason: "changed", current: "moved" });
        expect(model.draftAtom()).toBe("edited");

        await model.observeExternal("base"); // moved back: the stale banner clears
        expect(model.conflictAtom()).toBeNull();
        dispose();
    });

    test("Keep editing keeps the ORIGINAL base, so the next save is still conditional on it", async () => {
        const { model, save, dispose } = make();
        model.startEdit("base");
        model.setDraft("edited");
        await model.observeExternal("moved");
        model.keepEditing();
        expect(model.conflictAtom()).toBeNull();
        await model.save();
        expect(save).toHaveBeenCalledWith("edited", "h(base)");
        dispose();
    });

    test("a refused save becomes a 'refused' conflict; Save anyway rebases onto the current content", async () => {
        const save = vi.fn().mockRejectedValueOnce(new Error("x: conflict: moved")).mockResolvedValue(undefined);
        const { model, dispose } = make(save);
        model.startEdit("base");
        model.setDraft("edited");
        expect(await model.save()).toBe(false);
        expect(model.conflictAtom()?.reason).toBe("refused");
        expect(model.draftAtom()).toBe("edited");

        await model.observeExternal("theirs"); // learns what's saved now; still "refused"
        expect(model.conflictAtom()).toEqual({ reason: "refused", current: "theirs" });

        expect(await model.overwrite()).toBe(true);
        expect(save).toHaveBeenLastCalledWith("edited", "h(theirs)");
        dispose();
    });

    test("with no local changes an external change is followed silently", async () => {
        const { model, dispose } = make();
        model.startEdit("base");
        await model.observeExternal("newer");
        expect(model.conflictAtom()).toBeNull();
        expect(model.draftAtom()).toBe("newer");
        expect(model.dirtyAtom()).toBe(false);
        dispose();
    });

    test("a non-conflict failure is an ordinary error and keeps the draft", async () => {
        const { model, dispose } = make(vi.fn().mockRejectedValue(new Error("disk full")));
        model.startEdit("base");
        model.setDraft("edited");
        expect(await model.save()).toBe(false);
        expect(model.conflictAtom()).toBeNull();
        expect(model.errorAtom()).toMatch(/disk full/);
        expect(model.editingAtom()).toBe(true);
        dispose();
    });
});
