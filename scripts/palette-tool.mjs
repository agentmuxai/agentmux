// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The shared palette's method, as a tool: where the hues of schema/color-palette.json come from, and a check
// that the file still follows it (docs/specs/SPEC_SHARED_PALETTE_12_HUES_BLACK_WHITE_2026_11_16.md §2.1, which
// keeps the method of SPEC_SHARED_PERCEPTUAL_PALETTE_16_2026_11_01.md §4.2).
//
//   node scripts/palette-tool.mjs            the slots, their distances, and the colour-blindness table
//   node scripts/palette-tool.mjs --check    exit 1 when the palette file no longer matches the method
//   node scripts/palette-tool.mjs --count 10 the hues the method gives for another count (a retune)
//
// The method: take each theme's identity tone as the app draws it, walk the hue circle in 0.1 degree steps
// summing CIEDE2000, average the two themes' normalised lengths, cut the circle into equal parts, slide the
// start across one slot width keeping the placement whose worst pair under simulated colour blindness is
// best, and round to whole degrees. A slot's stored hex is the swatch tone at its hue.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// ── colour math: the app's own (frontend/app/theme/oklch.ts), ported as is ──
const srgbToLinear = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const linearToSrgb = (c) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);
const oklabToLinearRgb = ([L, a, b]) => {
    const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
    const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
    const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
    return [4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s, -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s];
};
const linearRgbToOklab = ([r, g, b]) => {
    const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
    const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
    const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
    return [0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s, 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s, 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s];
};
const inGamut = (rgb) => rgb.every((c) => c >= -1e-4 && c <= 1 + 1e-4);
/** OKLCH to linear sRGB, chroma lowered (L and h kept) until it fits; also the chroma used. */
export function oklchToLinear(l, c, h) {
    const rad = (h * Math.PI) / 180;
    const at = (chroma) => oklabToLinearRgb([l, chroma * Math.cos(rad), chroma * Math.sin(rad)]);
    let lin = at(c);
    let used = c;
    if (!inGamut(lin)) {
        let lo = 0, hi = c;
        for (let i = 0; i < 24; i++) {
            const mid = (lo + hi) / 2;
            if (inGamut(at(mid))) lo = mid;
            else hi = mid;
        }
        lin = at(lo);
        used = lo;
    }
    return { lin: lin.map((x) => Math.min(1, Math.max(0, x))), chroma: used };
}
const hex2 = (x) => Math.round(Math.min(1, Math.max(0, linearToSrgb(x))) * 255).toString(16).padStart(2, "0");
export const linToHex = (lin) => `#${hex2(lin[0])}${hex2(lin[1])}${hex2(lin[2])}`;
export const oklchToHex = (l, c, h) => linToHex(oklchToLinear(l, c, h).lin);
export const hexToLin = (hex) => {
    const n = parseInt(hex.slice(1), 16);
    return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255].map(srgbToLinear);
};
export const oklchOfHex = (hex) => {
    const [l, a, b] = linearRgbToOklab(hexToLin(hex));
    const h = (Math.atan2(b, a) * 180) / Math.PI;
    return { l, c: Math.hypot(a, b), h: h < 0 ? h + 360 : h };
};

