// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-ui-primitives.mjs — CI ratchet for the line-style component set.
//
// docs/specs/SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §7
//
// THE RULES. Controls come from frontend/app/element/ui/ and follow its line
// style. Eight counts may not go up compared with the base branch (the
// merge-base with origin/main, or $GITHUB_BASE_REF):
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
// Four more hold the shared scales (SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_
// 2026_10_15.md §6.6, item 20), all per file, in SCSS/CSS:
//
//   5. durations    raw times (`120ms`, `0.5s`) in transition and animation
//                   declarations, where a `--duration-*` token belongs
//   6. fontSizes    `font-size` given as a raw px value, not a `--text-*` token
//   7. fallbacks    `var(--x, …)` where `--x` is set on `:root` itself, outside
//                   any at-rule: the fallback can never apply, and a wrong one
//                   misleads (`var(--radius-sm, 4px)` renders 0)
//   8. zIndex       `z-index` given as a raw number, not a `--z-*` token
//
// A count above the base's fails: use the element/ui/ components instead. A
// count below it is reported and passes. There is no checked-in baseline:
// one used to live in scripts/ui-primitives-baseline.json, and since every
// migration PR edited it, each merge put the others into conflict. Comparing
// with the base branch gives the same ratchet with nothing to keep in sync.
//
// Why a script and not stylelint: stylelint isn't run in CI (see
// scripts/check-no-transition-all.sh), and rule 2 needs SCSS nesting resolved.
//
// Usage:
//   node scripts/check-ui-primitives.mjs   # working tree vs the base branch

import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const FRONTEND = join(REPO_ROOT, "frontend");
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
    const stack = []; // { selector, skip, conditional }
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
            const parent = stack.length ? stack[stack.length - 1] : { selector: "", skip: false, conditional: false };
            const skip = parent.skip || /^@(mixin|function)\b/.test(head);
            const selector = head.startsWith("@") ? parent.selector : resolveSelector(parent.selector, head);
            // Inside @media, @include, @if and the like; @layer always applies.
            const conditional = parent.conditional || (head.startsWith("@") && !/^@layer\b/.test(head));
            stack.push({ selector, skip, conditional });
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
        const m = /^([a-z0-9-]+)\s*:\s*([\s\S]+)$/i.exec(decl);
        if (!m) return;
        out.push({
            selector: top.selector,
            property: m[1].toLowerCase(),
            value: m[2].trim(),
            line: bufLine,
            conditional: top.conditional,
        });
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
    // Not a name built in a template literal, `var(--conn-icon-color-${n})`.
    for (const m of source.matchAll(/var\(\s*(--[\w-]+)(?![\w-]|\$\{)/g)) names.add(m[1]);
    return names;
}

// ── Rules 5-8: the shared scales ────────────────────────────────────────────

// An optional sign, for a negative delay (`-120ms`); the look-behind still
// rejects a time inside a name such as `tab-1s`.
const RAW_TIME = /(?<![\w.-])-?\d*\.?\d+m?s\b/g;
const unimportant = (value) => value.replace(/\s*!important$/, "");

export function rawDurations(scss) {
    return declarations(scss)
        .filter((d) => /^(transition|animation)(-duration|-delay)?$/.test(d.property))
        .reduce((n, d) => n + (d.value.match(RAW_TIME) ?? []).length, 0);
}

export function rawFontSizes(scss) {
    return declarations(scss).filter((d) => d.property === "font-size" && /^\d*\.?\d+px$/.test(unimportant(d.value)));
}

export function rawZIndexes(scss) {
    return declarations(scss).filter((d) => d.property === "z-index" && /^-?\d+$/.test(unimportant(d.value)));
}

/** Custom properties set on `:root` itself (alone or in a list), outside any at-rule: defined in every element. */
export function rootTokens(scss) {
    const onRoot = (selector) => selector.split(",").some((s) => s.trim() === ":root");
    return declarations(scss)
        .filter((d) => onRoot(d.selector) && !d.conditional && d.property.startsWith("--"))
        .map((d) => d.property);
}

/** `var(--x, …)` uses whose `--x` is one of `tokens`, so the fallback never applies. */
export function deadFallbacks(scss, tokens) {
    let n = 0;
    for (const m of stripComments(scss).matchAll(/var\(\s*(--[\w-]+)\s*,/g)) if (tokens.has(m[1])) n++;
    return n;
}

// ── Collect ─────────────────────────────────────────────────────────────────

export function collect(files) {
    const buttons = {};
    const fills = new Set();
    const radius = {};
    const defined = new Set();
    const used = new Set();
    const durations = {};
    const fontSizes = {};
    const zIndex = {};
    const styles = [];

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
            styles.push({ path, source });
            countInto(durations, path, rawDurations(source));
            countInto(fontSizes, path, rawFontSizes(source).length);
            countInto(zIndex, path, rawZIndexes(source).length);
        }
    }

    const tokens = new Set(styles.flatMap(({ source }) => rootTokens(source)));
    const fallbacks = {};
    for (const { path, source } of styles) countInto(fallbacks, path, deadFallbacks(source, tokens));

    // `--tw-*` are Tailwind's; `--fa-*` are Font Awesome's optional hooks
    // (public/fontawesome/), always read with FA's own fallback; `--shiki-*`
    // are set inline on each highlighted run by Shiki, the `-font-style` and
    // `-font-weight` ones only when the theme styles that token.
    const undefinedVars = [...used].filter((name) => !defined.has(name) && !/^--(tw|fa|shiki)-/.test(name));
    return {
        buttons: sortObject(buttons),
        solidFills: [...fills].sort(),
        radius: sortObject(radius),
        undefinedVars: undefinedVars.sort(),
        durations: sortObject(durations),
        fontSizes: sortObject(fontSizes),
        fallbacks: sortObject(fallbacks),
        zIndex: sortObject(zIndex),
    };
}

