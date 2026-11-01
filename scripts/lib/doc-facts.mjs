// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Facts a doc states that the code can give: a count, a range, or a list of names. Used by
// check-doc-references.mjs with `--facts`. Each fact names the doc page (relative to the
// `--docs` folder), reads the figure or the names from it, reads the same from the code,
// and reports a difference. A page that is not there is skipped, so a docs repo that does
// not have the page is not failed for it.

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { maskRust } from "./source-mask.mjs";

// ── Reading the docs ────────────────────────────────────────────────────────

/** The lines of `text` from the heading that matches `start` up to the next heading of the same or a higher level (or the line that matches `until`). */
export function section(text, start, until = null) {
    const lines = text.split("\n");
    const at = lines.findIndex((l) => start.test(l));
    if (at < 0) return null;
    const level = /^(#{1,6})\s/.exec(lines[at])?.[1].length ?? 6;
    const out = [];
    for (let i = at + 1; i < lines.length; i++) {
        if (until?.test(lines[i])) break;
        const h = /^(#{1,6})\s/.exec(lines[i]);
        if (!until && h && h[1].length <= level) break;
        out.push({ n: i + 1, text: lines[i] });
    }
    return out;
}

const backticked = (s) => [...s.matchAll(/`([^`\n]+)`/g)].map((m) => m[1]);

/** Table rows of `lines` (after the header and its rule), as arrays of cells. */
export function tableRows(lines) {
    const rows = [];
    let header = true;
    for (const { n, text } of lines) {
        const t = text.trim();
        if (!t.startsWith("|")) {
            header = true;
            continue;
        }
        if (/^\|?\s*:?-{2,}/.test(t)) {
            header = false;
            continue;
        }
        if (header) continue;
        rows.push({ n, cells: t.replace(/^\|/, "").replace(/\|\s*$/, "").split(/(?<!\\)\|/).map((c) => c.trim()) });
    }
    return rows;
}

// ── Reading the code ────────────────────────────────────────────────────────

const read = (code, path) => (existsSync(join(code, path)) ? readFileSync(join(code, path), "utf8") : null);

/** Index of the bracket closing the one at `m[open]`. */
function closeOf(m, open) {
    const o = m[open];
    const c = o === "(" ? ")" : o === "[" ? "]" : "}";
    let depth = 0;
    for (let j = open; j < m.length; j++) {
        if (m[j] === o) depth++;
        else if (m[j] === c && --depth === 0) return j;
    }
    return -1;
}

/** The variants of `enum <name>` in Rust source `src`, or null. */
export function rustEnumVariants(src, name) {
    const m = maskRust(src);
    const head = new RegExp(`\\benum\\s+${name}\\b[^{]*\\{`).exec(m);
    if (!head) return null;
    const open = head.index + head[0].length - 1;
    const close = closeOf(m, open);
    const out = [];
    let depth = 0;
    let atStart = true;
    for (let i = open + 1; i < close; i++) {
        const ch = m[i];
        if (ch === "{" || ch === "(" || ch === "[") depth++;
        else if (ch === "}" || ch === ")" || ch === "]") depth--;
        else if (depth === 0) {
            if (ch === ",") atStart = true;
            else if (atStart && /[A-Za-z_]/.test(ch)) {
                const id = /^[A-Za-z_]\w*/.exec(m.slice(i))[0];
                out.push(id);
                i += id.length - 1;
                atStart = false;
            } else if (atStart && ch === "#") {
                // an attribute on the variant: skip to its closing bracket
                const end = closeOf(m, m.indexOf("[", i));
                if (end > 0) i = end;
            }
        }
    }
    return out;
}

/** The string literals between the brackets that open at the first `opener` match after `anchor` in raw source `src`. */
function stringsAfter(src, anchor, opener) {
    const m = maskRust(src);
    const at = m.search(anchor);
    if (at < 0) return null;
    const rest = opener.exec(m.slice(at));
    if (!rest) return null;
    const open = at + rest.index + rest[0].length - 1;
    const close = closeOf(m, open);
    return [...src.slice(open, close).matchAll(/"([^"\n]+)"/g)].map((x) => x[1]);
}

// ── Comparing ───────────────────────────────────────────────────────────────

const sorted = (set) => [...set].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));

/** Findings for two name sets that should be equal: `docs` and `code` map a name to the doc line (or 0). */
function sameNames(page, what, docs, code, where) {
    const out = [];
    for (const name of sorted(docs.keys())) {
        if (!code.has(name)) out.push({ kind: "fact", text: `${what}:${name}`, line: docs.get(name), message: `${what}: \`${name}\` is documented but is not ${where}` });
    }
    for (const name of sorted(code)) {
        if (!docs.has(name)) out.push({ kind: "fact", text: `${what}:${name}`, line: 0, message: `${what}: \`${name}\` is ${where} but the page does not list it` });
    }
    return out.map((f) => ({ ...f, page }));
}

