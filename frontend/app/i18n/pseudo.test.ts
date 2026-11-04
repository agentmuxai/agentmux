import { describe, expect, it } from "vitest";
import { pseudoDict, pseudoLocalize } from "./pseudo";

describe("pseudoLocalize", () => {
    it("accents the letters, pads by about a third and brackets the text", () => {
        const out = pseudoLocalize("Open agent");
        expect(out.startsWith("[Óþéñ áĝéñţ")).toBe(true);
        expect(out.endsWith("]")).toBe(true);
        expect(out.length).toBeGreaterThan("Open agent".length * 1.3);
    });

    it("leaves placeholders untouched, so interpolation still works", () => {
        expect(pseudoLocalize("Hello, {{name}}")).toContain("{{name}}");
        expect(pseudoLocalize("{{count}} files")).toMatch(/^\[\{\{count\}\} /);
    });

    it("converts every entry of a dictionary and keeps its keys", () => {
        const out = pseudoDict({ a: "x", "b.one": "y {{n}}" });
        expect(Object.keys(out)).toEqual(["a", "b.one"]);
        expect(out.a.startsWith("[ẋ")).toBe(true);
        expect(out["b.one"]).toContain("{{n}}");
    });
});
