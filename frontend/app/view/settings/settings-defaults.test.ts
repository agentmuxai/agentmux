// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One setting's default is written in up to five places: `schema/settings.json`
 * (the source of truth), `settings-template.jsonc`, the Settings pane control
 * (`sections/*.tsx`, as the fallback for an unset key), the frontend code that
 * uses the setting, and the backend's read (srv, or the launcher for the few
 * settings it owns). This test reads them all and fails when they disagree, so
 * a default changed in one place can't leave the others showing or doing
 * something else.
 *
 * Extraction is by pattern, not by running the code. Each source has a floor on
 * how many defaults it must yield, so a pattern that stops matching fails here
 * instead of silently checking nothing.
 */

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "../../../..");
const read = (rel: string) => readFileSync(join(ROOT, rel), "utf8");
const lineOf = (text: string, index: number) => text.slice(0, index).split("\n").length;

type Found = { value: unknown; at: string };
type Source = "template" | "ui" | "app" | "srv";

// ── Known disagreements ─────────────────────────────────────────────────────
// Each needs a decision about which value is right; until then it is listed
// here. The test also fails when a listed key agrees again, so this only shrinks.
const KNOWN_MISMATCHES: Record<string, string> = {
    // TODO: pick one. The template says 0.9 and the pane 1.5, and the layout
    // itself disagrees: 0.8 in layoutGeometry.ts, 1.0 in tilelayout-shared.tsx.
    "window:magnifiedblocksize": "template 0.9, pane 1.5, layout 0.8 or 1.0",
};

// ── The schema ──────────────────────────────────────────────────────────────

const schemaProps: Record<string, { default?: unknown }> = JSON.parse(read("schema/settings.json")).$defs.SettingsType
    .properties;
const SCHEMA_KEYS = new Set(Object.keys(schemaProps));
const schemaDefault = (key: string): Found | undefined =>
    schemaProps[key] && "default" in schemaProps[key]
        ? { value: schemaProps[key].default, at: "schema/settings.json" }
        : undefined;

// ── settings-template.jsonc ─────────────────────────────────────────────────
// Every entry is a commented-out line: `    // "key":   value,  // note`.
// `[ \t]`, not `\s`: with the `m` flag `\s` would run on into the next line.

function templateValues(): Map<string, Found[]> {
    const text = read("settings-template.jsonc");
    const out = new Map<string, Found[]>();
    for (const m of text.matchAll(/^[ \t]*\/\/[ \t]*"([^"]+)":[ \t]*(.*?)[ \t]*,?[ \t]*(?:\/\/.*)?$/gm)) {
        const at = `settings-template.jsonc:${lineOf(text, m.index!)}`;
        push(out, m[1], { value: JSON.parse(m[2]), at });
    }
    return out;
}

// ── The Settings pane ───────────────────────────────────────────────────────
// How a control reads its key, and the default that implies:
//   `(s()["k"] as T) ?? LIT` / `|| LIT`   → LIT (or an imported constant)
//   `!!(s()["k"] …)` / `!(s()["k"] …)`    → false
//   `s()["k"] !== false`                  → true
//   `kindRow(entry, "k"[, LIT])`          → LIT, else kindRow's own `fallback`
// Any other read (`as string | undefined`, tri-state `term:durable`) declares none.

const SECTIONS_DIR = "frontend/app/view/settings/sections";
const LITERAL = String.raw`"(?:[^"\\]|\\.)*"|-?\d[\d_]*(?:\.\d+)?|true|false|\{\}`;
// A named constant used as a fallback (`DEFAULT_MASTER_VOLUME`, `DefaultTermTheme`).
const CONST = String.raw`[A-Z]\w*(?![\w.(])`;

/** A fallback token as a value: a literal, or a named constant resolved to its literal. */
function tokenValue(rel: string, text: string, token: string): unknown {
    if (/^[A-Z]/.test(token)) return resolveConst(rel, text, token);
    return JSON.parse(/^-?\d/.test(token) ? token.replace(/_/g, "") : token);
}

