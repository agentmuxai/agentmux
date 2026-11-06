// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-window-modals.mjs.
// Spec: docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md §5.

import { describe, expect, it } from "vitest";
import { KEEP, checkFile, collect, scan } from "./check-window-modals.mjs";

const rules = (path, text) => checkFile(path, text).map((v) => `${v.rule}:${v.line}`);

describe("a window modal from a file that is not in KEEP", () => {
    it("flags openModal, as a bare call and through modalsModel, with its line", () => {
        const src = 'import { openModal } from "x";\n\nopenModal(Dialog);\nmodalsModel.openModal(Other, {});\n';
        expect(rules("frontend/app/view/new-thing.ts", src)).toEqual(["openModal:3", "openModal:4"]);
    });

    it("does not flag openPaneModal, or a method that only ends in openModal", () => {
        expect(rules("frontend/app/view/a.ts", "openPaneModal(blockId, Dialog);\nthis.reopenModal();\nx.openModal(1);")).toEqual([]);
    });

    it("flags a ConfirmModal with no scope, on one line or several, but not one that names its scope", () => {
        const src = [
            '<ConfirmModal open title="x" />',
            '<ConfirmModal scope="pane" open title="y" />',
            "<ConfirmModal",
            "  open",
            "/>",
            "<ConfirmModal",
            "  open",
            '  scope="tab"',
            "/>",
            "",
        ].join("\n");
        expect(rules("frontend/app/view/a.tsx", src)).toEqual(["confirmModal:1", "confirmModal:3"]);
    });

    it("reads a scope after an arrow function in an earlier prop (a correctly scoped modal is not flagged)", () => {
        expect(rules("frontend/app/view/a.tsx", '<ConfirmModal onConfirm={() => go()} scope="pane" />')).toEqual([]);
        expect(rules("frontend/app/view/a.tsx", '<ConfirmModal onConfirm={() => go()} title="x" />')).toEqual(["confirmModal:1"]);
        expect(rules("frontend/app/view/a.tsx", '<ConfirmModal onConfirm={() => setScope("pane")} />')).toEqual(["confirmModal:1"]);
    });

    it("does not mistake a // or /* inside a string for a comment, so a call after it is still seen", () => {
        expect(rules("frontend/app/view/a.ts", 'const u = "a//b"; openModal(X);')).toEqual(["openModal:1"]);
        expect(rules("frontend/app/view/a.ts", 'const g = "**/*.ts"; openModal(X);')).toEqual(["openModal:1"]);
        expect(rules("frontend/app/view/a.ts", "const t = `//${x}`; openModal(X);")).toEqual(["openModal:1"]);
    });

    it("an apostrophe in JSX text does not swallow the code after it", () => {
        const src = ["<p>Don't allow</p>", "openModal(X);", "// it's fine", ""].join("\n");
        expect(rules("frontend/app/view/a.tsx", src)).toEqual(["openModal:2"]);
    });

    it("flags an explicit window scope, as a prop and as an option", () => {
        expect(rules("frontend/app/view/a.tsx", '<Modal scope="window" />\nopen({ scope: "window" });')).toEqual([
            "windowScope:1",
            "windowScope:2",
        ]);
    });

    it("flags window.confirm, window.alert and window.prompt wherever they are", () => {
        expect(rules("frontend/app/view/a.ts", 'if (window.confirm("sure?")) {}\nwindow.alert("x");\nwindow.prompt("y");')).toEqual([
            "freezes-renderer:1",
            "freezes-renderer:2",
            "freezes-renderer:3",
        ]);
    });

    it("ignores comments, tests and the modal primitives", () => {
        expect(rules("frontend/app/view/a.ts", "// openModal(Dialog) was here\n/* window.confirm() */\n")).toEqual([]);
        expect(rules("frontend/app/view/a.test.ts", "openModal(Dialog);")).toEqual([]);
        expect(rules("frontend/app/element/modal.tsx", 'scope="window"')).toEqual([]);
    });
});

describe("a file in KEEP", () => {
    const file = "frontend/app/window/window-close.ts";

    it("may open as many window modals as its entry says, and no more", () => {
        expect(KEEP.get(file)?.openModal).toBe(2);
        expect(rules(file, "openModal(A);\nopenModal(B);\n")).toEqual([]);
        expect(rules(file, "openModal(A);\nopenModal(B);\nopenModal(C);\n")).toEqual(["openModal:3"]);
    });

    it("still may not use a kind its entry does not list", () => {
        expect(rules(file, "openModal(A);\n<ConfirmModal open />\n")).toEqual(["confirmModal:2"]);
    });

    it("every entry says why", () => {
        for (const [path, entry] of KEEP) expect(entry.why, path).toMatch(/\S/);
    });
});

describe("scan", () => {
    it("counts each kind", () => {
        expect(scan('openModal(A);\n<ConfirmModal open />\n<Modal scope="window" />').counts).toEqual({
            openModal: 1,
            confirmModal: 1,
            windowScope: 1,
        });
    });
});

describe("the tree", () => {
    it("has no window modal outside KEEP, and KEEP lists nothing that is gone", () => {
        const { violations, stale } = collect();
        expect(violations.map((v) => `${v.path}:${v.line} ${v.rule}`)).toEqual([]);
        expect(stale).toEqual([]);
    });
});
