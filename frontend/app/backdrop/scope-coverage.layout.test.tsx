// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Every colour a component draws inside a pane follows the pane's backdrop
 * (docs/specs/SPEC_PANE_BACKGROUND_THEMES_2026_11_15.md §9, check 3). Each component kind is rendered for real, its
 * compiled stylesheet and the real theme are loaded in a browser, and the pane's scope is set with a distinct sentinel
 * colour for every surface and ink. Every colour an element ends up with (text, fill, borders, outline, shadows) must
 * then be a sentinel, a see-through version of one, or a mix of two (the theme builds tints and edges that way). A
 * colour that is none of those did not come from the pane: it is a literal, or a token the scope does not re-point,
 * and the test names the component, the element and the property.
 *
 * jsdom cannot resolve `var()` through compiled SCSS, so this is a browser test; it skips, rather than fails, on a
 * machine with no usable browser (layout-browser.ts findBrowser).
 */
import { cleanup, render } from "@solidjs/testing-library";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import * as sass from "sass";
import type { JSX } from "solid-js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { Button, Card, CloseButton, EmptyState, Kbd, Notice, Pill, SectionHeader, SegmentedControl, StatusDot, Tag, Tabs } from "../element/ui";
import { findBrowser, measureInBrowser, NO_BROWSER_HINT } from "../view/agent/components/layout-browser";
import { deriveBuiltin } from "./builtins";
import { scopeVars, type ScopeVars } from "./scope";

const BROWSER = findBrowser();
if (!BROWSER) console.info(NO_BROWSER_HINT);

const APP = join(__dirname, "..");
const compile = (file: string) => sass.compile(join(APP, file)).css;

// ── The sentinels ───────────────────────────────────────────────────────────

/** A colour no theme token is: saturated, off the grey axis, far apart from each other. */
const SENTINELS: Record<string, string> = {
    "--surface": "#0a3d91",
    "--surface-raised": "#1a5f3e",
    "--surface-overlay": "#5c1f7a",
    "--ink-text": "#ffe14d",
    "--ink-muted": "#ff9d3b",
    "--ink-faint": "#c97aff",
    "--ink-border": "#00d5c4",
    "--ink-divider": "#8bff3b",
    "--ink-accent": "#ff3b6b",
    "--ink-error": "#ff3bf0",
    "--ink-warning": "#3b9dff",
    "--ink-success": "#d4ff00",
    "--ink-info": "#00ffa6",
    "--ink-attention": "#ff7ad9",
    "--ink-on-attention": "#2bffd6",
    "--ink-on-accent": "#d6ff2b",
    "--ink-focus": "#ff6a00",
    "--ink-hover": "#7a7aff",
    "--ink-selected": "#e63b00",
};

function sentinelScope(): ScopeVars {
    const base = scopeVars(deriveBuiltin("blue", "dark")!);
    const vars: ScopeVars = { ...base, ...SENTINELS };
    const [r, g, b] = [0xff, 0x3b, 0x6b];
    vars["--accent-color-rgb"] = `${r}, ${g}, ${b}`;
    return vars;
}

const inline = (vars: ScopeVars) =>
    Object.entries(vars)
        .map(([name, value]) => `${name}:${value}`)
        .join(";");

// ── What is rendered ────────────────────────────────────────────────────────

interface Kind {
    id: string;
    ui: () => JSX.Element;
}

const noop = () => {};
const TONES = ["neutral", "ok", "warn", "error", "accent"] as const;

