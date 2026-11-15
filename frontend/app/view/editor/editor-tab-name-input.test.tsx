// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The name box of a document tab and the line under it
 * (editor-tab-name-input.tsx): what is selected, which keys do what, what
 * leaving the box does, and what a screen reader is told.
 * docs/specs/SPEC_EDITOR_TAB_RENAME_2026_11_14.md §3, §11.9.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PaneTabRenameInput } from "@/app/element/PaneTabRenameInput";
import { SaveAsInput } from "./editor-save-as-input";
import { TabNameInput, TabRenameLine } from "./editor-tab-name-input";
import { InlineInput } from "./file-tree-inline-input";
import type { RenameLine, TabRename } from "./editor-tab-rename";

/** A name box's state, recording what the box asks of it. */
function fakeRename(name: string) {
    const [busy, setBusy] = createSignal(false);
    const [line, setLine] = createSignal<RenameLine | null>(null);
    const calls = { commit: [] as string[], cancel: [] as boolean[], typed: [] as string[] };
    const rename: TabRename = {
        tabId: () => "t1",
        busy,
        line,
        refusal: () => null,
        start: () => true,
        startFromDoubleClick: () => true,
        initialName: () => name,
        setTyped: (text) => void calls.typed.push(text),
        cancel: (returnFocus = false) => void calls.cancel.push(returnFocus),
        commit: async (typed) => void calls.commit.push(typed),
    };
    return { rename, calls, setBusy, setLine };
}

function mountBox(name: string) {
    const fake = fakeRename(name);
    const view = render(() => (
        <div class="editor-main-column">
            <TabNameInput rename={fake.rename} lineId="line-1" />
            <TabRenameLine rename={fake.rename} id="line-1" left={() => 48} />
        </div>
    ));
    const input = view.container.querySelector("input") as HTMLInputElement;
    return { ...fake, view, input };
}

afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

describe("the name box", () => {
    it("starts focused, with the name selected up to its extension", () => {
        const { input } = mountBox("notes.test.ts");
        expect(document.activeElement).toBe(input);
        expect(input.value).toBe("notes.test.ts");
        expect([input.selectionStart, input.selectionEnd]).toEqual([0, "notes.test".length]);
    });

    it("selects a name with no extension whole", () => {
        const { input } = mountBox(".env");
        expect([input.selectionStart, input.selectionEnd]).toEqual([0, 4]);
    });

    it("Enter commits what is typed, Escape cancels and asks for the keyboard to go back", () => {
        const { input, calls } = mountBox("a.ts");
        fireEvent.input(input, { target: { value: "b.ts" } });
        fireEvent.keyDown(input, { key: "Enter" });
        expect(calls.typed).toEqual(["b.ts"]);
        expect(calls.commit).toEqual(["b.ts"]);
        fireEvent.keyDown(input, { key: "Escape" });
        expect(calls.cancel).toEqual([true]);
    });

    it("Enter that confirms an input-method composition is not a commit", () => {
        const { input, calls } = mountBox("a.ts");
        fireEvent.keyDown(input, { key: "Enter", isComposing: true });
        // Safari: the composition has ended, and the key that ended it says 229.
        fireEvent.keyDown(input, { key: "Enter", keyCode: 229 });
        expect(calls.commit).toEqual([]);
        fireEvent.keyDown(input, { key: "Enter", keyCode: 13 });
        expect(calls.commit).toEqual(["a.ts"]);
    });

    it("F2 moves the selection on: the name, the whole name, the extension", () => {
        const { input } = mountBox("notes.md");
        const selection = () => [input.selectionStart, input.selectionEnd];
        fireEvent.keyDown(input, { key: "F2" });
        expect(selection()).toEqual([0, 8]);
        fireEvent.keyDown(input, { key: "F2" });
        expect(selection()).toEqual([6, 8]);
        fireEvent.keyDown(input, { key: "F2" });
        expect(selection()).toEqual([0, 5]);
    });

    it("keeps its keys and clicks to itself: the tab around it hears none of them", () => {
        const fake = fakeRename("a.ts");
        const heard: string[] = [];
        const view = render(() => (
            <div role="tab" tabindex={-1} onKeyDown={() => heard.push("keydown")} onClick={() => heard.push("click")} onDblClick={() => heard.push("dblclick")}>
                <TabNameInput rename={fake.rename} lineId="line-1" />
            </div>
        ));
        const input = view.container.querySelector("input") as HTMLInputElement;
        fireEvent.keyDown(input, { key: "z", ctrlKey: true });
        fireEvent.keyDown(input, { key: "a", ctrlKey: true });
        fireEvent.keyDown(input, { key: "F2" });
        fireEvent.click(input);
        fireEvent.dblClick(input);
        expect(heard).toEqual([]);
    });

    it("leaving the box cancels, without taking the keyboard back", () => {
        const { input, calls } = mountBox("a.ts");
        vi.spyOn(document, "hasFocus").mockReturnValue(true);
        fireEvent.blur(input);
        expect(calls.cancel).toEqual([false]);
        expect(calls.commit).toEqual([]);
    });

    it("the whole window losing focus is not leaving: the box stays", () => {
        const { input, calls } = mountBox("a.ts");
        vi.spyOn(document, "hasFocus").mockReturnValue(false);
        fireEvent.blur(input);
        expect(calls.cancel).toEqual([]);
    });

    it("can't be edited while the answer is on its way", () => {
        const { input, setBusy } = mountBox("a.ts");
        expect(input.readOnly).toBe(false);
        setBusy(true);
        expect(input.readOnly).toBe(true);
    });
});

