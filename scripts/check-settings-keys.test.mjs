// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-settings-keys.mjs on small made-up schemas and sources.

import { describe, expect, it } from "vitest";
import { check, compare, parseBaseline, readKeys, withoutTestModules } from "./check-settings-keys.mjs";

const entry = (extra = {}) => ({ type: "boolean", default: true, description: "d", "x-section": "App", ...extra });
const schema = (props) => ({ $defs: { SettingsType: { properties: props } } });
const STRUCT = (...keys) => `pub struct SettingsType {\n${keys.map((k, i) => `    #[serde(rename = "${k}")]\n    pub f${i}: bool,\n`).join("")}\n}\n`;
const USES = (n) => Array.from({ length: n }, (_, i) => `s.f${i}`).join(" ");
const ts = (path, text) => ({ path, text });
// The two floors (80 frontend, 100 srv) are met by padding, so a test is about one finding.
const pad = (n, prefix) => Array.from({ length: n }, (_, i) => `${prefix}:k${i}`);
const padded = (extraSources = [], extraProps = {}) => {
    const fe = pad(80, "fe");
    const rs = pad(100, "rs");
    const props = Object.fromEntries([...fe, ...rs].map((k) => [k, entry()]));
    return {
        schema: schema({ ...props, ...extraProps }),
        sources: [
            ts("frontend/a.ts", fe.map((k) => `getSettingsKeyAtom("${k}")`).join("\n")),
            ts("crates/srv-core/src/backend/wconfig/types.rs", STRUCT(...rs)),
            ts("crates/srv-core/src/backend/uses.rs", `fn f(s: &S) { ${USES(rs.length)} }`),
            ...extraSources,
        ],
    };
};
const kinds = (findings) => findings.map((f) => `${f.kind} ${f.key}`);

describe("check-settings-keys", () => {
    it("finds nothing when the schema and the reads agree", () => {
        const { schema: s, sources } = padded();
        expect(check(s, sources)).toEqual([]);
    });

    it("fails a key the frontend reads that the schema lacks", () => {
        const { schema: s, sources } = padded([ts("frontend/b.tsx", 'settingsAtom()?.["x:new"]')]);
        expect(kinds(check(s, sources))).toEqual(["read-not-in-schema x:new"]);
    });

    it("fails a key srv reads through an extra read that the schema lacks", () => {
        const { schema: s, sources } = padded([ts("crates/srv/src/a.rs", 'fn f(c: &C) { c.extra_bool("x:srvnew"); }')]);
        expect(kinds(check(s, sources))).toEqual(["read-not-in-schema x:srvnew"]);
    });

    it("does not count a connection's own settings as a settings read", () => {
        const { schema: s, sources } = padded([ts("frontend/c.tsx", 'const n = r().settings?.["display:name"];')]);
        expect(check(s, sources)).toEqual([]);
    });

    it("counts a typed key as read when its field is used outside the struct, and not when it is only declared", () => {
        const { schema: s, sources } = padded([ts("crates/srv/src/uses2.rs", "fn g(s: &S) { s.t_used }")], { "x:used": entry(), "x:declared": entry() });
        const more = '    #[serde(rename = "x:used")]\n    pub t_used: bool,\n    #[serde(rename = "x:declared")]\n    pub t_declared: bool,\n}\n';
        const swapped = sources.map((src) => (src.path.endsWith("types.rs") ? ts(src.path, src.text.replace(/\n}\n$/, `\n${more}`)) : src));
        expect(kinds(check(s, swapped))).toEqual(["schema-not-read x:declared"]);
    });

    it("does not count a field of another struct, or a key only a test module reads", () => {
        const rs = `${STRUCT()}\npub struct Other {\n    #[serde(rename = "ssh:user")]\n    pub u: String,\n}\n#[cfg(test)]\nmod tests {\n    fn t(c: &C) { c.extra_bool("x:fixture"); }\n}\n`;
        const { schema: s, sources } = padded([ts("crates/srv/src/o.rs", rs)]);
        expect(check(s, sources)).toEqual([]);
    });

    it("does not let a schema family stand in for a key the code reads", () => {
        const { schema: s, sources } = padded([ts("frontend/d.ts", 'getSettingsKeyAtom("fam:thing")')], { "fam:*": { type: "object" } });
        expect(kinds(check(s, sources))).toEqual(["read-not-in-schema fam:thing"]);
        const listed = padded([ts("frontend/d.ts", 'getSettingsKeyAtom("fam:thing")')], { "fam:*": { type: "object" }, "fam:thing": entry() });
        expect(check(listed.schema, listed.sources)).toEqual([]);
    });

    it("reads a key a settings module names as a constant, and a prefix constant's family", () => {
        const consts = 'pub const KEY_A: &str = "buildcache:a";\npub const SETTING: &str = "term:b";\npub const ROUTE_CAP_SETTING: &str = "model:cap:";\npub const META_KEY_X: &str = "pane:meta";\n';
        const { schema: s, sources } = padded([ts("crates/srv-core/src/backend/consts.rs", consts)]);
        expect(kinds(check(s, sources))).toEqual(["prefix-not-in-schema model:cap:", "read-not-in-schema buildcache:a", "read-not-in-schema term:b"]);
        const ok = padded([ts("crates/srv-core/src/backend/consts.rs", consts)], { "buildcache:a": entry(), "term:b": entry(), "model:cap:one": entry() });
        expect(check(ok.schema, ok.sources).filter((f) => f.kind !== "schema-not-read")).toEqual([]);
    });

    it("fails a schema key nothing reads, unless it says x-unread", () => {
        const { schema: s, sources } = padded([], { "x:dead": entry(), "x:other": entry({ "x-unread": "read by a script" }) });
        expect(kinds(check(s, sources))).toEqual(["schema-not-read x:dead"]);
    });

    it("fails an entry without a description, a section or a default, and a secret with a default", () => {
        const dead = { "x:a": entry({ description: "" }), "x:b": entry({ "x-section": "" }), "x:c": { type: "string", description: "d", "x-section": "App" } };
        const secret = { "x:d": entry({ "x-secret": true, default: "abc" }), "x:e": entry({ "x-platform": ["beos"] }) };
        const all = Object.keys({ ...dead, ...secret });
        const { schema: s, sources } = padded([ts("frontend/e.ts", all.map((k) => `getSettingsKeyAtom("${k}")`).join("\n"))], { ...dead, ...secret });
        expect(kinds(check(s, sources))).toEqual([
            "bad-platform x:e", "no-default x:c", "no-description x:a", "no-section x:b", "secret-default x:d",
        ]);
    });

    it("accepts x-nodefault in place of a default", () => {
        const { schema: s, sources } = padded([ts("frontend/f.ts", 'getSettingsKeyAtom("x:nd")')], { "x:nd": { type: "string", description: "d", "x-section": "App", "x-nodefault": "unset uses your shell" } });
        expect(check(s, sources)).toEqual([]);
    });

    it("fails when an extractor finds too few keys, so a pattern that stops matching is noticed", () => {
        const found = check(schema({ "a:b": entry() }), [ts("frontend/a.ts", 'getSettingsKeyAtom("a:b")')]);
        expect(kinds(found)).toEqual(["floor frontend", "floor srv"]);
    });
});

