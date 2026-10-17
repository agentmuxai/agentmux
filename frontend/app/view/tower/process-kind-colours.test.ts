// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The five process-kind colours (`--process-kind-*`, theme.scss) stay apart
 * from each other and readable on the surface, in every theme
 * (SPEC_TOWER_THEME_SURFACE_AND_COLOR_2026_10_17.md §4). A terminal palette
 * looks like twelve colours but some themes repeat them (Monokai's blue and
 * cyan are one hex), so each theme is measured, not trusted.
 *
 * - Apart: every pair at least 9 apart in OKLab (×100); about 2 is barely
 *   visible, and an icon needs more to be told apart at a glance.
 * - Readable: at least 3:1 against `--main-bg-color`, WCAG's floor for a
 *   non-text mark (1.4.11).
 */

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const appDir = join(__dirname, "..", "..");
const KINDS = ["agents", "agentmux", "shells", "builds", "runtimes"] as const;
const MIN_APART = 9;
const MIN_CONTRAST = 3;

type Rgb = [number, number, number];
type Vars = Map<string, string>;

/** Every `--name: value;` in `css`, the first of each name (a theme's own
 *  block comes first in its file). */
function declarations(css: string): Vars {
    const out: Vars = new Map();
    for (const m of css.matchAll(/--([a-z0-9-]+)\s*:\s*([^;]+);/g)) {
        if (!out.has(m[1])) out.set(m[1], m[2].trim());
    }
    return out;
}

/** The body of the first rule whose selector is exactly `selector`. */
function block(css: string, selector: string): string {
    const at = css.indexOf(`${selector} {`);
    if (at < 0) return "";
    let depth = 0;
    for (let i = css.indexOf("{", at); i < css.length; i++) {
        if (css[i] === "{") depth++;
        else if (css[i] === "}" && --depth === 0) return css.slice(css.indexOf("{", at) + 1, i);
    }
    return "";
}

const lin = (c: number) => {
    const v = c / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
};
const unlin = (c: number) => {
    const v = Math.min(1, Math.max(0, c));
    return 255 * (v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055);
};

function toOklab([r, g, b]: Rgb): Rgb {
    const [R, G, B] = [lin(r), lin(g), lin(b)];
    const l = Math.cbrt(0.4122214708 * R + 0.5363325363 * G + 0.0514459929 * B);
    const m = Math.cbrt(0.2119034982 * R + 0.6806995451 * G + 0.1073969566 * B);
    const s = Math.cbrt(0.0883024619 * R + 0.2817188376 * G + 0.6299787005 * B);
    return [
        0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
    ];
}

function fromOklab([L, a, b]: Rgb): Rgb {
    const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
    const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
    const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
    return [
        unlin(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
        unlin(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
        unlin(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
    ];
}

/** A colour as the browser would resolve it from `vars`: a hex, an `rgb()` or
 *  `rgba()` (alpha ignored: the window is opaque by default), a `var()`, or a
 *  `color-mix(in oklab, A p%, B)`. */
function resolve(value: string, vars: Vars, depth = 0): Rgb {
    if (depth > 10) throw new Error(`var() loop at ${value}`);
    const v = value.trim();
    const ref = /^var\(--([a-z0-9-]+)\)$/.exec(v);
    if (ref) {
        const target = vars.get(ref[1]);
        if (!target) throw new Error(`--${ref[1]} is not defined`);
        return resolve(target, vars, depth + 1);
    }
    const mix = /^color-mix\(in oklab,\s*(.+?)\s+(\d+(?:\.\d+)?)%,\s*(.+)\)$/.exec(v);
    if (mix) {
        const share = Number(mix[2]) / 100;
        const [a, b] = [toOklab(resolve(mix[1], vars, depth + 1)), toOklab(resolve(mix[3], vars, depth + 1))];
        return fromOklab([0, 1, 2].map((i) => a[i] * share + b[i] * (1 - share)) as Rgb);
    }
    const hex = /^#([0-9a-f]{6})\b/i.exec(v);
    if (hex) return [0, 2, 4].map((i) => parseInt(hex[1].slice(i, i + 2), 16)) as Rgb;
    const short = /^#([0-9a-f]{3})\b/i.exec(v);
    if (short) return [...short[1]].map((c) => parseInt(c + c, 16)) as Rgb;
    const rgb = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/.exec(v);
    if (rgb) return [Number(rgb[1]), Number(rgb[2]), Number(rgb[3])];
    throw new Error(`can't resolve ${v}`);
}

const apart = (a: Rgb, b: Rgb) => {
    const [x, y] = [toOklab(a), toOklab(b)];
    return 100 * Math.hypot(x[0] - y[0], x[1] - y[1], x[2] - y[2]);
};
const luminance = ([r, g, b]: Rgb) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
const contrast = (a: Rgb, b: Rgb) => {
    const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
    return (x + 0.05) / (y + 0.05);
};

/** Every theme's variables: theme.scss's `:root`, then the default theme's
 *  own block, or a named theme's file over the `:root` ones. */
function themes(): Map<string, Vars> {
    const themeScss = readFileSync(join(appDir, "theme.scss"), "utf8");
    const base = declarations(block(themeScss, ":root"));
    const out = new Map<string, Vars>();
    out.set("default", new Map([...base, ...declarations(block(themeScss, ":root:not([data-theme])"))]));
    const dir = join(appDir, "themes");
    for (const f of readdirSync(dir).filter((n) => n.endsWith(".scss") && n !== "index.scss")) {
        out.set(f.replace(/\.scss$/, ""), new Map([...base, ...declarations(readFileSync(join(dir, f), "utf8"))]));
    }
    return out;
}

describe("process-kind colours", () => {
    const all = themes();

    it("are measured in every theme", () => {
        expect(all.size).toBeGreaterThanOrEqual(13);
    });

    for (const [theme, vars] of all) {
        it(`${theme}: the five kinds are apart and readable`, () => {
            const bg = resolve(vars.get("main-bg-color") ?? "", vars);
            const colours = KINDS.map((k) => [k, resolve(`var(--process-kind-${k})`, vars)] as const);
            for (const [k, c] of colours) {
                expect(contrast(c, bg), `${theme}: ${k} against the surface`).toBeGreaterThanOrEqual(MIN_CONTRAST);
            }
            for (let i = 0; i < colours.length; i++) {
                for (let j = i + 1; j < colours.length; j++) {
                    const [[ka, a], [kb, b]] = [colours[i], colours[j]];
                    expect(apart(a, b), `${theme}: ${ka} and ${kb}`).toBeGreaterThanOrEqual(MIN_APART);
                }
            }
        });
    }

    it("no kind is the terminal's red, which means trouble", () => {
        for (const [theme, vars] of all) {
            for (const k of KINDS) {
                expect(vars.get(`process-kind-${k}`) ?? "", `${theme}: ${k}`).not.toMatch(/term-red\)/);
            }
        }
    });
});