const KINDS: Kind[] = [
    ...(["accent", "neutral", "danger", "quiet", "attention"] as const).map((tone) => ({
        id: `Button ${tone}`,
        ui: () => (
            <>
                <Button tone={tone}>Label</Button>
                <Button tone={tone} icon="check" pressed>
                    Pressed
                </Button>
                <Button tone={tone} disabled>
                    Disabled
                </Button>
            </>
        ),
    })),
    { id: "Button link and row", ui: () => <><Button variant="link">Link</Button><Button variant="row">Row</Button></> },
    { id: "CloseButton", ui: () => <CloseButton onClick={noop} /> },
    ...(["accent", "warning", "danger", "success"] as const).map((tone) => ({
        id: `Notice ${tone}`,
        ui: () => (
            <Notice tone={tone} title="Title" icon="circle-info" actions={<Button>Act</Button>} onDismiss={noop}>
                A message
            </Notice>
        ),
    })),
    ...TONES.map((tone) => ({ id: `Pill ${tone}`, ui: () => <Pill tone={tone} icon="check">Status</Pill> })),
    ...TONES.map((tone) => ({
        id: `Tag ${tone}`,
        ui: () => (
            <>
                <Tag tone={tone}>Chip</Tag>
                <Tag tone={tone} muted>
                    Muted
                </Tag>
                <Tag tone={tone} kind="count">
                    3
                </Tag>
            </>
        ),
    })),
    ...(["ok", "busy", "warn", "error", "idle"] as const).map((tone) => ({ id: `StatusDot ${tone}`, ui: () => <StatusDot tone={tone} label={tone} /> })),
    {
        id: "Card",
        ui: () => (
            <>
                <Card title="A card" subtitle="Second line" status={{ tone: "ok", label: "Connected" }} meta={[<span>meta</span>]} actions={<Button>Open</Button>} />
                <Card title="Selected" selected />
                <Card title="Muted" muted />
                <Card title="Needs attention" attention onSelect={noop} />
            </>
        ),
    },
    { id: "EmptyState", ui: () => <EmptyState title="Nothing here" icon="inbox" actions={<Button>Add</Button>}>Text</EmptyState> },
    { id: "Kbd", ui: () => <Kbd>Ctrl+K</Kbd> },
    { id: "SectionHeader", ui: () => <><SectionHeader title="Title" count={3} /><SectionHeader title="Group" variant="group" /></> },
    {
        id: "SegmentedControl",
        ui: () => (
            <SegmentedControl
                ariaLabel="x"
                value="a"
                onChange={noop}
                options={[
                    { value: "a", label: "One" },
                    { value: "b", label: "Two" },
                ]}
            />
        ),
    },
    {
        id: "Tabs",
        ui: () => (
            <Tabs
                idPrefix="t"
                ariaLabel="x"
                value="a"
                onChange={noop}
                items={[
                    { id: "a", label: "One" },
                    { id: "b", label: "Two" },
                ]}
            />
        ),
    },
];

/** The stylesheets of the kinds above, as the app loads them (each component imports its own). */
const SHEETS = [
    "element/ui/ui.scss",
    "element/ui/card.scss",
    "element/ui/pill.scss",
    "element/ui/tag.scss",
    "element/ui/close-button.scss",
    "element/ui/pane-parts.scss",
];

function tailwindTokens(): string {
    const setup = readFileSync(join(APP, "../tailwindsetup.css"), "utf8");
    return `:root{${[...setup.matchAll(/^\s*(--color-[\w-]+):\s*([^;]+);/gm)].map((m) => `${m[1]}:${m[2]}`).join(";")}}`;
}

function markup(k: Kind): string {
    const { container, unmount } = render(() => <>{k.ui()}</>);
    const html = container.innerHTML;
    unmount();
    return `<section data-kind="${k.id}"><div data-pane style="background:var(--main-bg-color);color:var(--main-text-color);padding:8px">${html}</div></section>`;
}

// ── Measuring ───────────────────────────────────────────────────────────────

interface Found {
    el: string;
    prop: string;
    value: string;
}

const MEASURE = `
const COLOUR = /(?:rgba?|color)\\([^)]*\\)/g;
const SIDES = ["Top", "Right", "Bottom", "Left"];
const out = {};
for (const s of document.querySelectorAll("[data-kind]")) {
  const found = [];
  const name = (el) => el.tagName.toLowerCase() + (typeof el.className === "string" && el.className ? "." + el.className.trim().split(/\\s+/).join(".") : "");
  const add = (el, prop, value) => { if (value) found.push({ el: name(el), prop, value }); };
  for (const el of s.querySelectorAll("[data-pane] *")) {
    const cs = getComputedStyle(el);
    if (cs.display === "none") continue;
    const hasText = Array.from(el.childNodes).some((n) => n.nodeType === 3 && n.textContent.trim());
    if (hasText) add(el, "color", cs.color);
    add(el, "background-color", cs.backgroundColor);
    for (const side of SIDES) {
      if (parseFloat(cs["border" + side + "Width"]) > 0 && cs["border" + side + "Style"] !== "none") add(el, "border-" + side.toLowerCase() + "-color", cs["border" + side + "Color"]);
    }
    if (parseFloat(cs.outlineWidth) > 0 && cs.outlineStyle !== "none") add(el, "outline-color", cs.outlineColor);
    for (const c of cs.boxShadow === "none" ? [] : cs.boxShadow.match(COLOUR) || []) add(el, "box-shadow", c);
  }
  out[s.dataset.kind] = found;
}
const pre = document.createElement("pre"); pre.id = "result"; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
`;

type Rgba = [number, number, number, number];

/** A computed colour in any of the forms a browser gives (`rgb(…)`, `rgba(…)`, `color(srgb …)`) as bytes and alpha. */
function parseColour(value: string): Rgba | null {
    const m = /^(rgba?|color)\((.*)\)$/.exec(value.trim());
    if (!m) return null;
    const body = m[2].replace(/^srgb\s+/, "");
    const [chan, alpha] = body.split("/").map((s) => s.trim());
    const parts = chan.split(/[\s,]+/).filter(Boolean).map(Number);
    if (parts.length < 3 || parts.some(Number.isNaN)) return null;
    const scale = m[1] === "color" ? 255 : 1;
    const a = alpha !== undefined ? parseFloat(alpha) : parts.length > 3 ? parts[3] : 1;
    return [parts[0] * scale, parts[1] * scale, parts[2] * scale, alpha !== undefined && alpha.endsWith("%") ? a / 100 : a];
}