describe("the older name boxes", () => {
    it("don't take the Enter that confirms an input-method composition either", () => {
        const confirmed: string[] = [];
        // One at a time: each of these ends when another box takes the focus.
        const boxes = [
            () => <SaveAsInput onConfirm={(v) => confirmed.push(`save as ${v}`)} onCancel={() => {}} />,
            () => <PaneTabRenameInput initialValue="old" onConfirm={(v) => confirmed.push(`tab ${v}`)} onCancel={() => {}} />,
            () => <InlineInput initialValue="a.ts" onConfirm={(v) => confirmed.push(`tree ${v}`)} onCancel={() => {}} />,
        ];
        for (const box of boxes) {
            const view = render(box);
            const input = view.container.querySelector("input") as HTMLInputElement;
            fireEvent.input(input, { target: { value: "typed" } });
            const before = confirmed.length;
            fireEvent.keyDown(input, { key: "Enter", isComposing: true });
            fireEvent.keyDown(input, { key: "Enter", keyCode: 229 });
            expect(confirmed.length).toBe(before);
            fireEvent.keyDown(input, { key: "Enter", keyCode: 13 });
            view.unmount();
        }
        expect(confirmed).toEqual(["save as typed", "tab typed", "tree typed"]);
    });
});

describe("what a screen reader is told", () => {
    it("the box has a name and is described by the line under it", () => {
        const { input, setLine, view } = mountBox("Untitled-1");
        expect(input.getAttribute("aria-label")).toBe("File name");
        expect(input.getAttribute("aria-describedby")).toBe("line-1");
        expect(view.container.querySelector("#line-1")).toBeNull();

        setLine({ kind: "hint", text: "Saves to c:/repo/notes.md" });
        const line = view.container.querySelector("#line-1")!;
        expect(line.textContent).toBe("Saves to c:/repo/notes.md");
        // A hint that changes with every key is read with the box, not announced each time.
        expect(line.getAttribute("role")).toBeNull();
        expect(input.getAttribute("aria-invalid")).toBe("false");
    });

    it("an error is announced, marks the box invalid, and has an icon as well as a colour", () => {
        const { input, setLine, view } = mountBox("a.ts");
        setLine({ kind: "error", text: "Something named “b.ts” already exists here." });
        const line = view.container.querySelector("#line-1")!;
        expect(line.getAttribute("role")).toBe("alert");
        expect(line.classList.contains("editor-tab-rename-line--error")).toBe(true);
        expect(line.querySelector("i[aria-hidden='true']")).not.toBeNull();
        expect(line.textContent).toBe("Something named “b.ts” already exists here.");
        expect(input.getAttribute("aria-invalid")).toBe("true");
    });

    it("the line starts under the box", () => {
        const { setLine, view } = mountBox("a.ts");
        setLine({ kind: "busy", text: "Renaming…" });
        expect((view.container.querySelector("#line-1") as HTMLElement).style.getPropertyValue("--rename-line-left")).toBe("48px");
    });
});
