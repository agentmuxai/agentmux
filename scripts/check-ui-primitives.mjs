// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-ui-primitives.mjs — CI ratchet for the line-style component set.
//
// docs/specs/SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §7
//
// THE RULES. Controls come from frontend/app/element/ui/ and follow its line
// style. Four counts are held at a checked-in baseline
// (scripts/ui-primitives-baseline.json) that may only go down:
//
//   1. buttons      hand-rolled `<button` elements in .tsx outside element/ui/,
//                   per file
//   2. solidFills   SCSS rules on a control (a button, tab, toggle or an
//                   active/primary/confirm/danger state) whose background is a
//                   solid `var(--accent-color|--error-color|--success-color)`
//   3. radius       `border-radius` values other than 0, `var(--radius-*)`,
//                   50%, inherit or none, per file (SPEC_HARD_CORNERS_2026_05_26)
//   4. undefinedVars `var(--x)` where `--x` is declared nowhere in frontend/
//                   (not in SCSS/CSS, not set from TS). Every use silently
//                   takes its fallback: settings.scss's `--panel-bg-alt` was
//                   a fixed grey in every theme.
//
// A count above the baseline fails: use the element/ui/ components instead.
// A count below it also fails, so the baseline drops with each migration:
// run `node scripts/check-ui-primitives.mjs --update` and commit the result.
// Adding to the baseline by hand needs a reason in the PR.
//
// Why a script and not stylelint: stylelint isn't run in CI (see
// scripts/check-no-transition-all.sh), and rule 2 needs SCSS nesting resolved.
//
// Usage:
//   node scripts/check-ui-primitives.mjs            # check
//   node scripts/check-ui-primitives.mjs --update   # rewrite the baseline

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const FRONTEND = join(REPO_ROOT, "frontend");
const BASELINE_PATH = join(REPO_ROOT, "scripts", "ui-primitives-baseline.json");
const UI_DIR = "frontend/app/element/ui/";

// ── File walking ────────────────────────────────────────────────────────────

function walk(dir, out = []) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        if (entry.name === "node_modules" || entry.name === "dist") continue;
        const full = join(dir, entry.name);
        if (entry.isDirectory()) walk(full, out);
        else out.push(full);
    }
    return out;
}

const rel = (p) => relative(REPO_ROOT, p).split(sep).join("/");
const isTest = (p) => /\.test\.(tsx?|mjs)$/.test(p);

// ── Rule 1: hand-rolled buttons ─────────────────────────────────────────────

export function countButtons(source) {
    return (source.match(/<button\b/g) ?? []).length;
}

// ── SCSS walking (rules 2 and 3) ────────────────────────────────────────────

/** Remove comments, keeping line breaks and string contents intact. */
export function stripComments(scss) {
    let out = "";
    let i = 0;
    let quote = null;
    while (i < scss.length) {
        const c = scss[i];
        const next = scss[i + 1];
        if (quote) {
            out += c;
            if (c === "\\") {
                out += next ?? "";
                i += 2;
                continue;
            }
            if (c === quote) quote = null;
            i++;
        } else if (c === '"' || c === "'") {
            quote = c;
            out += c;
            i++;
        } else if (c === "/" && next === "*") {
            const end = scss.indexOf("*/", i + 2);
            const stop = end === -1 ? scss.length : end + 2;
            out += scss.slice(i, stop).replace(/[^\n]/g, "");
            i = stop;
        } else if (c === "/" && next === "/" && scss[i - 1] !== ":") {
            // `//` comment; `:` guard keeps `url(https://…)`.
            while (i < scss.length && scss[i] !== "\n") i++;
        } else {
            out += c;
            i++;
        }
    }
    return out;
}

/** Resolve a nested selector against its parent, SCSS-style (`&` or descendant). */
export function resolveSelector(parent, child) {
    const children = child.split(",").map((s) => s.trim()).filter(Boolean);
    if (!parent) return children.join(", ");
    const parents = parent.split(",").map((s) => s.trim());
    const out = [];
    for (const p of parents) {
        for (const c of children) out.push(c.includes("&") ? c.replace(/&/g, p) : `${p} ${c}`);
    }
    return out.join(", ");
}

