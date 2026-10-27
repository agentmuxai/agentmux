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
const page = ({ root, body, text = "Resource group" }) => ({
    documentElement: boxes(root),
    body: { ...boxes(body), innerText: text },
});

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
