// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Redaction (redact.mjs) and a shot's `verify` (verify.mjs). Redaction runs
// against jsdom's document; `verify` against a fake session, since jsdom does
// no layout and can't hit-test.

import { afterEach, describe, expect, it } from "vitest";
import { MIN_LENGTH, REPLACEMENTS, machinePairs, parseRedactArg, redactDocument, redactExpression } from "./redact.mjs";
import { checkVerify, onScreenExpression } from "./verify.mjs";

afterEach(() => {
    document.body.innerHTML = "";
});

describe("machinePairs", () => {
    it("replaces the user name and the hostname, extra pairs first", () => {
        expect(machinePairs({ user: "alice", host: "WORKSTATION-7", addresses: ["198.51.100.7"] }, [{ from: "acme-corp", to: "example" }])).toEqual([
            { from: "acme-corp", to: "example" },
            { from: "alice", to: REPLACEMENTS.user },
            { from: "WORKSTATION-7", to: REPLACEMENTS.host },
            { from: "198.51.100.7", to: REPLACEMENTS.address },
        ]);
    });

    it("skips names that are too short or already covered", () => {
        expect("ab".length).toBeLessThan(MIN_LENGTH);
        expect(machinePairs({ user: "ab", host: "box", addresses: [] })).toEqual([{ from: "box", to: REPLACEMENTS.host }]);
        // A machine named after its user: one pair, not two.
        expect(machinePairs({ user: "Alice", host: "alice", addresses: [] })).toEqual([{ from: "Alice", to: REPLACEMENTS.user }]);
        expect(machinePairs({ user: "", host: "", addresses: [] })).toEqual([]);
    });
});

describe("parseRedactArg", () => {
    it("splits at the first =", () => {
        expect(parseRedactArg("corp=example")).toEqual({ from: "corp", to: "example" });
        expect(parseRedactArg("a=b=c")).toEqual({ from: "a", to: "b=c" });
    });

    it("rejects a value without both sides", () => {
        for (const bad of ["corp", "=example", "corp="]) expect(() => parseRedactArg(bad)).toThrow(/expected from=to/);
    });
});

describe("redactDocument", () => {
    const pairs = [
        { from: "alice", to: "demo" },
        { from: "WORKSTATION-7", to: "demo-host" },
    ];

    it("replaces whole words in text, ignoring case, and counts them", () => {
        document.body.innerHTML = `
            <div id="status">workstation-7 · Alice@WORKSTATION-7</div>
            <div id="path">C:\\Users\\alice\\projects</div>
            <div id="words">malice aliced alice_x alice2</div>`;
        const count = redactDocument(pairs);
        expect(document.getElementById("status").textContent).toBe("demo-host · demo@demo-host");
        expect(document.getElementById("path").textContent).toBe("C:\\Users\\demo\\projects");
        // Not whole words: letters or digits on either side.
        expect(document.getElementById("words").textContent).toBe("malice aliced demo_x alice2");
        expect(count).toBe(5);
    });

    it("rewrites tooltips, placeholders, labels and field values", () => {
        document.body.innerHTML = `
            <button title="Connected to WORKSTATION-7" aria-label="alice's menu">x</button>
            <input placeholder="/home/alice" value="alice@WORKSTATION-7">`;
        expect(redactDocument(pairs)).toBe(5);
        const button = document.querySelector("button");
        expect(button.getAttribute("title")).toBe("Connected to demo-host");
        expect(button.getAttribute("aria-label")).toBe("demo's menu");
        const input = document.querySelector("input");
        expect(input.getAttribute("placeholder")).toBe("/home/demo");
        expect(input.value).toBe("demo@demo-host");
    });

    it("replaces an address only where it stands alone", () => {
        document.body.innerHTML = `<p>203.0.113.26 | 203.0.113.260 | 1.203.0.113.26</p>`;
        expect(redactDocument([{ from: "203.0.113.26", to: REPLACEMENTS.address }])).toBe(1);
        expect(document.querySelector("p").textContent).toBe("192.0.2.10 | 203.0.113.260 | 1.203.0.113.26");
    });

    it("treats a name as text, not a pattern", () => {
        document.body.innerHTML = `<p>a.b+c and axb+c</p>`;
        expect(redactDocument([{ from: "a.b+c", to: "x" }])).toBe(1);
        expect(document.querySelector("p").textContent).toBe("x and axb+c");
    });

    it("leaves a page without the names untouched", () => {
        document.body.innerHTML = `<p title="t">nothing here</p>`;
        expect(redactDocument(pairs)).toBe(0);
        expect(document.querySelector("p").textContent).toBe("nothing here");
    });
});

describe("redactExpression", () => {
    it("runs the same redaction when evaluated in the page", () => {
        document.body.innerHTML = `<p>alice on WORKSTATION-7</p>`;
        const expression = redactExpression([{ from: "alice", to: "demo" }]);
        // What Runtime.evaluate does with it: an expression, in page scope.
        expect((0, eval)(expression)).toBe(1);
        expect(document.querySelector("p").textContent).toBe("demo on WORKSTATION-7");
    });
});

describe("checkVerify", () => {
    // A session whose page has exactly the selectors in `visible` on screen.
    const fakeSession = (visible) => ({
        evaluated: [],
        async evaluate(expression) {
            this.evaluated.push(expression);
            return visible.some((sel) => expression === onScreenExpression(sel));
        },
    });

    it("passes a shot without a verify", async () => {
        await expect(checkVerify(fakeSession([]), { id: "x" })).resolves.toBeUndefined();
    });

    it("checks a selector, or every selector in an array", async () => {
        const session = fakeSession([".menu", ".modal"]);
        await expect(checkVerify(session, { verify: ".menu" })).resolves.toBeUndefined();
        await expect(checkVerify(session, { verify: [".menu", ".modal"] })).resolves.toBeUndefined();
        await expect(checkVerify(session, { verify: [".menu", ".popover"] })).rejects.toThrow("verify: .popover isn't on screen");
    });

    it("takes a function's true as a pass and its string as the reason", async () => {
        const session = fakeSession([]);
        await expect(checkVerify(session, { verify: async () => true })).resolves.toBeUndefined();
        await expect(checkVerify(session, { verify: () => "the Theme submenu didn't open" })).rejects.toThrow(
            "verify: the Theme submenu didn't open"
        );
        await expect(checkVerify(session, { verify: () => false })).rejects.toThrow("verify: failed");
    });
});