// ── CIELAB and CIEDE2000 ──
const linToLab = ([r, g, b]) => {
    const X = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047;
    const Y = 0.2126729 * r + 0.7151522 * g + 0.072175 * b;
    const Z = (0.0193339 * r + 0.119192 * g + 0.9503041 * b) / 1.08883;
    const f = (t) => (t > 216 / 24389 ? Math.cbrt(t) : (24389 / 27 * t + 16) / 116);
    const [fx, fy, fz] = [f(X), f(Y), f(Z)];
    return [116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)];
};
export function de2000(labA, labB) {
    const [L1, a1, b1] = labA, [L2, a2, b2] = labB;
    const rad = Math.PI / 180;
    const C1 = Math.hypot(a1, b1), C2 = Math.hypot(a2, b2);
    const Cb = (C1 + C2) / 2;
    const G = 0.5 * (1 - Math.sqrt(Cb ** 7 / (Cb ** 7 + 25 ** 7)));
    const a1p = (1 + G) * a1, a2p = (1 + G) * a2;
    const C1p = Math.hypot(a1p, b1), C2p = Math.hypot(a2p, b2);
    const hp = (b, a) => (a === 0 && b === 0 ? 0 : ((Math.atan2(b, a) / rad) + 360) % 360);
    const h1p = hp(b1, a1p), h2p = hp(b2, a2p);
    const dLp = L2 - L1, dCp = C2p - C1p;
    let dhp = 0;
    if (C1p * C2p !== 0) {
        dhp = h2p - h1p;
        if (dhp > 180) dhp -= 360;
        else if (dhp < -180) dhp += 360;
    }
    const dHp = 2 * Math.sqrt(C1p * C2p) * Math.sin((dhp / 2) * rad);
    const Lbp = (L1 + L2) / 2, Cbp = (C1p + C2p) / 2;
    let hbp = h1p + h2p;
    if (C1p * C2p !== 0) {
        if (Math.abs(h1p - h2p) <= 180) hbp = (h1p + h2p) / 2;
        else hbp = h1p + h2p < 360 ? (h1p + h2p + 360) / 2 : (h1p + h2p - 360) / 2;
    }
    const T = 1 - 0.17 * Math.cos((hbp - 30) * rad) + 0.24 * Math.cos(2 * hbp * rad) + 0.32 * Math.cos((3 * hbp + 6) * rad) - 0.2 * Math.cos((4 * hbp - 63) * rad);
    const dTheta = 30 * Math.exp(-(((hbp - 275) / 25) ** 2));
    const Rc = 2 * Math.sqrt(Cbp ** 7 / (Cbp ** 7 + 25 ** 7));
    const Sl = 1 + (0.015 * (Lbp - 50) ** 2) / Math.sqrt(20 + (Lbp - 50) ** 2);
    const Sc = 1 + 0.045 * Cbp, Sh = 1 + 0.015 * Cbp * T;
    const Rt = -Math.sin(2 * dTheta * rad) * Rc;
    return Math.sqrt((dLp / Sl) ** 2 + (dCp / Sc) ** 2 + (dHp / Sh) ** 2 + Rt * (dCp / Sc) * (dHp / Sh));
}
export const dE = (linA, linB) => de2000(linToLab(linA), linToLab(linB));

// ── colour-blindness: Machado, Oliveira and Fernandes 2009, severity 1.0, on linear RGB ──
export const CVD = {
    protan: [[0.152286, 1.052583, -0.204868], [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998]],
    deutan: [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.01182, 0.04294, 0.968881]],
    tritan: [[1.255528, -0.076749, -0.178779], [-0.078411, 0.930809, 0.147602], [0.004733, 0.691367, 0.3039]],
};
export const simulate = (lin, kind) => CVD[kind].map((row) => Math.min(1, Math.max(0, row[0] * lin[0] + row[1] * lin[1] + row[2] * lin[2])));

// ── the tones of a hue slot: a copy of PANE_COLOR_TOKENS (frontend/app/block/pane-color-scheme.ts), which
//    palette-tool.test.mjs holds to the original ──
export const TOKENS = {
    dark: { identity: { l: 0.75, c: 0.15 }, border: { l: 0.45, c: 0.09 }, pill: { l: 0.33, c: 0.06 }, pillActive: { l: 0.39, c: 0.09 }, headerTint: { l: 0.27, c: 0.035 }, widgetTint: { l: 0.82, c: 0.07 } },
    light: { identity: { l: 0.52, c: 0.17 }, border: { l: 0.8, c: 0.07 }, pill: { l: 0.91, c: 0.04 }, pillActive: { l: 0.86, c: 0.07 }, headerTint: { l: 0.95, c: 0.025 }, widgetTint: { l: 0.45, c: 0.09 } },
};
export const SWATCH = { l: 0.7, c: 0.14 };
export const tone = (theme, role, hue) => oklchToLinear(TOKENS[theme][role].l, TOKENS[theme][role].c, hue).lin;
/** As the app draws it: through the 8-bit hex. */
export const drawn = (theme, role, hue) => hexToLin(oklchToHex(TOKENS[theme][role].l, TOKENS[theme][role].c, hue));

