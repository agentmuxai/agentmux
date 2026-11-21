// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The conformance file both parsers run (R12), and a bounded fuzz run of this one. srv runs the same file in
// crates/common/src/deeplinks/conformance_tests.rs.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { encodePart, formatLink, parseLink, pathProblem } from "./parse";
import { DEEPLINKS, destinationLabel } from "./registry";

type Written = string | { prefix: string; repeat: string; count: number };
type Case = { link: Written; canonical?: Written; ok?: Record<string, unknown>; error?: string };

const here = dirname(fileURLToPath(import.meta.url));
const conformance = JSON.parse(readFileSync(join(here, "../../../crates/common/deeplinks/conformance.json"), "utf8")) as {
    valid: Case[];
    invalid: Case[];
};

const textOf = (w: Written): string => (typeof w === "string" ? w : w.prefix + w.repeat.repeat(w.count));
const expectedMap = (m: Record<string, Written | number>) =>
    Object.fromEntries(Object.entries(m).map(([k, v]) => [k, typeof v === "object" ? textOf(v) : v]));

describe("the conformance file", () => {
    it("has cases (it was read)", () => {
        expect(conformance.valid.length).toBeGreaterThan(10);
        expect(conformance.invalid.length).toBeGreaterThan(20);
    });

    for (const c of conformance.valid) {
        const link = textOf(c.link);
        it(`reads ${link.slice(0, 80)}`, () => {
            const got = parseLink(link);
            if (!got.ok) throw new Error(`${link}: refused with ${got.code}`);
            const ok = c.ok as { view: string; place: string | null; values: Record<string, Written>; params: Record<string, Written>; anchor: string | null };
            expect(got.link.view).toBe(ok.view);
            expect(got.link.place).toBe(ok.place);
            expect(got.link.values).toEqual(expectedMap(ok.values));
            expect(got.link.params).toEqual(expectedMap(ok.params));
            expect(got.link.anchor).toBe(ok.anchor);
            expect(got.link.canonical).toBe(c.canonical ? textOf(c.canonical) : link);
        });
    }

    for (const c of conformance.invalid) {
        const link = textOf(c.link);
        it(`refuses ${link.slice(0, 80)} as ${c.error}`, () => {
            const got = parseLink(link);
            expect(got.ok ? `parsed as ${got.link.canonical}` : got.code).toBe(c.error);
            if (!got.ok) expect(got.reason).toBe(DEEPLINKS.errors[c.error!]);
        });
    }
});

describe("the written form (R7)", () => {
    it("formats a destination the way the parser reads it back", () => {
        const link = formatLink({ view: "editor", place: "file", values: { path: "C:/work/a b.md" }, params: { line: 4 } });
        expect(link).toBe("agentmux://editor/C%3A%2Fwork%2Fa%20b.md?line=4");
        const back = parseLink(link);
        expect(back.ok && back.link.values.path).toBe("C:/work/a b.md");
    });

    it("writes parameters in the registry's order whatever order they are given in", () => {
        expect(formatLink({ view: "editor", place: "file", values: { path: "/a" }, params: { offset: 2, line: 1 } })).toBe(
            "agentmux://editor/%2Fa?line=1&offset=2"
        );
    });

    it("encodes every byte that is not unreserved, in upper case", () => {
        expect(encodePart("a/é~")).toBe("a%2F%C3%A9~");
    });
});

describe("paths (G7, the part both parsers check)", () => {
    it.each(["/", "/home/me/a.md", "C:/", "c:/work/notes.md"])("accepts %s", (p) => expect(pathProblem(p)).toBeNull());
    it.each(["a.md", "C:a", "//host/share", "\\\\host\\share", "/a/../b", "/a/./b", "/a//b", "C:/aux.txt", "C:/a:b", "/a."])("refuses %s", (p) =>
        expect(pathProblem(p)).toBe("bad_path")
    );
});

describe("the words for a place (R10, R17)", () => {
    const t = (k: string) => ({ "deeplinks.views.settings": "Settings", "settings.sections.recording": "Recording" })[k] ?? k;
    it("joins the view's and the place's labels with the one separator", () => {
        const got = parseLink("agentmux://settings/recording");
        expect(got.ok && destinationLabel(got.link, t)).toBe("Settings › Recording");
    });
});

/** A small seeded generator, so a failure names a link that fails again. */
function prng(seed: number) {
    let s = seed >>> 0;
    return () => {
        s = (s * 1664525 + 1013904223) >>> 0;
        return s / 2 ** 32;
    };
}

describe("a bounded fuzz run (R12)", () => {
    const heads = ["agentmux://settings/", "agentmux://kanban/", "agentmux://editor/", "agentmux://files/%2F", "agentmux://vault/", "agentmux://task/", "agentmux://", ""];
    const common = "aestk10-._~".split("");
    const structural = "/?#&=%".split("");
    const hex = "AF2EC3".split("");
    const exotic = ["é", "\u202e", "\u0000", " ", "\\", ":", "\u{1F600}", "Z"];

    it("never throws, and an accepted link's written form reads back as itself", () => {
        const rand = prng(20261121);
        const pick = <T>(xs: T[]) => xs[Math.floor(rand() * xs.length)];
        for (let i = 0; i < 4000; i++) {
            let link = pick(heads);
            const n = Math.floor(rand() * 40);
            for (let j = 0; j < n; j++) {
                const r = rand();
                link += r < 0.55 ? pick(common) : r < 0.8 ? pick(structural) : r < 0.93 ? pick(hex) : pick(exotic);
            }
            const got = parseLink(link);
            if (!got.ok) continue;
            const again = parseLink(got.link.canonical);
            expect(again.ok, link).toBe(true);
            if (again.ok) {
                expect(again.link.canonical).toBe(got.link.canonical);
                expect(again.link.values).toEqual(got.link.values);
            }
        }
    });
});