describe("withoutTestModules and readKeys", () => {
    it("removes a nested test module whole", () => {
        const text = 'fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() { if x { y } }\n}\nfn b() {}\n';
        expect(withoutTestModules(text)).toBe("fn a() {}\n\nfn b() {}\n");
    });

    it("is not thrown by a brace inside a string, a raw string, a char or a comment", () => {
        const text = 'fn a(){}\n#[cfg(test)]\nmod tests {\n  const A: &str = "{ open";\n  const B: &str = r#"}} "q" {"#;\n  fn c(){ let x = \'{\'; } // }\n}\nfn b(){}\n';
        expect(withoutTestModules(text)).toBe("fn a(){}\n\nfn b(){}\n");
    });

    it("reads each kind of key from its own source", () => {
        const r = readKeys([ts("frontend/a.ts", 'getSettingsKeyAtom("a:fe")'), ts("crates/a.rs", 'x.extra_u64("a:rs");')]);
        expect([...r.frontend, ...r.srv]).toEqual(["a:fe", "a:rs"]);
    });
});

describe("the baseline", () => {
    const f = (kind, key) => ({ kind, key, detail: "" });

    it("passes a listed gap and fails a new one", () => {
        const base = parseBaseline("# note\nno-description\tx:a\treason\n");
        const r = compare([f("no-description", "x:a"), f("no-description", "x:b")], base);
        expect(r.added.map((x) => x.key)).toEqual(["x:b"]);
        expect(r.stale).toEqual([]);
    });

    it("fails a listed entry that no longer applies, so the list only shrinks", () => {
        const r = compare([], parseBaseline("no-description\tx:a\treason\n"));
        expect(r.stale).toEqual(["no-description x:a"]);
    });

    it("fails an entry with no reason, and never lets a floor be listed", () => {
        expect(compare([f("no-description", "x:a")], parseBaseline("no-description\tx:a\t\n")).noReason).toEqual(["no-description x:a"]);
        expect(compare([f("floor", "srv")], parseBaseline("floor\tsrv\treason\n")).added).toHaveLength(1);
    });
});
