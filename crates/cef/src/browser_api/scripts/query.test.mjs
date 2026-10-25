// @vitest-environment node
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// query.js's reach on the shared page: what a pane's agent may read or press. The page is a
// small stand-in tree (jsdom has no layout), enough for the rules: ownership by
// [data-blockid], a prompt for the person ([data-operator-only]) out of reach, and a click that
// would land on something else refused.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "query.js"), "utf8");

class El {
    constructor(name, attrs = {}, rect = { x: 0, y: 0, width: 10, height: 10 }) {
        this.name = name;
        this.attrs = attrs;
        this.rect = rect;
        this.children = [];
        this.parent = null;
        this.attributes = Object.entries(attrs).map(([name, value]) => ({ name, value }));
        this.textContent = name;
        this.tagName = "DIV";
    }
    add(child) {
        child.parent = this;
        this.children.push(child);
        return child;
    }
    getAttribute(n) {
        return this.attrs[n] ?? null;
    }
    *descendants() {
        for (const c of this.children) {
            yield c;
            yield* c.descendants();
        }
    }
    matches(sel) {
        const m = /^\[([\w-]+)(?:="([^"]*)")?\]$/.exec(sel);
        if (m) return m[1] in this.attrs && (m[2] === undefined || this.attrs[m[1]] === m[2]);
        return sel === this.name;
    }
    closest(sel) {
        for (let e = this; e; e = e.parent) if (e.matches(sel)) return e;
        return null;
    }
    querySelectorAll(sel) {
        return [...this.descendants()].filter((e) => e.matches(sel));
    }
    querySelector(sel) {
        return this.querySelectorAll(sel)[0] ?? null;
    }
    contains(other) {
        for (let e = other; e; e = e.parent) if (e === this) return true;
        return false;
    }
    getBoundingClientRect() {
        return this.rect;
    }
    scrollIntoView() {}
}

/** A page: chrome with a button, two panes, and (optionally) a prompt over everything. */
function page({ prompt } = {}) {
    const body = new El("body");
    const chrome = body.add(new El("chrome"));
    const menu = chrome.add(new El("menu", {}, { x: 0, y: 0, width: 20, height: 20 }));
    const mine = body.add(new El("mine", { "data-blockid": "me" }));
    const myButton = mine.add(new El("my-button", {}, { x: 100, y: 100, width: 20, height: 20 }));
    const other = body.add(new El("other", { "data-blockid": "them" }));
    other.add(new El("their-button"));
    let promptRoot = null;
    let install = null;
    if (prompt) {
        promptRoot = body.add(new El("prompt", prompt === "marked" ? { "data-operator-only": "" } : {}));
        install = promptRoot.add(new El("install", {}, { x: 300, y: 300, width: 40, height: 20 }));
    }
    // What is on top at a point: the prompt covers everything when it is open.
    const win = {};
    const doc = {
        body,
        activeElement: body,
        querySelectorAll: (s) => body.querySelectorAll(s),
        elementFromPoint: (x, y) => {
            if (promptRoot) return x === 310 && y === 310 ? install : promptRoot;
            if (x === 110 && y === 110) return myButton;
            if (x === 10 && y === 10) return menu;
            return body;
        },
    };
    new Function("window", "document", "CSS", src)(win, doc, { escape: (s) => s });
    return { win, body, chrome, menu, mine, myButton, install, promptRoot };
}

describe("what an agent's pane may reach on the shared page", () => {
    it("reaches its own pane and the shared chrome, not another pane or a container of one", () => {
        const p = page();
        expect(p.win.__amq_allowed_for(p.myButton, "me")).toBe(true);
        expect(p.win.__amq_allowed_for(p.menu, "me")).toBe(true);
        expect(p.win.__amq_allowed_for(p.body.querySelector("their-button"), "me")).toBe(false);
        expect(p.win.__amq_allowed_for(p.body, "me")).toBe(false);
    });

    it("leaves out a prompt for the person, what is inside it, and what contains it", () => {
        const p = page({ prompt: "marked" });
        expect(p.win.__amq_allowed_for(p.install, "me")).toBe(false);
        expect(p.win.__amq_allowed_for(p.promptRoot, "me")).toBe(false);
        expect(p.win.__amq_allowed_for(p.body, "me")).toBe(false);
        expect(p.win.__amq_query("install", 0, "me")).toEqual({ matches: [] });
        expect(p.win.__amq_centroid_of("install", "me")).toBeNull();
    });

    it("reaches an unmarked dialog as before", () => {
        const p = page({ prompt: "plain" });
        expect(p.win.__amq_allowed_for(p.install, "me")).toBe(true);
    });
});

describe("a click lands on what was asked for", () => {
    it("gives the centre of an element that is on top", () => {
        const p = page();
        expect(p.win.__amq_centroid_of("my-button", "me")).toEqual({ x: 110, y: 110 });
    });

    it("refuses an element something else is drawn over", () => {
        const p = page({ prompt: "marked" });
        // The pane's button is allowed, but the prompt and its backdrop are on top of it.
        expect(p.win.__amq_centroid_of("my-button", "me")).toEqual({ covered: true });
    });

    it("does not check a browser pane's own page (no block to scope to)", () => {
        const p = page();
        expect(p.win.__amq_centroid_of("my-button", null)).toEqual({ x: 110, y: 110 });
        const covered = page({ prompt: "marked" });
        expect(covered.win.__amq_centroid_of("my-button", null)).toEqual({ x: 110, y: 110 });
    });
});
