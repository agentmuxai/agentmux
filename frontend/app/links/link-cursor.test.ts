// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment node

/**
 * A link shows the link hand. The app's cursor rule (app.scss, "Cursor layer") is that the hand belongs to links and
 * link-like openers only: an `<a>`, an element with `role="link"`, a link made from text (`.entity-link`) and a
 * text-styled button that opens something (`<Button opens>`). Buttons, menus, tabs, cards and rows keep the arrow.
 *
 * This is the check that keeps it so, next to the link renderer: it fails when a link element is left without the
 * hand, whether by a missing global rule or by a component's own rule that sets another cursor on it. A name that
 * does not resolve (`--dead`), a bare name with no such file (`--plain`) and a disabled link are the places the arrow is right.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const APP = join(__dirname, "..");
const FRONTEND = join(APP, "..");

function walk(dir: string, ext: RegExp, out: string[] = []): string[] {
    for (const name of readdirSync(dir)) {
        const path = join(dir, name);
        if (statSync(path).isDirectory()) {
            if (name !== "node_modules") walk(path, ext, out);
        } else if (ext.test(name) && !/\.test\./.test(name)) {
            out.push(path);
        }
    }
    return out;
}

// ─── Stylesheets: every `cursor` declaration with the full selector it applies to ───────────────────────────────

interface CursorRule {
    file: string;
    line: number;
    selector: string;
    value: string;
}

/** Parent selectors with a child's: `&` is the parent, otherwise the child is a descendant. */
function resolve(parents: string[], child: string): string[] {
    const kids = child.split(",").map((s) => s.trim()).filter(Boolean);
    if (parents.length === 0) return kids;
    const out: string[] = [];
    for (const parent of parents) {
        for (const kid of kids) out.push(kid.includes("&") ? kid.replace(/&/g, parent) : `${parent} ${kid}`);
    }
    return out;
}

/** The `cursor: …` declarations of one stylesheet, each with the selectors of the block that holds it. */
export function cursorRules(file: string, text: string): CursorRule[] {
    const rules: CursorRule[] = [];
    const stack: string[][] = [[]];
    let pending = "";
    let line = 1;
    let pendingLine = 1;
    const flushDecl = () => {
        const decl = pending.trim();
        pending = "";
        const m = /^cursor\s*:\s*([^;]+)$/.exec(decl);
        if (!m) return;
        for (const selector of stack[stack.length - 1]) rules.push({ file, line: pendingLine, selector, value: m[1].trim() });
    };
    for (let i = 0; i < text.length; i++) {
        const c = text[i];
        if (c === "\n") line++;
        if (c === "/" && text[i + 1] === "/") {
            while (i < text.length && text[i] !== "\n") i++;
            line++;
            continue;
        }
        if (c === "/" && text[i + 1] === "*") {
            const end = text.indexOf("*/", i + 2);
            const stop = end < 0 ? text.length : end + 2;
            line += (text.slice(i, stop).match(/\n/g) ?? []).length;
            i = stop - 1;
            continue;
        }
        // An interpolation and a string are kept whole; one with no end on its line is just a character.
        if ((c === "#" && text[i + 1] === "{") || c === '"' || c === "'") {
            const eol = text.indexOf("\n", i);
            const lineEnd = eol < 0 ? text.length : eol;
            const end = c === "#" ? text.indexOf("}", i) : text.indexOf(c, i + 1);
            if (end > i && end < lineEnd) {
                pending += text.slice(i, end + 1);
                i = end;
                continue;
            }
        }
        if (c === "{") {
            const prelude = pending.trim();
            pending = "";
            const top = stack[stack.length - 1];
            // An at-rule (`@media`, `@container`, `@include x { … }`) adds no selector of its own.
            stack.push(prelude.startsWith("@") ? top : resolve(top, prelude));
        } else if (c === "}") {
            flushDecl();
            if (stack.length > 1) stack.pop();
        } else if (c === ";") {
            flushDecl();
        } else {
            if (pending.trim() === "") pendingLine = line;
            pending += c;
        }
    }
    return rules;
}

const STYLES = walk(FRONTEND, /\.s?css$/).map((file) => ({ file, text: readFileSync(file, "utf8") }));
const CURSORS = STYLES.flatMap(({ file, text }) => cursorRules(relative(FRONTEND, file), text));

/** What a selector ends in: its last compound, `.a .b:hover` gives `.b:hover`. */
function subject(selector: string): string {
    return selector.split(/[\s>+~]+/).filter(Boolean).pop() ?? "";
}

