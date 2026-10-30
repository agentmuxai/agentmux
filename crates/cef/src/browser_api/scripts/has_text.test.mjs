// @vitest-environment node
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// has_text.js, the check wait.rs runs in each frame, with a stand-in document:
// jsdom has no layout, so what matters here is the rule over the boxes.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "has_text.js"), "utf8");
const hasText = (document, text) => new Function("document", `return (${src});`)(document)(text);

const boxes = (n) => ({ getClientRects: () => new Array(n).fill({}) });
// `zones` are the elements `querySelectorAll` finds: { selector, text, copy?, vault? }.
const el = ({ text, copy = false, vault = false, parent = null }) => ({
    textContent: text,
    innerText: text,
    parentElement: parent,
    children: [{}, {}],
    hasAttribute: (a) => vault && a === "data-agentmux-vault",
    getAttribute: () => (copy ? "Copy" : null),
});
const page = ({ root, body, text = "Resource group", zones = [] }) => ({
    documentElement: boxes(root),
    body: { ...boxes(body), innerText: text },
    querySelectorAll: (sel) =>
        sel.startsWith("button")
            ? zones.filter((z) => z.copy).map((z) => ({ ...el({ text: "Copy" }), parentElement: el(z) }))
            : zones.filter((z) => !z.copy).map(el),
});
const SECRET = "Xy8Q~abcDEF1234ghijKLmnoPQRstu.vwxYZ0123";

describe("has_text: text counts only in a frame that is shown", () => {
    it("finds text on an ordinary page", () => {
        expect(hasText(page({ root: 1, body: 1 }), "Resource group")).toBe(true);
    });

    it("does not find text that is not there", () => {
        expect(hasText(page({ root: 1, body: 1 }), "Subscription")).toBe(false);
    });

    it("ignores text in a frame with no layout (its iframe is display: none)", () => {
        expect(hasText(page({ root: 0, body: 0 }), "Resource group")).toBe(false);
    });

    it("finds visible text under a display: contents body, which has no box of its own", () => {
        expect(hasText(page({ root: 1, body: 0 }), "Resource group")).toBe(true);
    });

    it("finds nothing on a page with no body yet", () => {
        expect(hasText({ documentElement: boxes(1), body: null }, "x")).toBe(false);
    });
});

describe("has_text: a secret the page shows once can't be waited for a piece at a time", () => {
    const shown = (zones) => page({ root: 1, body: 1, text: `New client secret Value ${SECRET} Copy`, zones });

    it("leaves a secret in code out, but not the text around it", () => {
        const p = shown([{ text: SECRET }]);
        expect(hasText(p, SECRET.slice(0, 8))).toBe(false);
        expect(hasText(p, "New client secret")).toBe(true);
    });

    it("leaves it out of a Copy button's container", () => {
        expect(hasText(shown([{ text: `Value ${SECRET}`, copy: true }]), SECRET.slice(4, 12))).toBe(false);
    });

    it("leaves an element holding a captured value out whole", () => {
        const p = page({ root: 1, body: 1, text: "Your code: 48213 done", zones: [{ text: "48213", vault: true }] });
        expect(hasText(p, "4821")).toBe(false);
        expect(hasText(p, "done")).toBe(true);
    });

    it("finds the same value outside where a page shows a secret", () => {
        expect(hasText(shown([]), SECRET.slice(0, 8))).toBe(true);
    });
});
