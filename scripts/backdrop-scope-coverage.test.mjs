// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The agent pane inside a backdrop (docs/specs/SPEC_PANE_BACKGROUND_THEMES_2026_11_15.md §7.6, check 3, the part that
// needs no browser). A pane's scope re-points a list of tokens (frontend/app/backdrop/mapped-tokens.json) and the
// generated `_aliases.scss` re-resolves what reads them. A token that theme.scss sets to a FIXED colour on `:root` and
// is in neither group stays window-themed inside a pane, so the agent pane must not read one unless it is named here,
// with why. A new fixed-colour token, or a new use of one, fails this until it is mapped or listed.
//
// What this cannot see (the look check does): a computed colour in a running window, a colour set from script, and a
// token set per theme in themes/*.scss rather than on theme.scss's `:root`.

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { dependentAliases, mappedNames, rootDefaults, varsIn } from "./gen-backdrop-aliases.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const AGENT_DIR = join(ROOT, "frontend/app/view/agent");
const COLOUR = /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?|oklch|oklab)\(|\bcolor-mix\(/i;

/** Tokens the agent pane reads that stay a fixed colour inside a backdrop, and why that is right. */
const KEEPS = {
    "--attention-color": "an agent needs you: an identity colour, fixed on purpose (theme.scss)",
    "--shadow-lg": "a black drop shadow reads the same on any surface",
    "--window-close-hover-bg": "the Windows close-button red, fixed by the platform",
    "--window-close-press-bg": "the Windows close-button red, fixed by the platform",
};

/** Tokens waiting on the transcript's tone-moved palette (the next PR); each leaves this list when it is mapped. */
const PENDING_TRANSCRIPT_PALETTE = [
    "--term-blue",
    "--term-bright-blue",
    "--term-bright-cyan",
    "--term-bright-green",
    "--term-bright-magenta",
    "--term-cyan",
    "--term-green",
    "--term-magenta",
    "--term-yellow",
];

const stripComments = (text) => text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^[ \t]*\/\/.*$/gm, "");

function agentSources(dir = AGENT_DIR, out = []) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const full = join(dir, entry.name);
        if (entry.isDirectory()) agentSources(full, out);
        else if (/\.(s?css|tsx?)$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name) && !entry.name.endsWith(".d.ts")) out.push(full);
    }
    return out;
}

const defaults = rootDefaults(readFileSync(join(ROOT, "frontend/app/theme.scss"), "utf8"));
const mapped = JSON.parse(readFileSync(join(ROOT, "frontend/app/backdrop/mapped-tokens.json"), "utf8"));
const mappedSet = new Set(mappedNames(mapped));
const derived = new Set(dependentAliases(defaults, mapped));

/** `:root` tokens set to a colour that no pane scope reaches: not mapped, not an alias of a mapped token. */
const fixedColour = new Set(
    [...defaults]
        .filter(([name, value]) => COLOUR.test(value) && !mappedSet.has(name) && !derived.has(name) && !varsIn(value).some((v) => mappedSet.has(v) || derived.has(v)))
        .map(([name]) => name)
);

/** Each fixed-colour token the agent pane reads, with the files that read it. */
function fixedColourUses() {
    const uses = new Map();
    for (const file of agentSources()) {
        const text = stripComments(readFileSync(file, "utf8"));
        for (const m of text.matchAll(/var\(\s*(--[\w-]+)/g)) {
            if (!fixedColour.has(m[1])) continue;
            uses.set(m[1], (uses.get(m[1]) ?? new Set()).add(file.slice(ROOT.length + 1)));
        }
    }
    return uses;
}

describe("the agent pane inside a backdrop", () => {
    const uses = fixedColourUses();
    const named = new Set([...Object.keys(KEEPS), ...PENDING_TRANSCRIPT_PALETTE]);

    it("reads no fixed-colour token that a pane scope does not reach, other than those named above", () => {
        const stray = [...uses].filter(([token]) => !named.has(token)).map(([token, files]) => `${token} (${[...files].join(", ")})`);
        expect(stray, "map the token in frontend/app/backdrop/mapped-tokens.json (and run node scripts/gen-backdrop-aliases.mjs), or list it in KEEPS with why").toEqual([]);
    });

    it("lists only tokens it still reads, so the list can't go stale", () => {
        const unused = [...named].filter((token) => !uses.has(token));
        expect(unused, "remove them from KEEPS or PENDING_TRANSCRIPT_PALETTE").toEqual([]);
    });

    it("keeps a reason for every token it keeps", () => {
        for (const [token, why] of Object.entries(KEEPS)) expect(why.length, token).toBeGreaterThan(10);
    });

    it("reaches the tokens the agent pane paints with most: the panel, modal, hover, strip and grey colours", () => {
        for (const token of ["--panel-bg-color", "--modal-bg-color", "--hover-bg-color", "--tab-strip-bg", "--block-bg-solid-color", "--grey-text-color"]) {
            expect(mappedSet.has(token), token).toBe(true);
        }
    });

    it("keeps the window's own warning colour as it is inside a pane: defined on :root, listed window-level, and not re-resolved", () => {
        expect(defaults.has("--app-warning-color")).toBe(true);
        expect(mapped.windowLevel).toContain("--app-warning-color");
        const aliases = readFileSync(join(ROOT, "frontend/app/backdrop/_aliases.scss"), "utf8");
        expect(aliases).not.toContain("--app-warning-color");
    });

    it("sees what it should: the scan finds the agent pane's files and a known fixed colour", () => {
        expect(agentSources().length).toBeGreaterThan(100);
        expect(fixedColour.has("--attention-color")).toBe(true);
        expect(fixedColour.has("--main-text-color")).toBe(false);
    });
});