// ── the placement ──
const STEP = 0.1;
const N_STEPS = Math.round(360 / STEP);
function cumulative(theme) {
    const cum = new Float64Array(N_STEPS + 1);
    let prev = tone(theme, "identity", 0);
    for (let i = 1; i <= N_STEPS; i++) {
        const cur = tone(theme, "identity", (i * STEP) % 360);
        cum[i] = cum[i - 1] + dE(prev, cur);
        prev = cur;
    }
    return cum;
}
export const RING = { dark: cumulative("dark"), light: cumulative("light") };
const total = { dark: RING.dark[N_STEPS], light: RING.light[N_STEPS] };
// The averaged, normalised cumulative length at each 0.1 degree: 0 at hue 0, 1 at hue 360.
const AVG = Float64Array.from({ length: N_STEPS + 1 }, (_, i) => (RING.dark[i] / total.dark + RING.light[i] / total.light) / 2);
/** The hue at which the averaged length reaches `t` (0 to 1, wrapping). */
function hueAt(t) {
    t = ((t % 1) + 1) % 1;
    let lo = 0, hi = N_STEPS;
    while (hi - lo > 1) {
        const mid = (lo + hi) >> 1;
        if (AVG[mid] <= t) lo = mid;
        else hi = mid;
    }
    const f = (t - AVG[lo]) / (AVG[hi] - AVG[lo] || 1);
    return (lo + f) * STEP;
}
export const placement = (n, start) => Array.from({ length: n }, (_, k) => hueAt(start + k / n)).sort((a, b) => a - b);

/** Every pair's distance at the identity tone, for normal vision and each simulated deficiency. */
export function measure(hues, { role = "identity" } = {}) {
    const out = {};
    for (const theme of ["dark", "light"]) {
        const cols = hues.map((h) => drawn(theme, role, h));
        const n = hues.length;
        const neigh = cols.map((c, i) => dE(c, cols[(i + 1) % n]));
        const res = { neighbourMin: Math.min(...neigh), neighbourMax: Math.max(...neigh), neighbours: neigh, cvd: {} };
        let allMin = Infinity;
        for (let i = 0; i < n; i++) for (let j = i + 1; j < n; j++) allMin = Math.min(allMin, dE(cols[i], cols[j]));
        res.allPairsMin = allMin;
        for (const kind of Object.keys(CVD)) {
            const sim = cols.map((c) => simulate(c, kind));
            let under5 = 0, worst = Infinity, worstPair = null, pairs = 0;
            for (let i = 0; i < n; i++)
                for (let j = i + 1; j < n; j++) {
                    const d = dE(sim[i], sim[j]);
                    pairs++;
                    if (d < 5) under5++;
                    if (d < worst) (worst = d), (worstPair = [i, j]);
                }
            const nmin = Math.min(...sim.map((c, i) => dE(c, sim[(i + 1) % n])));
            res.cvd[kind] = { under5, pairs, worst, worstPair, neighbourMin: nmin };
        }
        out[theme] = res;
    }
    return out;
}
const worstCvd = (m) => Math.min(...["dark", "light"].flatMap((t) => Object.values(m[t].cvd).map((c) => c.worst)));
const under5Total = (m) => ["dark", "light"].reduce((a, t) => a + Object.values(m[t].cvd).reduce((x, c) => x + c.under5, 0), 0);

/** Slide the start across one slot width; keep the placement whose worst colour-blind pair is best. */
export function best(n, { slides = 90 } = {}) {
    let top = null;
    for (let s = 0; s < slides; s++) {
        const start = s / slides / n;
        const hues = placement(n, start).map((h) => Math.round(h) % 360).sort((a, b) => a - b);
        const m = measure(hues);
        const score = worstCvd(m);
        const cand = { start, hues, m, score, under5: under5Total(m) };
        if (!top || cand.score > top.score + 1e-9 || (Math.abs(cand.score - top.score) < 1e-9 && cand.under5 < top.under5)) top = cand;
    }
    return top;
}