/**
 * Every declaration in a stylesheet, with the resolved selector it sits
 * under. At-rules (@media, @container, @include with a block) don't add to
 * the selector; @mixin and @function bodies are skipped entirely, since
 * their selectors only mean something where they are included.
 */
export function declarations(scss) {
    const text = stripComments(scss);
    const stack = []; // { selector, skip }
    const out = [];
    let buf = "";
    let line = 1;
    let bufLine = 1;
    let interpolation = 0; // depth inside `#{…}`, which is not a block
    for (const c of text) {
        if (c === "\n") line++;
        if (c === "{" && buf.endsWith("#")) {
            interpolation++;
            buf += c;
        } else if (c === "}" && interpolation > 0) {
            interpolation--;
            buf += c;
        } else if (c === "{") {
            const head = buf.trim();
            const parent = stack.length ? stack[stack.length - 1] : { selector: "", skip: false };
            const skip = parent.skip || /^@(mixin|function)\b/.test(head);
            const selector = head.startsWith("@") ? parent.selector : resolveSelector(parent.selector, head);
            stack.push({ selector, skip });
            buf = "";
            bufLine = line;
        } else if (c === "}") {
            flush();
            stack.pop();
            buf = "";
            bufLine = line;
        } else if (c === ";") {
            flush();
            buf = "";
            bufLine = line;
        } else {
            if (!buf.trim()) bufLine = line;
            buf += c;
        }
    }
    return out;

    function flush() {
        const decl = buf.trim();
        const top = stack[stack.length - 1];
        if (!top || top.skip) return;
        const m = /^([a-z-]+)\s*:\s*([\s\S]+)$/i.exec(decl);
        if (!m) return;
        out.push({ selector: top.selector, property: m[1].toLowerCase(), value: m[2].trim(), line: bufLine });
    }
}

// The token alone, or with a fallback: `var(--error-color, #f38ba8)`.
const SOLID_FILL = /^var\(--(accent|error|success)-color(\s*,[^)]*)?\)\s*(!important)?$/;
const CONTROL_SELECTOR =
    /(btn|button|tab|toggle)\b|is-active|is-selected|--active\b|--on\b|--primary\b|--confirm\b|--destructive\b|--danger\b|\.solid\b|\.primary\b/i;

/** True when every selector in the list targets a pseudo-element (an underline, a marker), not the control. */
const onlyPseudoElements = (selector) => selector.split(",").every((s) => /::?(after|before)\s*$/.test(s.trim()));

export function solidFills(scss) {
    return declarations(scss)
        .filter((d) => (d.property === "background" || d.property === "background-color") && SOLID_FILL.test(d.value))
        .filter((d) => CONTROL_SELECTOR.test(d.selector) && !onlyPseudoElements(d.selector))
        .map((d) => d.selector.replace(/\s+/g, " "));
}

const RADIUS_OK = /^(0|0px|50%|inherit|none|initial|unset|var\(--radius-[a-z]+\))$/;

export function badRadii(scss) {
    return declarations(scss)
        .filter((d) => /^border(-(top|bottom)-(left|right))?-radius$/.test(d.property))
        .filter((d) => {
            const value = d.value.replace(/\s*!important$/, "");
            return !value.split(/\s+/).every((part) => RADIUS_OK.test(part));
        });
}

// ── Rule 4: undefined custom properties ─────────────────────────────────────

