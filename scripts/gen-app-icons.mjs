#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Every raster of the AgentMux app icon, drawn from the two SVG sources in assets/icon/:
//
//   agentmux-dark.svg    the default: a dark square tile, four blocks in one blue-violet
//   agentmux-light.svg   the light-theme one: a light tile with a darker edge, blocks in a deeper blue-violet
//
// The sources are flat rectangles on a 512 grid whose edges are multiples of 32 units, i.e. whole
// pixels at 16, 32, 48 ... px. This reads the rectangles and draws them itself (exact area coverage,
// no image library), so a run on any machine writes the same bytes:
//
//   node scripts/gen-app-icons.mjs           write every file
//   node scripts/gen-app-icons.mjs --check   fail if a committed file isn't what this would write
//
// Written: Linux hicolor PNGs, the Windows .ico, the frontend's 256 px icon, the tray and notification
// PNGs, the MSIX logos (plus the unplated / light-unplated taskbar sets), the macOS 1024 px icon (the
// blocks on Apple's rounded template) and the macOS menu-bar glyph, assets/icon/web/ (the favicon
// set for the landing and docs sites), and the mark the app draws for itself: the four blocks alone, on
// no tile (the splash, the loading spinner, the empty-pane logo). See assets/icon/README.md.

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const GRID = 512;

// ---- reading the sources ------------------------------------------------------------------------