const f1 = (x) => x.toFixed(1);

/** The palette file's slots: the hue slots in file order, and the neutral ones. */
export function readPalette(file = new URL("../schema/color-palette.json", import.meta.url)) {
    const palette = JSON.parse(readFileSync(file, "utf8"));
    return { palette, hues: palette.slots.filter((s) => s.neutral === undefined), neutrals: palette.slots.filter((s) => s.neutral !== undefined) };
}

/** What in the palette file no longer follows the method: an empty list when it all does. */
export function problems({ palette, hues, neutrals } = readPalette()) {
    const out = [];
    const want = best(hues.length).hues;
    const have = hues.map((s) => s.hue);
    if (want.join() !== have.join()) out.push(`hues are ${have.join(", ")}; the method gives ${want.join(", ")}`);
    for (const s of hues) {
        const hex = oklchToHex(palette.swatch.l, palette.swatch.c, s.hue);
        if (s.hex !== hex) out.push(`${s.id}: hex is ${s.hex}; the swatch tone at ${s.hue} degrees is ${hex}`);
    }
    const NEUTRAL_HEX = { black: "#000000", white: "#ffffff" };
    for (const s of neutrals) {
        if (NEUTRAL_HEX[s.neutral] !== s.hex) out.push(`${s.id}: a ${s.neutral} neutral is ${NEUTRAL_HEX[s.neutral] ?? "not a neutral"}, not ${s.hex}`);
    }
    if (palette.swatch.l !== SWATCH.l || palette.swatch.c !== SWATCH.c) out.push("the swatch tone differs from this tool's");
    return out;
}

function printSlots({ hues, neutrals }) {
    console.log("slot      hue  stored    dark      light");
    for (const s of hues) {
        const [dark, light] = ["dark", "light"].map((t) => oklchToHex(TOKENS[t].identity.l, TOKENS[t].identity.c, s.hue));
        console.log(`${s.id.padEnd(9)} ${String(s.hue).padStart(3)}  ${s.hex}  ${dark}  ${light}`);
    }
    for (const s of neutrals) console.log(`${s.id.padEnd(9)}   -  ${s.hex}  (its tones are in pane-color-scheme.ts)`);
}

function printMeasure(label, hues) {
    console.log(`
${label}: ${hues.join(", ")}`);
    for (const role of ["identity", "border", "pill"]) {
        const m = measure(hues, { role });
        console.log(`  neighbours, ${role} tone: ${f1(m.dark.neighbourMin)} to ${f1(m.dark.neighbourMax)} dark, ${f1(m.light.neighbourMin)} to ${f1(m.light.neighbourMax)} light`);
    }
    const m = measure(hues);
    for (const kind of Object.keys(CVD)) {
        const [d, l] = [m.dark.cvd[kind], m.light.cvd[kind]];
        console.log(`  ${kind}: pairs closer than 5: ${d.under5} of ${d.pairs} dark, ${l.under5} light; worst ${Math.min(d.worst, l.worst).toFixed(1)}`);
    }
}

function main(argv) {
    const read = readPalette();
    if (argv.includes("--check")) {
        const found = problems(read);
        for (const p of found) console.error(`palette-tool: ${p}`);
        console.log(found.length ? `palette-tool: ${found.length} problem(s) in schema/color-palette.json` : `palette-tool: ok. ${read.hues.length} hues follow the method, ${read.neutrals.length} neutrals.`);
        return found.length ? 1 : 0;
    }
    const at = argv.indexOf("--count");
    if (at >= 0) {
        const n = Number(argv[at + 1]);
        if (!Number.isInteger(n) || n < 3 || n > 48) {
            console.error("palette-tool: --count takes a whole number from 3 to 48");
            return 2;
        }
        printMeasure(`${n} hues by the method`, best(n).hues);
        return 0;
    }
    console.log(`ring length: dark ${f1(RING.dark[N_STEPS])}, light ${f1(RING.light[N_STEPS])}`);
    printSlots(read);
    printMeasure(`the ${read.hues.length} hues`, read.hues.map((s) => s.hue));
    return 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) process.exit(main(process.argv.slice(2)));
