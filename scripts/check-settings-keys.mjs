#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-settings-keys.mjs: CI gate that keeps the settings schema (`schema/settings.json`) and the code that reads
// settings one to one, `docs/specs/SPEC_SETTINGS_JSON_EVERY_KEY_ONE_TO_ONE_2026_11_01.md` §5.3.
//
// What it fails on:
//   read-not-in-schema  a key the code reads (srv's typed settings and `extra` reads, the frontend's settings atom
//                       and bracket reads) that the schema does not list
//   schema-not-read     a schema key that no production source reads: by its name, or through its typed field in srv
//                       (declaring the field does not count). Mark one that is read another way `x-unread`.
//   no-description / no-default / no-section / secret-default / bad-platform
//                       a schema entry that is missing what the generated settings.json shows for it
//   floor               an extractor that finds too few keys: its pattern stopped matching, so it would check nothing
//
// Today's gaps are listed in scripts/check-settings-keys.baseline, each with a reason. A finding that is not listed fails;
// a listed entry that no longer applies fails too, so the list only shrinks (the check-file-sizes.baseline pattern).
// The template drift check is `gen-settings-template.mjs --check`; it runs beside this one in CI.
//
// Usage:
//   node scripts/check-settings-keys.mjs                 check against the baseline
//   node scripts/check-settings-keys.mjs --print         every finding, in baseline form (to write or lower the baseline)

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { isMain } from "./lib/is-main.mjs";

const SCHEMA = "schema/settings.json";
const BASELINE = "scripts/check-settings-keys.baseline";
const PLATFORMS = new Set(["windows", "macos", "linux"]);
/** Fewest keys each kind of read must yield, so a pattern that stops matching fails instead of checking nothing. */
const FLOORS = { frontend: 80, srv: 100 };
const KEY = String.raw`"([a-z][a-z0-9_]*(?::[A-Za-z0-9_.-]+)+)"`;

const FRONTEND_READS = [
    new RegExp(String.raw`getSettingsKeyAtom(?:<[^>(]*>)?\(\s*${KEY}`, "g"),
    // `settings[..]`, not `conn().settings[..]`: a connection has settings of its own.
    new RegExp(String.raw`(?:\bs\(\)|settingsAtom\(\)\??\.|(?<![.\w)])settings\??\.?|fullConfig\??\.settings\??\.?)\??\[${KEY}\]`, "g"),
    new RegExp(String.raw`(?:useSettingsKey|useSetting|getSetting)\w*\(\s*${KEY}`, "g"),
];
const SRV_READS = [
    new RegExp(String.raw`\.extra(?:_\w+)?\.get\(\s*${KEY}`, "g"),
    new RegExp(String.raw`\bextra_\w+\(\s*${KEY}`, "g"),
    new RegExp(String.raw`\bsetting_\w+\(\s*[^,()]*,\s*${KEY}`, "g"),
];
const SETTINGS_STRUCT = "pub struct SettingsType {";
const TYPED_FIELD = new RegExp(String.raw`#\[serde\([^\]]*rename\s*=\s*${KEY}[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*pub\s+(\w+)\s*:`, "g");

/**
 * The index just past a Rust string, raw string, char literal or comment that starts at `i`, or `i` when none does, so
 * a brace inside one is not counted when a block's end is looked for.
 */
