// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { bodyFocusTarget } from "./editor-body-focus";

function root(html: string): HTMLElement {
    const el = document.createElement("div");
    el.innerHTML = html;
    document.body.replaceChildren(el);
    return el;
}

describe("bodyFocusTarget", () => {
    it("prefers a viewer's own focusable control", () => {
        const r = root('<div class="editor-body-wrap"><div class="editor-viewer"><div class="canvas" tabindex="0"></div></div></div>');
        expect(bodyFocusTarget(r)?.className).toBe("canvas");
    });

    it("focuses an info card or audio/video viewer itself, without adding a tab stop", () => {
        const r = root('<div class="editor-body-wrap"><div class="editor-viewer info-card-viewer"><button>Open</button></div></div>');
        const target = bodyFocusTarget(r)!;
        expect(target.classList.contains("info-card-viewer")).toBe(true);
        expect(target.tabIndex).toBe(-1);
        target.focus();
        expect(document.activeElement).toBe(target);
    });

    it("focuses a markdown preview", () => {
        const r = root('<div class="editor-preview-pane"><div class="editor-preview-content"><h1>Notes</h1></div></div>');
        expect(bodyFocusTarget(r)?.className).toBe("editor-preview-content");
    });

    it("finds nothing when there is no body", () => {
        expect(bodyFocusTarget(root("<div></div>"))).toBeNull();
    });
});
