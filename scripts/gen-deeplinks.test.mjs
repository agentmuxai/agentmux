// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// gen-deeplinks.mjs: the real registry is sound and its generated files are current; a registry that breaks a rule
// the spec sets (a worker an agent may start, a link that names a target or a connection, a label not in the
// dictionary) is refused.

import { describe, expect, it } from "vitest";
import { load, outputs, renderTs, validate } from "./gen-deeplinks.mjs";
import { existsSync, readFileSync } from "node:fs";

const { reg, en } = load();
const labels = new Set(Object.keys(en));
const clone = () => JSON.parse(JSON.stringify(reg));

describe("gen-deeplinks", () => {
    it("finds nothing wrong with the registry", () => {
        expect(validate(reg, labels)).toEqual([]);
    });

    it("has written what it would write (run node scripts/gen-deeplinks.mjs)", () => {
        for (const [path, text] of outputs(reg, en)) {
            expect(existsSync(path), path).toBe(true);
            expect(readFileSync(path, "utf8").replace(/\r\n/g, "\n"), path).toBe(text);
        }
    });

    it("refuses a worker that an agent may open (G1)", () => {
        const r = clone();
        r.views.find((v) => v.id === "term").agent = "yes";
        expect(validate(r, labels).join("\n")).toContain("reveal-only");
    });

    it.each(["pane", "tab", "window", "target", "connection"])("refuses a parameter named %s (§3.2, G7)", (name) => {
        const r = clone();
        r.views.find((v) => v.id === "editor").places[0].params.push({ name, kind: "id", label: "deeplinks.params.line" });
        expect(validate(r, labels).join("\n")).toContain("never names a target");
    });

    it("refuses a label the dictionary does not have", () => {
        const r = clone();
        r.views[0].label = "deeplinks.views.nothing";
        expect(validate(r, labels).join("\n")).toContain("is not in en.json");
    });

    it("refuses a duplicate view and an alias that shadows a view", () => {
        const r = clone();
        r.views.push({ ...r.views[0] });
        r.aliases.push({ from: ["settings"], to: ["help"] });
        const problems = validate(r, labels).join("\n");
        expect(problems).toContain("duplicate");
        expect(problems).toContain("shadows");
    });

    it("names every label labelKey in the app's table, so the dictionary's key check reads it", () => {
        const ts = renderTs(reg);
        expect(ts).toContain('labelKey: "settings.sections.recording"');
        expect(ts).not.toMatch(/\blabel: "/);
    });
});