function skipLiteral(s, i) {
    const raw = /^r(#*)"/.exec(s.slice(i, i + 40));
    if (raw && !/\w/.test(s[i - 1] ?? " ")) {
        const close = s.indexOf(`"${raw[1]}`, i + raw[0].length);
        return close < 0 ? s.length : close + 1 + raw[1].length;
    }
    if (s[i] === '"') {
        for (let j = i + 1; j < s.length; j++) {
            if (s[j] === "\\") j++;
            else if (s[j] === '"') return j + 1;
        }
        return s.length;
    }
    if (s[i] === "'" && (s[i + 1] === "\\" ? s[i + 3] === "'" || s[i + 4] === "'" : s[i + 2] === "'")) return s.indexOf("'", i + 2) + 1;
    if (s.startsWith("//", i)) return s.indexOf("\n", i) < 0 ? s.length : s.indexOf("\n", i);
    if (s.startsWith("/*", i)) return s.indexOf("*/", i + 2) < 0 ? s.length : s.indexOf("*/", i + 2) + 2;
    return i;
}

/** The index just past the block whose opening brace is already consumed, ending at `from`. */
function blockEnd(s, from) {
    let depth = 1;
    let i = from;
    while (i < s.length && depth > 0) {
        const j = skipLiteral(s, i);
        if (j !== i) {
            i = j;
            continue;
        }
        if (s[i] === "{") depth++;
        else if (s[i] === "}") depth--;
        i++;
    }
    return i;
}

/** `text` without `#[cfg(test)] mod x { ... }` blocks, whose fixtures name keys no real code reads. */
export function withoutTestModules(text) {
    let out = text;
    for (;;) {
        const m = /#\[cfg\(test\)\]\s*mod\s+\w+\s*\{/.exec(out);
        if (!m) return out;
        out = out.slice(0, m.index) + out.slice(blockEnd(out, m.index + m[0].length));
    }
}

function matches(text, patterns) {
    const keys = new Set();
    for (const re of patterns) for (const m of text.matchAll(re)) keys.add(m[1]);
    return keys;
}

/** The bounds of the typed settings struct in `text`: `[start, end)`, or null when it is not there. */
function settingsStruct(text) {
    const start = text.indexOf(SETTINGS_STRUCT);
    if (start < 0) return null;
    const end = text.indexOf("\n}", start);
    return [start, end < 0 ? text.length : end];
}

/** The keys the typed settings struct names, as `Map<key, field>`: its `rename`d fields, and nothing from the structs around it. */
function typedKeys(text) {
    const bounds = settingsStruct(text);
    const out = new Map();
    if (!bounds) return out;
    for (const m of text.slice(bounds[0], bounds[1]).matchAll(TYPED_FIELD)) out.set(m[1], m[2]);
    return out;
}

/** `text` without the typed settings struct, which declares keys and does not read them. */
function withoutSettingsStruct(text) {
    const bounds = settingsStruct(text);
    return bounds ? text.slice(0, bounds[0]) + text.slice(bounds[1]) : text;
}

/**
 * Every key the code reads. `sources` is `[{ path, text }]` for production files only (tests excluded by the caller);
 * a `.rs` file has its test modules removed here.
 */
export function readKeys(sources) {
    const frontend = new Set();
    const srv = new Set();
    const typed = new Map();
    for (const { path, text } of sources) {
        if (path.endsWith(".rs")) {
            const prod = withoutTestModules(text);
            for (const k of matches(prod, SRV_READS)) srv.add(k);
            for (const [k, field] of typedKeys(prod)) {
                srv.add(k);
                typed.set(k, field);
            }
        } else {
            for (const k of matches(text, FRONTEND_READS)) frontend.add(k);
        }
    }
    return { frontend, srv, typed };
}

/** The schema's entries: `{ key: entry }` for a real setting, and the `ns:` prefixes of its `ns:*` families. */
export function schemaEntries(schema) {
    const props = schema.$defs.SettingsType.properties;
    const keys = {};
    const families = [];
    for (const [k, v] of Object.entries(props)) {
        if (k.endsWith(":*")) families.push(k.slice(0, -1));
        else keys[k] = v;
    }
    return { keys, families };
}

/** Every finding as `{ kind, key, detail }`, sorted. */
export function check(schema, sources) {
    const { keys, families } = schemaEntries(schema);
    const { frontend, srv, typed } = readKeys(sources);
    const out = [];
    const add = (kind, key, detail = "") => out.push({ kind, key, detail });
    if (frontend.size < FLOORS.frontend) add("floor", "frontend", `${frontend.size} keys found, at least ${FLOORS.frontend} expected`);
    if (srv.size < FLOORS.srv) add("floor", "srv", `${srv.size} keys found, at least ${FLOORS.srv} expected`);
    const inSchema = (k) => k in keys || families.some((f) => k.startsWith(f));
    for (const k of new Set([...frontend, ...srv])) if (!inSchema(k)) add("read-not-in-schema", k);
    // A key is read when production code names it, or uses its typed field; the struct that declares the fields does not count.
    const text = sources.map((s) => (s.path.endsWith(".rs") ? withoutSettingsStruct(withoutTestModules(s.text)) : s.text)).join("\n");
    const isRead = (k) => text.includes(`"${k}"`) || (typed.has(k) && new RegExp(String.raw`\.${typed.get(k)}\b`).test(text));
    for (const [k, v] of Object.entries(keys)) {
        if (!v["x-unread"] && !isRead(k)) add("schema-not-read", k);
        if (!v.description) add("no-description", k);
        if (!v["x-section"]) add("no-section", k);
        if (!("default" in v) && !("x-nodefault" in v)) add("no-default", k);
        if (v["x-secret"] && v.default) add("secret-default", k, "a secret's commented default must be empty");
        for (const p of [v["x-platform"]].flat().filter(Boolean)) if (!PLATFORMS.has(p)) add("bad-platform", k, `unknown platform ${p}`);
    }
    return out.sort((a, b) => a.kind.localeCompare(b.kind) || a.key.localeCompare(b.key));
}

/** The baseline as a Map `"kind key" -> reason`. A line is `kind<TAB>key<TAB>reason`; `#` starts a comment. */
export function parseBaseline(text) {
    const map = new Map();
    for (const line of text.split(/\r?\n/)) {
        if (!line.trim() || line.startsWith("#")) continue;
        const [kind, key, ...reason] = line.split("\t");
        map.set(`${kind} ${key}`, reason.join("\t").trim());
    }
    return map;
}

/** What fails: findings not in the baseline, and baseline entries that no longer apply. A `floor` is never baselined. */
export function compare(findings, baseline) {
    const now = new Set(findings.map((f) => `${f.kind} ${f.key}`));
    const added = findings.filter((f) => f.kind === "floor" || !baseline.has(`${f.kind} ${f.key}`));
    const stale = [...baseline.keys()].filter((k) => !now.has(k));
    const noReason = [...baseline].filter(([, reason]) => !reason).map(([k]) => k);
    return { added, stale, noReason };
}

function productionSources() {
    const files = execFileSync("git", ["ls-files", "frontend", "crates"], { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }).split("\n").filter(Boolean);
    const isProd = (f) => /\.(ts|tsx|rs)$/.test(f) && !/(\.test\.|_tests?\.rs$|\/tests\/|\.d\.ts$|^frontend\/types\/)/.test(f);
    return files.filter(isProd).map((path) => ({ path, text: readFileSync(path, "utf8") }));
}

function main() {
    const findings = check(JSON.parse(readFileSync(SCHEMA, "utf8")), productionSources());
    if (process.argv.includes("--print")) {
        for (const f of findings) console.log(`${f.kind}\t${f.key}\t${f.detail}`);
        return;
    }
    const baseline = existsSync(BASELINE) ? parseBaseline(readFileSync(BASELINE, "utf8")) : new Map();
    const { added, stale, noReason } = compare(findings, baseline);
    for (const f of added) console.error(`FAIL ${f.kind} ${f.key}${f.detail ? `: ${f.detail}` : ""}`);
    for (const k of stale) console.error(`FAIL stale baseline entry "${k}": it no longer applies, so remove it from ${BASELINE}`);
    for (const k of noReason) console.error(`FAIL baseline entry "${k}" has no reason`);
    if (added.length + stale.length + noReason.length > 0) {
        console.error("\nschema/settings.json and the code that reads settings must list the same keys: add the key to the schema (key, type, default, description, x-section), or remove the read. See docs/specs/SPEC_SETTINGS_JSON_EVERY_KEY_ONE_TO_ONE_2026_11_01.md.");
        process.exit(1);
    }
    console.log(`settings keys: ${findings.length} known gap(s) in the baseline, nothing new.`);
}

if (isMain(import.meta.url)) main();
