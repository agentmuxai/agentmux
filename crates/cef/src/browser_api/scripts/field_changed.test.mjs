// @vitest-environment node
//
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// field_changed.js, the rule that marks the fields of a form the agent filled apart from the ones the page
// already had, run as the same text the host splices in. The controls are plain objects with the properties
// the rule reads.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "field_changed.js"), "utf8");
const fieldChanged = new Function(`return (${src});`)();

const text = (value, defaultValue) => ({ tagName: "INPUT", value, defaultValue });

describe("fieldChanged", () => {
    it("one filled field is changed and the untouched site placeholders are not", () => {
        const form = [
            ["Business name", text("Acme Ltd", "")],
            ["Country", text("US", "US")],
            ["Language", text("en", "en")],
            ["Time zone", text("UTC", "UTC")],
        ];
        expect(form.map(([, el]) => fieldChanged(el, "text"))).toEqual([true, false, false, false]);
    });

    it("a checkbox or radio is changed when its checked state is not the page's", () => {
        expect(fieldChanged({ tagName: "INPUT", checked: true, defaultChecked: false }, "checkbox")).toBe(true);
        expect(fieldChanged({ tagName: "INPUT", checked: true, defaultChecked: true }, "radio")).toBe(false);
    });

    it("a select is changed when an option's selection is not the page's", () => {
        const opts = (a, b) => [
            { selected: a[0], defaultSelected: b[0] },
            { selected: a[1], defaultSelected: b[1] },
        ];
        expect(fieldChanged({ tagName: "SELECT", options: opts([false, true], [true, false]) }, "select-one")).toBe(true);
        expect(fieldChanged({ tagName: "SELECT", options: opts([true, false], [true, false]) }, "select-one")).toBe(false);
    });

    it("an untouched dropdown whose first option the page never marked is not changed", () => {
        const o = (selected, disabled = false) => ({ selected, defaultSelected: false, disabled });
        expect(fieldChanged({ tagName: "SELECT", options: [o(true), o(false)] }, "select-one")).toBe(false);
        expect(fieldChanged({ tagName: "SELECT", options: [o(false), o(true)] }, "select-one")).toBe(true);
        // A disabled first option is not what the browser shows.
        expect(fieldChanged({ tagName: "SELECT", options: [o(false, true), o(true)] }, "select-one")).toBe(false);
    });

    it("a field the agent wrote to is changed whatever its default says", () => {
        // A controlled input moves its default along with what was typed.
        const typed = { tagName: "INPUT", value: "Acme", defaultValue: "Acme", hasAttribute: (n) => n === "data-agentmux-filled" };
        expect(fieldChanged(typed, "text")).toBe(true);
        expect(fieldChanged({ ...typed, hasAttribute: () => false }, "text")).toBe(false);
    });

    it("a file input is changed when it holds a file", () => {
        expect(fieldChanged({ tagName: "INPUT", files: [{ name: "a.pdf" }] }, "file")).toBe(true);
        expect(fieldChanged({ tagName: "INPUT", files: [] }, "file")).toBe(false);
    });
});
