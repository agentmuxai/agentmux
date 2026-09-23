// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// ReAgent P1 on PR #3519: editor-view.tsx's post-rebuild auto-focus claim
// fires on every CodeMirror rebuild, including the external-file-reload path
// (reactive on model.contentAtom()). Block-level focus alone can't tell "the
// user just selected this pane" from "the user is typing in this pane's file
// tree while a reload lands", so the claim is additionally gated on this
// guard. These cases pin the exact distinctions the guard has to make.

import { afterEach, describe, expect, it } from "vitest";
import { userIsTypingElsewhereIn } from "./editor-focus-guard";

function mount(html: string): HTMLElement {
    const root = document.createElement("div");
    root.className = "editor-view";
    root.innerHTML = html;
    document.body.appendChild(root);
    return root;
}

describe("userIsTypingElsewhereIn", () => {
    afterEach(() => {
        document.body.innerHTML = "";
    });

    it("is true when an <input> inside the editor root has focus (file-tree rename/filter)", () => {
        const root = mount('<div class="editor-tree-column"><input class="file-tree-filter" /></div>');
        const input = root.querySelector("input")!;
        input.focus();
        expect(document.activeElement).toBe(input);
        expect(userIsTypingElsewhereIn(root, document.activeElement)).toBe(true);
    });

    it("is true for a contenteditable inside the root that is NOT the (already destroyed) CodeMirror", () => {
        const root = mount('<div contenteditable="true" tabindex="0"></div>');
        const el = root.querySelector<HTMLElement>("[contenteditable]")!;
        el.focus();
        expect(userIsTypingElsewhereIn(root, document.activeElement)).toBe(true);
    });

    it("is false when focus has fallen back to <body> — the state right after CodeMirror is destroyed for a rebuild", () => {
        const root = mount("<div></div>");
        // Nothing focused inside root; jsdom reports body.
        expect(document.activeElement).toBe(document.body);
        expect(userIsTypingElsewhereIn(root, document.activeElement)).toBe(false);
    });

    it("is false when the focused editable control lives OUTSIDE this editor's root (another pane)", () => {
        const root = mount("<div></div>");
        const other = document.createElement("textarea");
        document.body.appendChild(other);
        other.focus();
        expect(document.activeElement).toBe(other);
        expect(userIsTypingElsewhereIn(root, document.activeElement)).toBe(false);
    });

    it("is false for a focused NON-editable element inside the root (e.g. an in-pane tab strip button)", () => {
        const root = mount('<button class="editor-tab">file.ts</button>');
        const btn = root.querySelector("button")!;
        btn.focus();
        expect(document.activeElement).toBe(btn);
        expect(userIsTypingElsewhereIn(root, document.activeElement)).toBe(false);
    });

    it("is false when the root itself is the active element", () => {
        const root = mount("");
        root.tabIndex = 0;
        root.focus();
        expect(userIsTypingElsewhereIn(root, document.activeElement)).toBe(false);
    });

    it("is false for a missing root or active element", () => {
        const root = mount("<input />");
        expect(userIsTypingElsewhereIn(undefined, document.activeElement)).toBe(false);
        expect(userIsTypingElsewhereIn(null, document.activeElement)).toBe(false);
        expect(userIsTypingElsewhereIn(root, null)).toBe(false);
        expect(userIsTypingElsewhereIn(root, undefined)).toBe(false);
    });
});