/** Every key a section reads (`keys`), and the defaults its controls declare. */
function uiValues(): { values: Map<string, Found[]>; keys: Set<string> } {
    const out = new Map<string, Found[]>();
    const keys = new Set<string>();
    for (const name of readdirSync(join(ROOT, SECTIONS_DIR))) {
        if (!name.endsWith(".tsx") || name.includes(".test.")) continue;
        const rel = `${SECTIONS_DIR}/${name}`;
        const text = read(rel);
        for (const m of text.matchAll(/(?:\bs\(\)|settingsAtom\(\)\?\.)\["([^"]+)"\]/g)) {
            const key = m[1];
            keys.add(key);
            const before = text.slice(Math.max(0, m.index! - 4), m.index!);
            const after = text.slice(m.index! + m[0].length, m.index! + m[0].length + 120);
            const at = `${rel}:${lineOf(text, m.index!)}`;
            const token = fallbackToken(before, after);
            if (token !== null) push(out, key, { value: tokenValue(rel, text, token), at });
        }
        const kindRow = /const kindRow = \([^)]*\bfallback = (true|false)\)/.exec(text);
        for (const m of text.matchAll(/kindRow\([^,()]+,\s*"([^"]+)"(?:\s*,\s*(true|false))?\s*\)/g)) {
            if (!kindRow) throw new Error(`${rel} calls kindRow but its default fallback wasn't found`);
            keys.add(m[1]);
            push(out, m[1], { value: JSON.parse(m[2] ?? kindRow[1]), at: `${rel}:${lineOf(text, m.index!)}` });
        }
    }
    return { values: out, keys };
}

/**
 * The fallback a read implies, from the text just before and after it: the
 * token after `??` / `||` (past closing parens and `as` casts), "true" for
 * `!== false` / `=== false`, "false" for a negation. Null when it implies none.
 */
function fallbackToken(before: string, after: string): string | null {
    const rest = after.replace(/^(?:\s*\)|\s+as\s+(?:<[^>]*>|[^)?;,}<])+)*/, "");
    const m = new RegExp(String.raw`^\s*(?:\?\?|\|\|)\s*(${LITERAL}|${CONST})`).exec(rest);
    if (m) return m[1];
    if (/^\s*[!=]==\s*false\b/.test(rest)) return "true";
    if (/!\(?\s*$/.test(before)) return "false";
    return null;
}

/** A named constant's literal value, from this file or the module it is imported from. */
function resolveConst(rel: string, text: string, name: string): unknown {
    const local = new RegExp(String.raw`const ${name}(?::[^=]+)?\s*=\s*(${LITERAL}|${CONST})\s*;`).exec(text);
    if (local) return tokenValue(rel, text, local[1]);
    const imp = new RegExp(String.raw`import\s*\{[^}]*\b${name}\b[^}]*\}\s*from\s*"([^"]+)"`).exec(text);
    if (!imp) throw new Error(`${rel}: can't resolve ${name}`);
    const from = imp[1].startsWith("@/") ? `frontend/${imp[1].slice(2)}` : join(dirname(rel), imp[1]);
    const file = [`${from}.ts`, `${from}.tsx`].find((f) => existsSync(join(ROOT, f)));
    if (!file) throw new Error(`${rel}: can't find ${imp[1]}`);
    return resolveConst(file, read(file), name);
}

// ── Where the frontend uses a setting ───────────────────────────────────────
// A read is `getSettingsKeyAtom("k")()`, `getOverrideConfigAtom(id, "k")()` or
// `<…settings…>["k"]`, used directly, or through a variable holding the atom
// (`a = getSettingsKeyAtom("k")`, then `a()`) or the value (`v = <read>;`).
// Each use gets fallbackToken, and a value variable also counts
// `typeof v === "number" ? v : X` and `if (v == null …) return X`.

