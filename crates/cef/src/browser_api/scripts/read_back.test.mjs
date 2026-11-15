// @vitest-environment node
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// read_back.js, what act.rs reports as an element's state after a click, fill, select or check: run as the
// same text the host sends, on a plain object standing in for the element. `hasValue` mirrors the real DOM
// fact that `"value" in el` is true for an input, textarea, select or button, and false for a link or a div
// (a button's own value is read, then thrown away by the `tagName !== "BUTTON"` check in read_back.js itself).
function el(tag, opts = {}) {
    const attrs = opts.attrs || {};
    const out = {
        tagName: tag.toUpperCase(),
        isContentEditable: opts.contentEditable || false,
        textContent: opts.text || "",
        innerText: opts.text || "",
        validationMessage: opts.validationMessage || "",
        validity: opts.validity,
        selectedOptions: opts.selectedOptions || [],
        hasAttribute: (name) => name in attrs,
        getAttribute: (name) => (name in attrs ? attrs[name] : null),
    };
    if (opts.type !== undefined) out.type = opts.type;
    if (opts.href !== undefined) {
        out.href = opts.href;
        attrs.href = opts.href;
    }
    if (opts.hasValue) out.value = opts.value;
    if (opts.checked !== undefined) out.checked = opts.checked;
    return out;
}

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "read_back.js"), "utf8");
const run = new Function(`return (${src});`)();

describe("read_back: a link or a plain button says something a reader can use", () => {
    it("gives a link's destination and visible text, not a bare tag", () => {
        const a = el("a", { href: "https://example.test/invoice/42", text: "View invoice" });
        expect(run.call(a)).toEqual({ tag: "a", href: "https://example.test/invoice/42", text: "View invoice" });
    });

    it("skips href when the link has none (a JS-handled anchor with no real destination)", () => {
        const a = el("a", { text: "Load more" });
        expect(run.call(a)).toEqual({ tag: "a", text: "Load more" });
    });

    it("prefers the aria-label over the visible text", () => {
        const a = el("a", { href: "#", attrs: { "aria-label": "Close the dialog" }, text: "x" });
        expect(run.call(a)).toEqual({ tag: "a", href: "#", text: "Close the dialog" });
    });

    it("gives a plain element (a div acting as a button) its visible text too, squeezed to one line", () => {
        const div = el("div", { attrs: { role: "button" }, text: "  Archive  this\nthread  " });
        expect(run.call(div)).toEqual({ tag: "div", text: "Archive this thread" });
    });

    it("says nothing extra when there is nothing to say", () => {
        const a = el("a", {});
        expect(run.call(a)).toEqual({ tag: "a" });
    });

    it("gives a button its visible text too: its own type (\"submit\" by default) is never enough on its own", () => {
        const button = el("button", { type: "submit", text: "Create account" });
        expect(run.call(button)).toEqual({ tag: "button", type: "submit", text: "Create account" });
    });
});

describe("read_back: a form field already has something to say, so it keeps its own fields only", () => {
    it("does not add text to an input that already reports a value", () => {
        const input = el("input", { hasValue: true, value: "lark@example.test", text: "ignored" });
        expect(run.call(input)).toEqual({ tag: "input", value: "lark@example.test" });
    });

    it("does not add text to a checked checkbox", () => {
        const box = el("input", { type: "checkbox", hasValue: true, value: "on", checked: true, text: "ignored" });
        expect(run.call(box)).toEqual({ tag: "input", type: "checkbox", value: "on", checked: true });
    });
});