function countInto(counts, path, n) {
    if (n > 0) counts[path] = n;
}

function sortObject(obj) {
    return Object.fromEntries(Object.entries(obj).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
}

// ── Compare ─────────────────────────────────────────────────────────────────

const PER_FILE = ["buttons", "radius", "durations", "fontSizes", "fallbacks", "zIndex"];

/** How `current` differs from `baseline` (the base branch), as human-readable lines. */
export function compare(current, baseline) {
    const grew = [];
    const shrank = [];

    for (const key of PER_FILE) {
        const files = new Set([...Object.keys(current[key] ?? {}), ...Object.keys(baseline[key] ?? {})]);
        for (const file of [...files].sort()) {
            const now = current[key]?.[file] ?? 0;
            const was = baseline[key]?.[file] ?? 0;
            if (now > was) grew.push(`${key}: ${file} has ${now}, base ${was}`);
            else if (now < was) shrank.push(`${key}: ${file} has ${now}, base ${was}`);
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

// ── The base branch ─────────────────────────────────────────────────────────

function git(args, input) {
    return spawnSync("git", args, { cwd: REPO_ROOT, input, maxBuffer: 1 << 30 });
}

/** The commit to compare against, or null when it can't be resolved (shallow clone, no remote). */
function baseCommit() {
    const branch = process.env.GITHUB_BASE_REF || "main";
    const rev = (ref) => {
        const r = git(["rev-parse", "--verify", "--quiet", ref]);
        return r.status === 0 ? r.stdout.toString().trim() : null;
    };
    // Set by ci-pr.yml on a push: the commit the branch pointed to before it.
    const before = process.env.UI_RATCHET_BEFORE?.trim();
    const pushedFrom = before && !/^0+$/.test(before) ? rev(`${before}^{commit}`) : null;
    for (const ref of [`origin/${branch}`, branch]) {
        const mb = git(["merge-base", "HEAD", ref]);
        if (mb.status === 0) return pickBase(mb.stdout.toString().trim(), rev("HEAD"), pushedFrom ?? rev("HEAD^"));
    }
    return null;
}

/**
 * The commit to compare against, given the merge-base with the base branch.
 * On a pull request that is where the branch left main. On a push to main
 * itself the merge-base is HEAD, which would compare the tree with itself and
 * never fail, so `previous` is used instead: the push event's `before` commit,
 * which covers every commit in the push, or failing that HEAD's first parent.
 */
export function pickBase(mergeBase, head, previous) {
    if (mergeBase !== head) return mergeBase;
    return previous;
}

/** Parse `git cat-file --batch` output into the contents of each blob, in order. */
export function parseCatFileBatch(buf) {
    const out = [];
    let i = 0;
    while (i < buf.length) {
        const nl = buf.indexOf(10, i);
        const header = buf.subarray(i, nl).toString();
        i = nl + 1;
        const m = /^\S+ (\S+) (\d+)$/.exec(header);
        if (!m) {
            out.push(null); // "<name> missing"
            continue;
        }
        const size = Number(m[2]);
        out.push(buf.subarray(i, i + size).toString("utf8"));
        i += size + 1; // contents, then a newline
    }
    return out;
}

/** The frontend's style and script files as they are at `ref`. */
function filesAtRef(ref) {
    const list = git(["ls-tree", "-r", "--name-only", ref, "--", "frontend"]);
    const paths = list.stdout
        .toString()
        .split("\n")
        .filter((p) => /\.(s?css|tsx?)$/.test(p));
    const batch = git(["cat-file", "--batch"], paths.map((p) => `${ref}:${p}`).join("\n") + "\n");
    const contents = parseCatFileBatch(batch.stdout);
    return paths.map((path, i) => ({ path, source: contents[i] ?? "" }));
}

// ── Main ────────────────────────────────────────────────────────────────────

if (isMain(import.meta.url)) {
    const base = baseCommit();
    if (!base) {
        console.log("check-ui-primitives: cannot resolve the base branch (shallow clone?) — skipping.");
        process.exit(0);
    }
    const current = collect(walk(FRONTEND).map((p) => ({ path: rel(p), source: readFileSync(p, "utf8") })));
    const before = collect(filesAtRef(base));
    const { grew, shrank } = compare(current, before);

    if (grew.length > 0) {
        console.log("ERROR: new UI that bypasses the line-style component set");
        console.log("(docs/specs/SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md):");
        console.log();
        for (const line of grew) console.log(`  ${line}`);
        console.log();
        console.log("Use Button / IconButton / Tabs / SegmentedControl / Field / inputs from");
        console.log("frontend/app/element/ui/, a line instead of a solid fill, a --radius-* token,");
        console.log("only custom properties that are defined, --duration-*, --text-* and --z-* tokens");
        console.log("instead of raw values, and no fallback on a token :root always sets.");
        process.exit(1);
    }
    const total = (o) => Object.values(o).reduce((a, b) => a + b, 0);
    console.log(
        `OK: no new hand-rolled controls vs ${base.slice(0, 9)}. Now ${total(current.buttons)} hand-rolled buttons, ` +
            `${current.solidFills.length} solid fills, ${total(current.radius)} off-token radii, ` +
            `${current.undefinedVars.length} undefined variables, ${total(current.durations)} raw durations, ` +
            `${total(current.fontSizes)} raw px font sizes, ${total(current.fallbacks)} fallbacks on always-defined ` +
            `tokens, ${total(current.zIndex)} raw z-indexes.`
    );
    if (shrank.length > 0) {
        console.log(`Down from the base branch (${shrank.length}):`);
        for (const line of shrank) console.log(`  ${line}`);
    }
}