const ATOM_READ = String.raw`(?:getSettingsKeyAtom|getOverrideConfigAtom)\((?:[^()"]*,\s*)?"([^"]+)"(?:\s+as\s+any)?\)`;
const BRACKET_READ = String.raw`\b\w*[sS]ettings\w*(?:\(\))?(?:\?\.)?\["([^"]+)"\]`;

/** Defaults the frontend applies through a helper or a stylesheet: file, and a pattern capturing the value. */
const FRONTEND_CONSTANTS: Record<string, [string, RegExp]> = {
    "term:scrollback": ["frontend/app/view/term/termscrollback.ts", /const DEFAULT_TERM_SCROLLBACK = (\d+);/],
    "term:fontfamily": ["frontend/app/view/term/termfontfamily.ts", /const DEFAULT_TERM_FONT_FAMILY = ("[^"]*");/],
    "term:scrollsensitivity": [
        "frontend/app/view/term/termscrollsensitivity.ts",
        /const DEFAULT_TERM_SCROLL_SENSITIVITY = ([\d.]+);/,
    ],
    "telemetry:numpoints": ["frontend/app/view/sysinfo/sysinfo-types.ts", /const DefaultNumPoints = (\d+);/],
    "window:tilegapsize": ["frontend/layout/lib/layoutResize.ts", /const DefaultGapSizePx = (\d+);/],
    "window:magnifiedblocksize": ["frontend/layout/lib/layoutGeometry.ts", /magnifiedNodeSizeAtom\) \?\? ([\d.]+);/],
    "window:magnifiedblockopacity": ["frontend/app/block/block.scss", /--magnified-block-opacity: ([\d.]+);/],
};

function frontendFiles(dir: string, out: string[] = []): string[] {
    for (const name of readdirSync(join(ROOT, dir))) {
        const rel = `${dir}/${name}`;
        if (statSync(join(ROOT, rel)).isDirectory()) {
            if (name !== "node_modules" && rel !== "frontend/types" && rel !== SECTIONS_DIR) frontendFiles(rel, out);
        } else if (/\.tsx?$/.test(name) && !/\.(test|bench)\.tsx?$|\.d\.ts$/.test(name)) out.push(rel);
    }
    return out;
}

function appValues(): Map<string, Found[]> {
    const out = new Map<string, Found[]>();
    for (const rel of frontendFiles("frontend")) {
        const text = read(rel);
        const add = (key: string, token: string | null, index: number) => {
            if (token !== null && SCHEMA_KEYS.has(key))
                push(out, key, { value: tokenValue(rel, text, token), at: `${rel}:${lineOf(text, index)}` });
        };
        const at = (start: number, end: number) =>
            fallbackToken(text.slice(Math.max(0, start - 4), start), text.slice(end, end + 120));
        // Direct reads.
        for (const m of text.matchAll(new RegExp(String.raw`${ATOM_READ}\(\)`, "g")))
            add(m[1], at(m.index!, m.index! + m[0].length), m.index!);
        for (const m of text.matchAll(new RegExp(BRACKET_READ, "g")))
            add(m[1], at(m.index!, m.index! + m[0].length), m.index!);
        // Through a variable, up to its next declaration.
        const scope = (name: string, from: number) => {
            const next = new RegExp(String.raw`\b(?:const|let)\s+${name}\b`, "g");
            next.lastIndex = from;
            const end = next.exec(text)?.index ?? text.length;
            return { start: from, body: text.slice(from, Math.min(end, from + 1500)) };
        };
        for (const m of text.matchAll(new RegExp(String.raw`(?:const|let)\s+(\w+)\s*=\s*${ATOM_READ}\s*;`, "g"))) {
            const { start, body } = scope(m[1], m.index! + m[0].length);
            for (const use of body.matchAll(new RegExp(String.raw`(?<![\w.])${m[1]}\(\)`, "g")))
                add(m[2], at(start + use.index!, start + use.index! + use[0].length), m.index!);
        }
        const valueDecl = String.raw`(?:const|let)\s+(\w+)(?::[^=]+)?\s*=\s*(?:untrack\(\(\)\s*=>\s*)?(?:${ATOM_READ}\(\)|(?:\w+\??\.)*${BRACKET_READ})\)?\s*;`;
        for (const m of text.matchAll(new RegExp(valueDecl, "g"))) {
            const [, name, atomKey, bracketKey] = m;
            const key = atomKey ?? bracketKey;
            const { start, body } = scope(name, m.index! + m[0].length);
            for (const use of body.matchAll(new RegExp(String.raw`(?<![\w.])${name}\b(?!\s*[(:=])`, "g")))
                add(key, at(start + use.index!, start + use.index! + use[0].length), m.index!);
            const typed = new RegExp(
                String.raw`typeof ${name} === "\w+"[^?;]*\?\s*${name}\s*:\s*(${LITERAL}|${CONST})`
            );
            const nullish = new RegExp(String.raw`if \(${name} == null[^)]*\)\s*return (${LITERAL}|${CONST})`);
            for (const re of [typed, nullish]) {
                const hit = re.exec(body);
                if (hit) add(key, hit[1], m.index!);
            }
        }
    }
    for (const [key, [rel, re]] of Object.entries(FRONTEND_CONSTANTS)) {
        const text = read(rel);
        const m = re.exec(text);
        if (!m) throw new Error(`FRONTEND_CONSTANTS: ${re} no longer matches in ${rel} (for ${key})`);
        push(out, key, { value: JSON.parse(m[1]), at: `${rel}:${lineOf(text, m.index)}` });
    }
    return out;
}