export function definedVars(source, kind) {
    const names = new Set();
    if (kind === "style") {
        for (const m of source.matchAll(/(--[\w-]+)\s*:/g)) names.add(m[1]);
    } else {
        // Set from script: style keys, setProperty calls, string constants.
        for (const m of source.matchAll(/["'`](--[\w-]+)["'`]/g)) names.add(m[1]);
    }
    return names;
}

/**
 * Drop block comments and whole-line `//` comments from TS/TSX. Not
 * quote-aware on purpose: an apostrophe in JSX text would throw a
 * quote-tracking stripper off, and a doc comment that mentions
 * `var(--x, …)` must not count as a use.
 */
export function stripScriptComments(source) {
    return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
}

export function usedVars(source) {
    const names = new Set();
    for (const m of source.matchAll(/var\(\s*(--[\w-]+)/g)) names.add(m[1]);
    return names;
}

// ── Collect ─────────────────────────────────────────────────────────────────

export function collect(files) {
    const buttons = {};
    const fills = new Set();
    const radius = {};
    const defined = new Set();
    const used = new Set();

    for (const { path, source } of files) {
        const isStyle = /\.(s?css)$/.test(path);
        const isScript = /\.(tsx?)$/.test(path);
        if (!isStyle && !isScript) continue;
        if (isTest(path)) continue;

        for (const name of definedVars(source, isStyle ? "style" : "script")) defined.add(name);
        for (const name of usedVars(isStyle ? stripComments(source) : stripScriptComments(source))) used.add(name);

        if (path.endsWith(".tsx") && !path.startsWith(UI_DIR)) {
            const n = countButtons(source);
            if (n > 0) buttons[path] = n;
        }
        if (isStyle) {
            for (const selector of solidFills(source)) fills.add(`${path} :: ${selector}`);
            const n = badRadii(source).length;
            if (n > 0) radius[path] = n;
        }
    }

    const undefinedVars = [...used].filter((name) => !defined.has(name) && !name.startsWith("--tw-"));
    return {
        buttons: sortObject(buttons),
        solidFills: [...fills].sort(),
        radius: sortObject(radius),
        undefinedVars: undefinedVars.sort(),
    };
}

function sortObject(obj) {
    return Object.fromEntries(Object.entries(obj).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
}

// ── Compare ─────────────────────────────────────────────────────────────────

/** Problems with `current` against `baseline`, as human-readable lines. */
export function compare(current, baseline) {
    const grew = [];
    const shrank = [];

    for (const key of ["buttons", "radius"]) {
        const files = new Set([...Object.keys(current[key]), ...Object.keys(baseline[key] ?? {})]);
        for (const file of [...files].sort()) {
            const now = current[key][file] ?? 0;
            const was = baseline[key]?.[file] ?? 0;
            if (now > was) grew.push(`${key}: ${file} has ${now}, baseline ${was}`);
            else if (now < was) shrank.push(`${key}: ${file} has ${now}, baseline ${was}`);
        }
    }
    for (const key of ["solidFills", "undefinedVars"]) {
        const was = new Set(baseline[key] ?? []);
        const now = new Set(current[key]);
        for (const item of now) if (!was.has(item)) grew.push(`${key}: new ${item}`);
        for (const item of was) if (!now.has(item)) shrank.push(`${key}: gone ${item}`);
    }
    return { grew, shrank };
}

// ── Main ────────────────────────────────────────────────────────────────────

const isMain = process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/\\/g, "/").split("/").pop());

if (isMain) {
    const files = walk(FRONTEND).map((p) => ({ path: rel(p), source: readFileSync(p, "utf8") }));
    const current = collect(files);

    if (process.argv.includes("--update")) {
        writeFileSync(BASELINE_PATH, JSON.stringify(current, null, 4) + "\n");
        const total = (o) => Object.values(o).reduce((a, b) => a + b, 0);
        console.log(
            `Baseline written: ${total(current.buttons)} hand-rolled buttons, ${current.solidFills.length} solid fills, ` +
                `${total(current.radius)} off-token radii, ${current.undefinedVars.length} undefined variables.`
        );
        process.exit(0);
    }

    const baseline = JSON.parse(readFileSync(BASELINE_PATH, "utf8"));
    const { grew, shrank } = compare(current, baseline);

    if (grew.length > 0) {
        console.log("ERROR: new UI that bypasses the line-style component set");
        console.log("(docs/specs/SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md):");
        console.log();
        for (const line of grew) console.log(`  ${line}`);
        console.log();
        console.log("Use Button / IconButton / Tabs / SegmentedControl / Field / inputs from");
        console.log("frontend/app/element/ui/, a line instead of a solid fill, a --radius-* token,");
        console.log("and only custom properties that are defined.");
    }
    if (shrank.length > 0) {
        console.log(grew.length > 0 ? "\nAlso, the baseline is out of date:" : "The baseline is out of date (good news: the count went down):");
        for (const line of shrank) console.log(`  ${line}`);
        console.log("\nRun `node scripts/check-ui-primitives.mjs --update` and commit scripts/ui-primitives-baseline.json.");
    }
    if (grew.length > 0 || shrank.length > 0) process.exit(1);
    console.log("OK: UI primitives at baseline.");
}