function sameNumber(page, what, found, actual, source) {
    return found
        .filter(({ value }) => value !== actual)
        .map(({ value, n }) => ({ kind: "fact", text: `${what}:${value}`, line: n, page, message: `${what}: the page says ${value}, ${source} says ${actual}` }));
}

function numbersIn(text, re) {
    const out = [];
    text.split("\n").forEach((line, i) => {
        for (const m of line.matchAll(re)) {
            const value = Number(m.slice(1).find((g) => g !== undefined));
            out.push({ value, n: i + 1 });
        }
    });
    return out;
}

// ── The facts ───────────────────────────────────────────────────────────────

/**
 * The facts: `run(text, code, files, page)` reads the page text and the code (`code` is the root of
 * the code tree, `files` its tracked files) and returns findings `{ kind, text, line, message }`;
 * `text` identifies a finding in a baseline.
 */
const RULES = [
    {
        what: "migration range",
        page: "internals/data-structure.md",
        run(text, code, files) {
            const nums = files.map((f) => /^crates\/srv\/src\/migrations\/m(\d{4})_/.exec(f)?.[1]).filter(Boolean).map(Number);
            if (!nums.length) return [];
            const last = Math.max(...nums);
            return sameNumber("internals/data-structure.md", "migration range", numbersIn(text, /`m0000`\s*[–-]\s*`m(\d{4})`/g), last, "the highest crates/srv/src/migrations/mNNNN file");
        },
    },
    {
        what: "wan.db schema counter",
        page: "internals/data-structure.md",
        run(text, code) {
            const src = read(code, "crates/srv/src/backend/storage/wan_identity.rs");
            const actual = src && Number(/\bconst SCHEMA_VERSION:\s*i64\s*=\s*(\d+)/.exec(src)?.[1]);
            if (!actual) return [];
            return sameNumber("internals/data-structure.md", "wan.db schema counter", numbersIn(text, /own counter,\s*(\d+)\)|schema counter\s*\((\d+)\)/g), actual, "SCHEMA_VERSION in wan_identity.rs");
        },
    },
    {
        what: "HostCommand variants",
        page: "internals/reducer-stack.md",
        extra: ["internals/architecture.md"],
        run(text, code, files, page) {
            const src = read(code, "crates/cef/src/reducer/mod.rs");
            const variants = src && rustEnumVariants(src, "HostCommand");
            if (!variants?.length) return [];
            const re = page === "internals/architecture.md" ? /(\d+) host-internal commands/g : /`HostCommand`\s*\((\d+) variants\)/g;
            return sameNumber(page, "HostCommand variants", numbersIn(text, re), variants.length, "the HostCommand enum");
        },
    },
    {
        what: "MuxAPI tools",
        page: "internals/muxapi.md",
        run(text, code) {
            const src = read(code, "crates/mcp/src/tool_schemas.rs");
            const body = section(text, /^## MCP tools \(the agent-facing slice\)/);
            if (!src || !body) return [];
            const names = new Map();
            for (const { n, text: line } of body) {
                for (const m of line.matchAll(/\[`([A-Za-z]+)`\]\(#/g)) if (!names.has(m[1])) names.set(m[1], n);
            }
            const inCode = new Set([...src.matchAll(/^ {2}"name":\s*"([A-Za-z]+)"/gm)].map((m) => m[1]));
            return sameNames("internals/muxapi.md", "MuxAPI tools", names, inCode, "a tool in tool_schemas.rs");
        },
    },
    {
        what: "drift kinds",
        page: "internals/wrr.md",
        run(text, code) {
            const src = read(code, "crates/common/src/ipc.rs");
            const variants = src && rustEnumVariants(src, "HwndDriftKind");
            const body = section(text, /^## The drift kinds/);
            if (!variants || !body) return [];
            const names = new Map();
            for (const { n, cells } of tableRows(body)) for (const b of backticked(cells[0] ?? "")) names.set(b, n);
            return sameNames("internals/wrr.md", "drift kinds", names, new Set(variants), "a variant of HwndDriftKind");
        },
    },
    {
        what: "host commands",
        page: "internals/ipc-catalog.md",
        run(text, code) {
            const src = read(code, "crates/cef/src/ipc.rs");
            const stubSrc = read(code, "crates/cef/src/commands/stubs.rs");
            const body = section(text, /^### Command Catalog/, /^## 4\. Channel B/);
            if (!src || !body) return [];
            const stubs = new Set((stubSrc && stringsAfter(stubSrc, /fn is_stub_command/, /matches!\(/)) || []);
            const names = new Map();
            let other = false;
            for (const { n, text: line } of body) {
                if (/^#{1,6}\s/.test(line)) other = /Other commands/.test(line);
                const t = line.trim();
                if (!t.startsWith("|") || /^\|?\s*:?-{2,}/.test(t)) continue;
                const cells = t.replace(/^\|/, "").replace(/\|\s*$/, "").split(/(?<!\\)\|/).map((c) => c.trim());
                const mine = other ? cells.slice(1).join(" ") : (cells[0] ?? "");
                for (const b of backticked(mine)) if (/^[a-z][a-z0-9_]*$/.test(b) && !names.has(b)) names.set(b, n);
            }
            const arms = routeCommandArms(src);
            if (!arms.size) return [];
            for (const s of stubs) arms.delete(s);
            for (const s of stubs) names.delete(s);
            return sameNames("internals/ipc-catalog.md", "host commands", names, arms, "a command of route_command");
        },
    },
    {
        what: "modal kinds",
        page: "internals/modal-system.md",
        run(text, code) {
            const src = read(code, "frontend/app/element/modal-layer.ts");
            if (!src) return [];
            const lines = text.split("\n");
            const at = lines.findIndex((l) => l.includes("The request kinds today are"));
            if (at < 0) return [];
            const inCode = new Set([...src.matchAll(/\bkind:\s*"([a-z0-9-]+)"/g)].map((m) => m[1]));
            const names = new Map();
            for (const b of backticked(lines[at].slice(lines[at].indexOf("The request kinds today are")))) {
                if (/^[a-z][a-z0-9-]*-\*$/.test(b)) {
                    // `bundle-import-*` stands for every kind that starts `bundle-import-`.
                    for (const k of inCode) if (k.startsWith(b.slice(0, -1))) names.set(k, at + 1);
                } else if (/^[a-z][a-z0-9-]*$/.test(b)) names.set(b, at + 1);
            }
            return sameNames("internals/modal-system.md", "modal kinds", names, inCode, "a kind in modal-layer.ts");
        },
    },
    {
        what: "pane env allow-list",
        page: "internals/env-vars.md",
        run(text, code) {
            const src = read(code, "crates/srv/src/backend/pane_env.rs");
            const body = section(text, /^### The pane allow-list \(`PANE_ENV_KEEP`\)/);
            const keep = src && stringsAfter(src, /\bPANE_ENV_KEEP:\s*&\[&str\]/, /=\s*&\[/);
            if (!keep || !body) return [];
            const names = new Map();
            for (const { n, cells } of tableRows(body)) for (const b of backticked(cells.slice(1).join(" "))) names.set(b, n);
            return sameNames("internals/env-vars.md", "pane env allow-list", names, new Set(keep), "on PANE_ENV_KEEP");
        },
    },
];

/** The string arms (`"name" =>`, `"a" | "b" =>`) of `match cmd` in `route_command`, as a Set. */
export function routeCommandArms(src) {
    const m = maskRust(src);
    const fn = m.search(/\bfn route_command\b/);
    if (fn < 0) return new Set();
    const mt = /\n([ \t]*)match cmd\s*\{/.exec(m.slice(fn));
    if (!mt) return new Set();
    const open = fn + mt.index + mt[0].length - 1;
    const close = closeOf(m, open);
    const arm = new RegExp(`^${mt[1]}    ((?:"[a-z0-9_]+"\\s*\\|\\s*)*"[a-z0-9_]+")\\s*(?:if [^=]*)?=>`, "gm");
    const out = new Set();
    for (const a of src.slice(open, close).matchAll(arm)) for (const s of a[1].matchAll(/"([a-z0-9_]+)"/g)) out.add(s[1]);
    return out;
}

/** Findings of every fact whose page is under `docsDir`: `{ page, kind, text, line, message }`. */
export function checkFacts(docsDir, code, files) {
    const out = [];
    for (const rule of RULES) {
        for (const page of [rule.page, ...(rule.extra ?? [])]) {
            const full = join(docsDir, page);
            let text;
            for (const ext of ["", "x"]) if (!text && existsSync(full + ext)) text = readFileSync(full + ext, "utf8");
            if (text === undefined) continue;
            out.push(...rule.run(text, code, files, page).map((f) => ({ ...f, page })));
        }
    }
    return out;
}