// ── The backend ─────────────────────────────────────────────────────────────
// Literal fallbacks at the read, in crates/srv and crates/launcher:
//   `setting_bool(extra, "k", LIT)`, and the `&format!("notify:os:{sfx}")` form per suffix
//   `"k")` + value conversions + `.unwrap_or(LIT)` / `.unwrap_or_else(|| LIT.to_string())`
//   `match setting_str(extra, "k") { … _ => Enum::Variant }` → "variant"
// plus BACKEND_CONSTANTS, and a non-`Option` `SettingsType` field, which takes
// its type's zero value when the key is unset.

const BACKEND_DIRS = ["crates/srv/src", "crates/launcher/src"];

/** Defaults the backend keeps in a constant instead of at the read: file, and a pattern capturing the value. */
const BACKEND_CONSTANTS: Record<string, [string, RegExp]> = {
    "attachments:maxfiles": ["crates/srv/src/backend/attachments/mod.rs", /const DEFAULT_MAX_FILES: \w+ = (\d+);/],
    "attachments:maxtotalmb": ["crates/srv/src/backend/attachments/mod.rs", /const DEFAULT_MAX_TOTAL_MB: \w+ = (\d+);/],
    "attachments:sendmaxedge": [
        "crates/srv/src/backend/attachments/mod.rs",
        /const DEFAULT_SEND_MAX_EDGE: \w+ = (\d+);/,
    ],
    "attachments:retentiondays": [
        "crates/srv/src/backend/attachments/mod.rs",
        /const DEFAULT_RETENTION_DAYS: \w+ = (\d+);/,
    ],
    "attachments:claudeinlinemax": [
        "crates/srv/src/backend/attachments/prompt.rs",
        /const DEFAULT_INLINE_MAX_COUNT: \w+ = (\d+);/,
    ],
    "attachments:claudesessioninlinemb": [
        "crates/srv/src/backend/attachments/prompt.rs",
        /const DEFAULT_SESSION_INLINE_MB: \w+ = (\d+);/,
    ],
    "telemetry:interval": ["crates/srv/src/backend/sysinfo.rs", /const DEFAULT_INTERVAL_SECS: f64 = ([\d.]+);/],
    "voice:whisperModel": ["crates/srv/src/server/voice.rs", /const DEFAULT_WHISPER_MODEL: &str = ("[^"]*");/],
    "app:showtray": ["crates/launcher/src/background_config.rs", /\bshow_tray: (true|false),/],
    "app:runinbackground": ["crates/launcher/src/background_config.rs", /\brun_in_background: (true|false),/],
    "app:startatlogin": ["crates/launcher/src/start_at_login.rs", /\.map\(\|v\| v\.unwrap_or\((true|false)\)\)/],
};

const isRustTest = (path: string) =>
    path.split("/").includes("tests") || /\/(tests|\w+_tests|tests_\w+|test_\w+|\w+_test)\.rs$/.test(path);

function rustFiles(dir: string, out: string[] = []): string[] {
    for (const name of readdirSync(join(ROOT, dir))) {
        const rel = `${dir}/${name}`;
        if (statSync(join(ROOT, rel)).isDirectory()) rustFiles(rel, out);
        else if (name.endsWith(".rs") && !isRustTest(rel)) out.push(rel);
    }
    return out;
}

// Between the key and `.unwrap_or`: closing parens and value conversions only,
// so a chain that turns the setting into something else (`quiet_hours_until`)
// isn't read as the setting's default.
const CONVERSIONS = String.raw`(?:\s*\)|\s*\.(?:and_then\(\s*\|\w+\|\s*\w+\.as_\w+\(\)\s*\)|and_then\([\w:]+::as_\w+\)|as_\w+\(\)|cloned\(\)|copied\(\)))*`;
const RUST_LITERAL = String.raw`true|false|-?\d+(?:\.\d+)?|"[^"]*"`;

function backendValues(): Map<string, Found[]> {
    const out = new Map<string, Found[]>();
    const files = BACKEND_DIRS.flatMap((dir) => rustFiles(dir));
    for (const rel of files) {
        // Inline test modules hold fixtures, not defaults.
        const full = read(rel);
        const testMod = /#\[cfg\(test\)\]\s*mod\s+\w+\s*\{/.exec(full);
        const text = testMod ? full.slice(0, testMod.index) : full;
        const add = (key: string, value: unknown, index: number) => {
            if (SCHEMA_KEYS.has(key)) push(out, key, { value, at: `${rel}:${lineOf(text, index)}` });
        };
        for (const m of text.matchAll(
            new RegExp(String.raw`setting_bool\(\s*\w+\s*,\s*"([^"]+)"\s*,\s*(true|false)\s*\)`, "g")
        ))
            add(m[1], JSON.parse(m[2]), m.index!);
        const unwrap = new RegExp(
            String.raw`"([^"]+:[^"]*)"${CONVERSIONS}\s*\.(?:unwrap_or\(\s*(${RUST_LITERAL})\s*\)|unwrap_or_else\(\s*\|\|\s*("[^"]*")\.to_string\(\)\s*\))`,
            "g"
        );
        for (const m of text.matchAll(unwrap)) add(m[1], JSON.parse(m[2] ?? m[3]), m.index!);
        for (const m of text.matchAll(
            /setting_str\(\s*\w+\s*,\s*"([^"]+)"\s*\)\s*\{[^{}]*?_\s*=>\s*\w+::(\w+)\s*,?\s*\}/g
        ))
            add(m[1], m[2].toLowerCase(), m.index!);
        for (const m of text.matchAll(
            /setting_bool\(\s*\w+\s*,\s*&format!\("([^"{]*)\{\w+\}"\)\s*,\s*(true|false)\s*\)/g
        )) {
            for (const suffix of notifySuffixes()) add(m[1] + suffix, JSON.parse(m[2]), m.index!);
        }
    }
    for (const [key, [rel, re]] of Object.entries(BACKEND_CONSTANTS)) {
        const text = read(rel);
        const m = re.exec(text);
        if (!m) throw new Error(`BACKEND_CONSTANTS: ${re} no longer matches in ${rel} (for ${key})`);
        push(out, key, { value: JSON.parse(m[1]), at: `${rel}:${lineOf(text, m.index)}` });
    }
    // Non-Option typed fields: unset means the type's zero value.
    const typesRel = "crates/srv/src/backend/wconfig/types.rs";
    const types = read(typesRel);
    const zero: Record<string, unknown> = { bool: false, f64: 0, i64: 0, u32: 0, u64: 0, String: "" };
    for (const m of types.matchAll(/rename = "([^"]+)"[^\]]*\]\s*pub \w+: (\w+),/g)) {
        if (m[2] in zero && schemaDefault(m[1]) !== undefined)
            push(out, m[1], { value: zero[m[2]], at: `${typesRel}:${lineOf(types, m.index!)}` });
    }
    return out;
}