const hexBytes = (hex: string): [number, number, number] => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as [number, number, number];
const SENTINEL_BYTES = Object.values(SENTINELS).map(hexBytes);

const dist = (a: number[], b: number[]) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);

/** The distance from `p` to the line segment `a`–`b`: a mix of two sentinels lies on it. */
function toSegment(p: number[], a: number[], b: number[]): number {
    const ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    const len2 = ab[0] ** 2 + ab[1] ** 2 + ab[2] ** 2;
    const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1] + (p[2] - a[2]) * ab[2]) / len2));
    return dist(p, [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t]);
}

/** Within the browser's rounding of a colour mixed in a colour space, not the pane's. */
const SLACK = 4;

/** Whether a colour is the pane's: a sentinel, or a mix of two (the theme's own tints and edges). */
export function followsScope(c: Rgba): boolean {
    if (c[3] < 0.01) return true;
    if (SENTINEL_BYTES.some((s) => dist(c, s) <= SLACK)) return true;
    for (let i = 0; i < SENTINEL_BYTES.length; i++) {
        for (let j = i + 1; j < SENTINEL_BYTES.length; j++) {
            if (toSegment(c, SENTINEL_BYTES[i], SENTINEL_BYTES[j]) <= SLACK) return true;
        }
    }
    return false;
}

/**
 * What a component is allowed to draw that is not the pane's, each with the reason. Empty by intent: a colour that is
 * not the pane's is a defect to fix in the component or in the scope, not a line here.
 */
const ALLOWED: { kind: RegExp; el: RegExp; prop: RegExp; why: string }[] = [];

const allowed = (kind: string, f: Found) => ALLOWED.some((a) => a.kind.test(kind) && a.el.test(f.el) && a.prop.test(f.prop));

describe("the sentinel check itself", () => {
    it("takes a sentinel, its see-through form and a mix of two, and refuses a theme grey and a white", () => {
        expect(followsScope([10, 61, 145, 1])).toBe(true);
        expect(followsScope([10, 61, 145, 0.2])).toBe(true);
        const [a, b] = [SENTINEL_BYTES[0], SENTINEL_BYTES[3]];
        expect(followsScope([(a[0] + b[0]) / 2, (a[1] + b[1]) / 2, (a[2] + b[2]) / 2, 1])).toBe(true);
        expect(followsScope([200, 200, 200, 1])).toBe(false);
        expect(followsScope([255, 255, 255, 1])).toBe(false);
        expect(followsScope([0, 0, 0, 0])).toBe(true);
    });

    it("reads each form of a computed colour", () => {
        expect(parseColour("rgb(10, 61, 145)")).toEqual([10, 61, 145, 1]);
        expect(parseColour("rgba(10, 61, 145, 0.5)")).toEqual([10, 61, 145, 0.5]);
        expect(parseColour("color(srgb 1 0 0 / 0.25)")).toEqual([255, 0, 0, 0.25]);
        expect(parseColour("rgb(10 61 145 / 50%)")).toEqual([10, 61, 145, 0.5]);
    });
});

describe.skipIf(!BROWSER)("every colour a component draws in a pane follows the pane's backdrop", () => {
    let r: Record<string, Found[]>;
    beforeAll(async () => {
        const css = [compile("theme.scss"), ":root{--window-opacity:1}", tailwindTokens(), compile("backdrop/backdrop.scss"), ...SHEETS.map(compile)].join("\n");
        const body = KINDS.map(markup).join("");
        const html = `<!doctype html><meta charset="utf-8"><style>${css}</style><body style="margin:0"><div data-backdrop style="${inline(sentinelScope())}">${body}</div><script>window.addEventListener("load",()=>{${MEASURE}})</script>`;
        r = await measureInBrowser<Record<string, Found[]>>(BROWSER!, html, { width: 900, height: 900 });
    }, 90_000);
    afterAll(() => cleanup());

    it.each(KINDS.map((k) => k.id))("%s", (id) => {
        const found = r[id] ?? [];
        expect(found.length, `${id} drew something`).toBeGreaterThan(0);
        const leaks = found
            .map((f) => ({ ...f, c: parseColour(f.value) }))
            .filter((f) => f.c && !followsScope(f.c) && !allowed(id, f))
            .map((f) => `${f.el} ${f.prop}: ${f.value}`);
        expect([...new Set(leaks)], `${id}: colours that did not come from the pane`).toEqual([]);
    });
});