/** The `<rect>`s of an icon SVG, in paint order: `{ x, y, w, h, fill: [r, g, b] }`, on the 512 grid. */
export function readRects(svgText) {
    const rects = [];
    for (const m of svgText.matchAll(/<rect\b([^>]*)\/>/g)) {
        const attr = (name) => (m[1].match(new RegExp(`\\b${name}="([^"]*)"`)) ?? [])[1];
        const fill = attr("fill");
        if (!/^#[0-9a-fA-F]{6}$/.test(fill ?? "")) throw new Error(`gen-app-icons: a <rect> without a #rrggbb fill: ${m[0]}`);
        rects.push({
            x: Number(attr("x") ?? 0),
            y: Number(attr("y") ?? 0),
            w: Number(attr("width")),
            h: Number(attr("height")),
            fill: [1, 3, 5].map((i) => parseInt(fill.slice(i, i + 2), 16)),
        });
    }
    if (rects.length === 0) throw new Error("gen-app-icons: no <rect> in the icon SVG");
    return rects;
}

// ---- drawing ------------------------------------------------------------------------------------

/** A premultiplied RGBA float canvas. */
function canvas(w, h) {
    return { w, h, px: new Float64Array(w * h * 4) };
}

/** Source-over `rgb` at coverage `cov` (0..1) onto pixel (x, y). */
function over(c, x, y, rgb, cov) {
    if (cov <= 0 || x < 0 || y < 0 || x >= c.w || y >= c.h) return;
    const i = (y * c.w + x) * 4;
    const inv = 1 - cov;
    c.px[i] = (rgb[0] / 255) * cov + c.px[i] * inv;
    c.px[i + 1] = (rgb[1] / 255) * cov + c.px[i + 1] * inv;
    c.px[i + 2] = (rgb[2] / 255) * cov + c.px[i + 2] * inv;
    c.px[i + 3] = cov + c.px[i + 3] * inv;
}

/** Fill the rectangle (x, y, w, h), in pixels (fractions allowed), with exact area coverage. */
function fillRect(c, x, y, w, h, rgb) {
    for (let py = Math.floor(y); py < Math.ceil(y + h); py++) {
        const cy = Math.min(py + 1, y + h) - Math.max(py, y);
        for (let px = Math.floor(x); px < Math.ceil(x + w); px++) {
            over(c, px, py, rgb, (Math.min(px + 1, x + w) - Math.max(px, x)) * cy);
        }
    }
}

/** Fill a rounded rectangle (corner radius r, in pixels), 8 x 8 samples a pixel on the corners, exact elsewhere. */
function fillRoundRect(c, x, y, w, h, r, rgb) {
    const N = 8;
    const inside = (sx, sy) => {
        if (sx < x || sy < y || sx > x + w || sy > y + h) return false;
        const dx = Math.max(x + r - sx, 0, sx - (x + w - r));
        const dy = Math.max(y + r - sy, 0, sy - (y + h - r));
        return dx * dx + dy * dy <= r * r;
    };
    for (let py = Math.floor(y); py < Math.ceil(y + h); py++) {
        for (let px = Math.floor(x); px < Math.ceil(x + w); px++) {
            const corner = (px < x + r || px + 1 > x + w - r) && (py < y + r || py + 1 > y + h - r);
            let cov;
            if (corner) {
                let n = 0;
                for (let j = 0; j < N; j++) for (let i = 0; i < N; i++) n += inside(px + (i + 0.5) / N, py + (j + 0.5) / N) ? 1 : 0;
                cov = n / (N * N);
            } else {
                cov = (Math.min(px + 1, x + w) - Math.max(px, x)) * (Math.min(py + 1, y + h) - Math.max(py, y));
            }
            over(c, px, py, rgb, cov);
        }
    }
}

/** The icon's rectangles, drawn at `size` px square. */
function drawIcon(rects, size) {
    const c = canvas(size, size);
    const k = size / GRID;
    for (const r of rects) fillRect(c, r.x * k, r.y * k, r.w * k, r.h * k, r.fill);
    return c;
}

/** The icon centred on a `w` x `h` transparent canvas, as big as fits (the wide MSIX logo). */
function drawCentred(rects, w, h) {
    const side = Math.min(w, h);
    const c = canvas(w, h);
    const k = side / GRID;
    const ox = Math.round((w - side) / 2);
    const oy = Math.round((h - side) / 2);
    for (const r of rects) fillRect(c, ox + r.x * k, oy + r.y * k, r.w * k, r.h * k, r.fill);
    return c;
}

/**
 * The macOS icon: Apple's rounded template (an 824 px rounded square, corner radius 185, centred on a 1024 px
 * canvas, so the Dock's own spacing holds) in the dark tile's colour, the four blocks inside it at the same
 * proportions as the flat icon. The blocks stay square; only the tile takes the rounding macOS expects.
 */
function drawMacIcon(rects, size) {
    const c = canvas(size, size);
    const k = size / 1024;
    const tile = rects[0];
    fillRoundRect(c, 100 * k, 100 * k, 824 * k, 824 * k, 185 * k, tile.fill);
    const s = 824 / GRID; // the 512 grid scaled into the 824 tile
    for (const r of rects.slice(1)) fillRect(c, (100 + r.x * s) * k, (100 + r.y * s) * k, r.w * s * k, r.h * s * k, r.fill);
    return c;
}

/**
 * The menu-bar glyph: the four blocks alone, black on transparent (macOS colours a template image from its
 * alpha), 36 px = 18 pt at 2x. Hand-placed on whole pixels: margin 3, gap 2, bar 4 high, columns 14 wide.
 */
function drawMenubarGlyph() {
    const c = canvas(36, 36);
    const black = [0, 0, 0];
    fillRect(c, 3, 3, 30, 4, black);
    fillRect(c, 3, 9, 14, 24, black);
    fillRect(c, 19, 9, 14, 11, black);
    fillRect(c, 19, 22, 14, 11, black);
    return c;
}

// ---- encoding -----------------------------------------------------------------------------------

const CRC = (() => {
    const t = new Uint32Array(256);
    for (let n = 0; n < 256; n++) {
        let v = n;
        for (let k = 0; k < 8; k++) v = v & 1 ? 0xedb88320 ^ (v >>> 1) : v >>> 1;
        t[n] = v >>> 0;
    }
    return t;
})();
const crc32 = (buf) => {
    let v = 0xffffffff;
    for (const b of buf) v = CRC[(v ^ b) & 0xff] ^ (v >>> 8);
    return (v ^ 0xffffffff) >>> 0;
};

/** An 8-bit RGBA PNG of canvas `c`. */
export function encodePng(c) {
    const raw = Buffer.alloc((c.w * 4 + 1) * c.h);
    for (let y = 0; y < c.h; y++) {
        raw[y * (c.w * 4 + 1)] = 0;
        for (let x = 0; x < c.w; x++) {
            const i = (y * c.w + x) * 4;
            const a = c.px[i + 3];
            const o = y * (c.w * 4 + 1) + 1 + x * 4;
            // Premultiplied -> straight; a fully transparent pixel is plain black.
            for (let ch = 0; ch < 3; ch++) raw[o + ch] = a > 0 ? Math.round(Math.min(1, c.px[i + ch] / a) * 255) : 0;
            raw[o + 3] = Math.round(a * 255);
        }
    }
    const chunk = (type, data) => {
        const len = Buffer.alloc(4);
        len.writeUInt32BE(data.length);
        const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
        const crc = Buffer.alloc(4);
        crc.writeUInt32BE(crc32(body));
        return Buffer.concat([len, body, crc]);
    };
    const ihdr = Buffer.alloc(13);
    ihdr.writeUInt32BE(c.w, 0);
    ihdr.writeUInt32BE(c.h, 4);
    ihdr.set([8, 6, 0, 0, 0], 8); // 8 bit, RGBA, deflate, no filter, no interlace
    return Buffer.concat([
        Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
        chunk("IHDR", ihdr),
        chunk("IDAT", deflateSync(raw, { level: 9 })),
        chunk("IEND", Buffer.alloc(0)),
    ]);
}

/** A Windows .ico of PNG images, one per canvas. */
export function encodeIco(canvases) {
    const pngs = canvases.map(encodePng);
    const head = Buffer.alloc(6 + 16 * pngs.length);
    head.writeUInt16LE(1, 2);
    head.writeUInt16LE(pngs.length, 4);
    let offset = head.length;
    pngs.forEach((png, i) => {
        const c = canvases[i];
        const e = 6 + 16 * i;
        head[e] = c.w >= 256 ? 0 : c.w;
        head[e + 1] = c.h >= 256 ? 0 : c.h;
        head.writeUInt16LE(1, e + 4); // planes
        head.writeUInt16LE(32, e + 6); // bits
        head.writeUInt32LE(png.length, e + 8);
        head.writeUInt32LE(offset, e + 12);
        offset += png.length;
    });
    return Buffer.concat([head, ...pngs]);
}

// ---- what is written ----------------------------------------------------------------------------

/** The favicon SVG for the web: the dark icon, and the light one while the browser is in dark mode (its tab strip is dark). */
function faviconSvg(dark, light) {
    const rectsOf = (rects) =>
        rects.map((r) => `<rect x="${r.x}" y="${r.y}" width="${r.w}" height="${r.h}" fill="#${r.fill.map((v) => v.toString(16).padStart(2, "0")).join("")}"/>`).join("");
    return (
        `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" shape-rendering="crispEdges">` +
        `<style>.l{display:none}@media (prefers-color-scheme:dark){.d{display:none}.l{display:inline}}</style>` +
        `<g class="d">${rectsOf(dark)}</g><g class="l">${rectsOf(light)}</g></svg>\n`
    );
}

const hex = (rgb) => `#${rgb.map((v) => v.toString(16).padStart(2, "0")).join("")}`;

/** The rectangles of the mark inside the app: the dark icon's four blocks, without the tile. */
export const markRects = (dark) => dark.slice(1);

/** The mark as an SVG (also inlined by hand in index.html's pre-splash; a test keeps the two equal). */
export function markSvg(dark) {
    const rects = markRects(dark).map((r) => `<rect x="${r.x}" y="${r.y}" width="${r.w}" height="${r.h}" fill="${hex(r.fill)}"/>`);
    return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" shape-rendering="crispEdges">${rects.join("")}</svg>
`;
}

/** Every output: `{ path (from the repo root), bytes }`. */
export function outputs(root = ROOT) {
    const dark = readRects(readFileSync(join(root, "assets/icon/agentmux-dark.svg"), "utf8"));
    const light = readRects(readFileSync(join(root, "assets/icon/agentmux-light.svg"), "utf8"));
    const out = [];
    const png = (path, c) => out.push({ path, bytes: encodePng(c) });

    for (const s of [16, 32, 48, 64, 128, 256, 512]) png(`assets/linux/icons/hicolor/${s}x${s}/apps/agentmux.png`, drawIcon(dark, s));
    png("frontend/app/asset/agentmux-icon.png", drawIcon(dark, 256));
    out.push({ path: "crates/cef/resources/win/agentmux.ico", bytes: encodeIco([16, 24, 32, 48, 64, 256].map((s) => drawIcon(dark, s))) });
    // The light one, embedded as resource 2 beside the dark one: the host swaps the window icon to it while Windows is in light mode.
    out.push({ path: "crates/cef/resources/win/agentmux-light.ico", bytes: encodeIco([16, 24, 32, 48, 64, 256].map((s) => drawIcon(light, s))) });
    for (const s of [71, 150, 300]) png(`assets/favicon-${s}x${s}.png`, drawIcon(dark, s));

    // MSIX: the Store and tile logos, and the taskbar sets (dark tile for a dark taskbar, light tile for a light one).
    const msix = "packaging/msix/assets";
    png(`${msix}/StoreLogo.png`, drawIcon(dark, 50));
    for (const s of [44, 71, 150, 310]) png(`${msix}/Square${s}x${s}Logo.png`, drawIcon(dark, s));
    png(`${msix}/Wide310x150Logo.png`, drawCentred(dark, 310, 150));
    for (const s of [16, 24, 32, 48, 256]) {
        png(`${msix}/Square44x44Logo.targetsize-${s}_altform-unplated.png`, drawIcon(dark, s));
        png(`${msix}/Square44x44Logo.targetsize-${s}_altform-lightunplated.png`, drawIcon(light, s));
    }

    png("assets/macos/agentmux-1024.png", drawMacIcon(dark, 1024));
    png("crates/launcher/resources/menubar-template.png", drawMenubarGlyph());

    // The mark the app draws for itself, blocks only: the web views' SVG, and the launcher's splash PNG
    // (256 x 256, RGBA, the size build.rs reads; the file keeps its old name, brain.png, so no Rust changes).
    out.push({ path: "frontend/app/asset/logo-mark.svg", bytes: Buffer.from(markSvg(dark)) });
    const mark = canvas(256, 256);
    for (const r of markRects(dark)) fillRect(mark, r.x / 2, r.y / 2, r.w / 2, r.h / 2, r.fill);
    png("crates/launcher/resources/brain.png", mark);

    // The favicon set for the landing and docs sites (their repos take these files).
    const web = "assets/icon/web";
    out.push({ path: `${web}/favicon.svg`, bytes: Buffer.from(faviconSvg(dark, light)) });
    png(`${web}/favicon-32.png`, drawIcon(dark, 32));
    png(`${web}/favicon-32-dark-mode.png`, drawIcon(light, 32));
    png(`${web}/apple-touch-icon-180.png`, drawIcon(dark, 180));
    out.push({ path: `${web}/favicon.ico`, bytes: encodeIco([16, 32, 48].map((s) => drawIcon(dark, s))) });
    return out;
}

const sha = (b) => createHash("sha256").update(b).digest("hex");

function main(argv) {
    const check = argv.includes("--check");
    const stale = [];
    for (const { path, bytes } of outputs()) {
        const file = join(ROOT, path);
        const same = existsSync(file) && sha(readFileSync(file)) === sha(bytes);
        if (same) continue;
        if (check) {
            stale.push(path);
        } else {
            mkdirSync(dirname(file), { recursive: true });
            writeFileSync(file, bytes);
            console.log(`  wrote ${path}`);
        }
    }
    if (check && stale.length) {
        console.error(`gen-app-icons: ${stale.length} file(s) are not what the SVGs in assets/icon/ draw:\n${stale.map((p) => `  - ${p}`).join("\n")}\nRun: node scripts/gen-app-icons.mjs`);
        process.exit(1);
    }
    if (check) console.log("gen-app-icons: ok (every icon is current)");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main(process.argv.slice(2));