function notifySuffixes(): string[] {
    const text = read("crates/srv/src/backend/notify/policy.rs");
    const body = /fn setting_suffix\(self\)[^{]*\{([\s\S]*?)\n    \}/.exec(text);
    if (!body) throw new Error("NotifyKind::setting_suffix not found in notify/policy.rs");
    return [...body[1].matchAll(/Some\("([^"]+)"\)/g)].map((m) => m[1]);
}

function push(map: Map<string, Found[]>, key: string, found: Found) {
    const list = map.get(key) ?? [];
    list.push(found);
    map.set(key, list);
}

// ── Compare ─────────────────────────────────────────────────────────────────

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/**
 * With no schema default, the pane's empty fallbacks (`?? ""`, `?? {}`) mean
 * "show the field empty", not a value of their own.
 */
const isEmptyFallback = (v: unknown) => v === "" || (typeof v === "object" && v !== null && !Object.keys(v).length);

function mismatches(sources: Record<Source, Map<string, Found[]>>): Map<string, string> {
    const out = new Map<string, string>();
    for (const key of SCHEMA_KEYS) {
        const schema = schemaDefault(key);
        const found: Found[] = [];
        for (const source of ["template", "ui", "app", "srv"] as Source[]) {
            for (const f of sources[source].get(key) ?? []) {
                if (!schema && isEmptyFallback(f.value)) continue;
                found.push(f);
            }
        }
        const all = schema ? [schema, ...found] : found;
        if (all.length < 2 || all.every((f) => same(f.value, all[0].value))) continue;
        out.set(key, all.map((f) => `${JSON.stringify(f.value)} (${f.at})`).join(" vs "));
    }
    return out;
}

describe("setting defaults agree across the schema, template, Settings pane and backend", () => {
    const ui = uiValues();
    const sources: Record<Source, Map<string, Found[]>> = {
        template: templateValues(),
        ui: ui.values,
        app: appValues(),
        srv: backendValues(),
    };

    it("finds defaults in every source (an extraction pattern stopped matching if not)", () => {
        expect(Object.values(schemaProps).filter((p) => "default" in p).length).toBeGreaterThanOrEqual(40);
        expect(sources.template.size).toBeGreaterThanOrEqual(80);
        expect(sources.ui.size).toBeGreaterThanOrEqual(60);
        expect(sources.app.size).toBeGreaterThanOrEqual(30);
        expect(sources.srv.size).toBeGreaterThanOrEqual(25);
    });

    it("every key the template and the Settings pane name is in the schema", () => {
        expect([...sources.template.keys()].filter((k) => !SCHEMA_KEYS.has(k))).toEqual([]);
        expect([...ui.keys].filter((k) => !SCHEMA_KEYS.has(k))).toEqual([]);
    });

    it("no key has two different defaults", () => {
        const found = mismatches(sources);
        const unexpected = [...found].filter(([key]) => !(key in KNOWN_MISMATCHES)).map(([k, v]) => `${k}: ${v}`);
        expect(unexpected, "Make every place use the schema's default (schema/settings.json)").toEqual([]);
    });

    it("KNOWN_MISMATCHES lists only keys that still disagree", () => {
        const found = mismatches(sources);
        expect(Object.keys(KNOWN_MISMATCHES).filter((key) => !found.has(key))).toEqual([]);
    });
});