const isLinkHand = (value: string) => value === "var(--cursor-link)" || value === "pointer";
/** Selectors where the arrow is right on a link: a name that does not resolve, and a disabled one. */
const isOff = (selector: string) => /--dead|--plain|:disabled|\[disabled|aria-disabled/.test(selector);

// ─── Markup: the classes carried by link elements ───────────────────────────────────────────────────────────────

interface Tag {
    file: string;
    line: number;
    name: string;
    text: string;
}

/** Every JSX opening tag of a file, braces and strings in its attributes kept whole. */
export function tagsOf(file: string, source: string): Tag[] {
    const tags: Tag[] = [];
    for (let i = 0; i < source.length; i++) {
        if (source[i] !== "<" || !/[A-Za-z]/.test(source[i + 1] ?? "")) continue;
        let j = i + 1;
        let depth = 0;
        let quote: string | null = null;
        for (; j < source.length; j++) {
            const c = source[j];
            if (quote) {
                if (c === quote && source[j - 1] !== "\\") quote = null;
            } else if (depth === 0 && (c === '"' || c === "'")) {
                quote = c;
            } else if (c === "{") {
                depth++;
            } else if (c === "}") {
                depth--;
            } else if (c === ">" && depth === 0 && source[j - 1] !== "=") {
                break;
            }
        }
        const text = source.slice(i, j + 1);
        const name = /^<([A-Za-z0-9.]+)/.exec(text)![1];
        tags.push({ file, line: source.slice(0, i).split("\n").length, name, text });
    }
    return tags;
}

const TSX = walk(FRONTEND, /\.tsx$/).map((file) => ({ file: relative(FRONTEND, file), source: readFileSync(file, "utf8") }));
const TAGS = TSX.flatMap(({ file, source }) => tagsOf(file, source));

const isLinkTag = (t: Tag) => t.name === "a" || /\brole="link"/.test(t.text);

/** The static class names on a tag (`class="a b"`, and the strings in a template or expression). */
function classesOf(t: Tag): string[] {
    const m = /\bclass=(?:"([^"]*)"|\{([^}]*(?:\{[^}]*\}[^}]*)*)\})/.exec(t.text);
    if (!m) return [];
    const raw = m[1] ?? m[2] ?? "";
    return [...raw.matchAll(/[A-Za-z_][\w-]*/g)].map((x) => x[0]);
}

describe("the link hand", () => {
    it("sees the link elements and stylesheets it checks", () => {
        expect(TAGS.filter(isLinkTag).length).toBeGreaterThan(10);
        expect(CURSORS.length).toBeGreaterThan(100);
    });

    it("is set once for every link in app.scss: with an href, with none, and with role=link", () => {
        const app = CURSORS.filter((r) => r.file === "app/app.scss");
        const hand = (selector: string) =>
            app.some((r) => r.selector === selector && r.value === "var(--cursor-link)");
        expect(hand("a[href]")).toBe(true);
        expect(hand("a:not([href])")).toBe(true);
        expect(hand('[role="link"]')).toBe(true);
    });

    it("keeps the arrow on a disabled link, in the same place", () => {
        const app = CURSORS.filter((r) => r.file === "app/app.scss");
        expect(app.some((r) => r.selector === 'a[aria-disabled="true"]' && r.value === "var(--cursor-default)")).toBe(true);
        expect(
            app.some((r) => r.selector === '[role="link"][aria-disabled="true"]' && r.value === "var(--cursor-default)")
        ).toBe(true);
    });

    it("gives a link made from text the hand, and a name that does not resolve the arrow", () => {
        const links = CURSORS.filter((r) => r.file === "app/links/linkified.scss");
        expect(links.find((r) => r.selector === ".entity-link")?.value).toBe("var(--cursor-link)");
        // Two classes, because the `a[href]` rule is more specific than one: with one the dead link would show the hand.
        expect(links.find((r) => r.selector === ".entity-link.entity-link--dead")?.value).toBe("var(--cursor-default)");
        expect(links.find((r) => r.selector === ".entity-link.entity-link--plain")?.value).toBe("var(--cursor-default)");
    });

    it("gives a button that opens something the hand", () => {
        const rule = CURSORS.find((r) => r.file === "app/element/ui/ui.scss" && r.selector.startsWith(".ui-button.ui-button--opens"));
        expect(rule?.value).toBe("var(--cursor-link)");
        expect(rule?.selector).toContain(":not(:disabled");
    });

    it("never lets a component's own rule set another cursor on a link element", () => {
        const linkClasses = new Map<string, string>();
        for (const t of TAGS.filter(isLinkTag)) for (const c of classesOf(t)) linkClasses.set(c, `${t.file}:${t.line}`);
        linkClasses.set("entity-link", "app/links/linkified.tsx");

        const wrong = CURSORS.filter((r) => {
            if (isLinkHand(r.value) || isOff(r.selector)) return false;
            const last = subject(r.selector);
            return [...linkClasses.keys()].some((c) => new RegExp(`\\.${c}(?![\\w-])`).test(last));
        }).map((r) => `${r.file}:${r.line} ${r.selector} { cursor: ${r.value} }`);
        expect(wrong).toEqual([]);
    });

    it("never takes the hand off a link with a utility class", () => {
        const wrong = TAGS.filter(isLinkTag)
            .filter((t) => /\bcursor-(default|text|auto|not-allowed|help)\b/.test(t.text))
            .map((t) => `${t.file}:${t.line} <${t.name}>`);
        expect(wrong).toEqual([]);
    });
});

describe("the check itself", () => {
    it("reads a nested stylesheet's selectors and cursors", () => {
        const rules = cursorRules(
            "x.scss",
            `.a { cursor: pointer; &:hover { cursor: default; } .b, &--c { cursor: var(--cursor-link) } @media (x) { .d { cursor: text; } } }`
        );
        expect(rules.map((r) => `${r.selector}|${r.value}`)).toEqual([
            ".a|pointer",
            ".a:hover|default",
            ".a .b|var(--cursor-link)",
            ".a--c|var(--cursor-link)",
            ".a .d|text",
        ]);
    });

    it("reads a JSX tag whose attributes hold braces and a greater-than sign", () => {
        const tags = tagsOf("x.tsx", `const x = <a class="k" onClick={() => a > b} role="link">t</a>;`);
        expect(tags).toHaveLength(1);
        expect(isLinkTag(tags[0])).toBe(true);
        expect(classesOf(tags[0])).toEqual(["k"]);
    });
});
