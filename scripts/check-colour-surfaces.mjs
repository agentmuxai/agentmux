// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-colour-surfaces.mjs: keeps the colour a person picked the colour they see.
// docs/specs/SPEC_PICKED_COLOUR_IS_THE_COLOUR_SEEN_2026_11_18.md §7.
//
// A user-coloured element (a coloured window tab, a coloured pane tab pill, a palette swatch) takes every colour
// from theme/paint.ts, one tone per state. A `filter`, `backdrop-filter`, `mix-blend-mode` or `opacity` declared on
// such an element changes the picked colour in a way the picker cannot show, and a state rule that has to win with
// `!important` hides the other states. Both fail here. ALLOW names the rules that still exist, each with the phase
// that removes it; an entry that no longer matches anything fails too, so the list only shrinks.
//
//   node scripts/check-colour-surfaces.mjs          the gate (the test of the same name runs it in CI)

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Selectors that name a user-coloured element. */
export const COLOURED = [/\.tab-colored\b/, /\.pane-tab--colored\b/, /\.pane-tab-colored\b/, /\.palette-swatch\b/];

/** Declarations that alter a colour from outside it. */
const FORBIDDEN = /^(filter|backdrop-filter|mix-blend-mode|opacity)$/;

/**
 * Rules that still break the above, with the phase of the spec that removes each. `file` is repo-relative; `selector`
 * is the flattened selector the rule resolves to, and `property` the declaration.
 */
export const ALLOW = [];

/** `text` with comments removed, newlines kept so a line number still counts. */
export function stripComments(text) {
    return text
        .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " "))
        .replace(/(^|[^:"'(])\/\/[^\n]*/g, (m, lead) => lead + " ".repeat(m.length - lead.length));
}

/** The comma-separated parts of a selector list, outside brackets and parentheses. */
function splitSelectors(list) {
    const parts = [];
    let depth = 0;
    let cur = "";
    for (const ch of list) {
        if (ch === "(" || ch === "[") depth++;
        if (ch === ")" || ch === "]") depth--;
        if (ch === "," && depth === 0) {
            parts.push(cur.trim());
            cur = "";
        } else cur += ch;
    }
    if (cur.trim()) parts.push(cur.trim());
    return parts;
}

/** `child` nested in `parent`: `&` stands for the parent, otherwise a descendant. */
const nest = (parent, child) => (child.includes("&") ? child.replaceAll("&", parent) : parent ? `${parent} ${child}` : child);

/**
 * Every declaration of `css` (SCSS) with the selectors it resolves to: `[{ selectors, property, value, line }]`.
 * Nested rules are flattened; at-rules (`@media`, `@include` blocks, `@keyframes`) are not selectors and are passed through.
 */
export function declarations(css) {
    const text = stripComments(css);
    const out = [];
    const stack = []; // each: array of flattened selectors, or null for an at-rule
    let buf = "";
    let line = 1;
    let startLine = 1;
    const current = () => {
        for (let i = stack.length - 1; i >= 0; i--) if (stack[i]) return stack[i];
        return [];
    };
    for (const ch of text) {
        if (ch === "\n") line++;
        if (ch === "{") {
            const head = buf.trim();
            buf = "";
            if (head.startsWith("@")) stack.push(null);
            else {
                const parents = current();
                const own = splitSelectors(head);
                stack.push(own.flatMap((s) => (parents.length ? parents.map((p) => nest(p, s)) : [s])));
            }
            startLine = line;
        } else if (ch === "}") {
            stack.pop();
            buf = "";
        } else if (ch === ";") {
            const m = /^\s*([a-z-]+)\s*:\s*([\s\S]*)$/.exec(buf);
            if (m && current().length) out.push({ selectors: current(), property: m[1], value: m[2].trim(), line: startLine });
            buf = "";
        } else {
            if (buf.trim() === "") startLine = line;
            buf += ch;
        }
    }
    return out;
}

// `:not(.tab-colored)` names an uncoloured element, not a coloured one.
const isColoured = (selector) => COLOURED.some((re) => re.test(selector.replace(/:not\([^)]*\)/g, "")));

/** The violations of one stylesheet: `[{ file, line, selector, property, why }]`, `ALLOW`ed ones left out. */
export function checkFile(file, css) {
    const found = [];
    for (const d of declarations(css)) {
        const forbidden = FORBIDDEN.test(d.property);
        const important = /!important/.test(d.value) && /^(background|background-color)$/.test(d.property);
        if (!forbidden && !important) continue;
        for (const selector of d.selectors.filter(isColoured)) {
            found.push({ file, line: d.line, selector, property: d.property, why: forbidden ? `${d.property} on a user-coloured element` : "!important on a fill" });
        }
    }
    return found;
}

function stylesheets(dir, out = []) {
    for (const name of readdirSync(dir)) {
        if (name === "node_modules" || name.startsWith(".")) continue;
        const p = join(dir, name);
        if (statSync(p).isDirectory()) stylesheets(p, out);
        else if (/\.scss$/.test(name)) out.push(p);
    }
    return out;
}

const allowed = (v) => ALLOW.some((a) => a.file === v.file && a.selector === v.selector && a.property === v.property);

/** `{ violations, stale }`: every violation not in `ALLOW`, and every `ALLOW` entry nothing matches any more. */
export function collect(root = REPO_ROOT) {
    const all = stylesheets(join(root, "frontend")).flatMap((p) => checkFile(relative(root, p).replaceAll("\\", "/"), readFileSync(p, "utf8")));
    return {
        violations: all.filter((v) => !allowed(v)),
        stale: ALLOW.filter((a) => !all.some((v) => a.file === v.file && a.selector === v.selector && a.property === v.property)),
    };
}

function main() {
    const { violations, stale } = collect();
    for (const v of violations) console.error(`${v.file}:${v.line}: ${v.why} (${v.selector} { ${v.property} })`);
    for (const a of stale) console.error(`ALLOW entry no longer matches, remove it: ${a.file} ${a.selector} ${a.property}`);
    if (violations.length || stale.length) {
        console.error("\nA user-coloured element takes each state from theme/paint.ts: a tone per state, no filter, opacity or blend over it, no !important on its fill.");
        process.exit(1);
    }
    console.log("check-colour-surfaces: ok");
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
