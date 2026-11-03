// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for gen-app-icons.mjs: the SVG reader, the pixel grid of the icon, the PNG and ICO encoders, and that
// every committed icon is what the SVGs draw.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { inflateSync } from "node:zlib";
import { describe, expect, it } from "vitest";
import { encodeIco, encodePng, outputs, readRects } from "./gen-app-icons.mjs";

const ROOT = join(import.meta.dirname, "..");
const read = (p) => readFileSync(join(ROOT, p), "utf8");

/** Decode an 8-bit RGBA PNG written by encodePng: `{ w, h, at(x, y) -> [r, g, b, a] }`. */
function decode(png) {
    const w = png.readUInt32BE(16);
    const h = png.readUInt32BE(20);
    let idat = Buffer.alloc(0);
    for (let o = 8; o < png.length; ) {
        const len = png.readUInt32BE(o);
        if (png.toString("ascii", o + 4, o + 8) === "IDAT") idat = Buffer.concat([idat, png.subarray(o + 8, o + 8 + len)]);
        o += 12 + len;
    }
    const raw = inflateSync(idat);
    return { w, h, at: (x, y) => [...raw.subarray(y * (w * 4 + 1) + 1 + x * 4, y * (w * 4 + 1) + 1 + x * 4 + 4)] };
}
const out = (path) => outputs().find((o) => o.path === path)?.bytes;

describe("the icon sources", () => {
    it("read as rectangles: a tile and the four blocks, flat colours", () => {
        for (const f of ["assets/icon/agentmux-dark.svg", "assets/icon/agentmux-light.svg"]) {
            const rects = readRects(read(f));
            expect(rects.length).toBeGreaterThanOrEqual(5);
            for (const r of rects) expect(r.fill).toHaveLength(3);
        }
        expect(() => readRects('<svg><rect width="4" height="4" fill="url(#g)"/></svg>')).toThrow(/fill/);
    });

    it("sit on the 16 px grid: every edge is a whole pixel at 16 px", () => {
        for (const f of ["assets/icon/agentmux-dark.svg", "assets/icon/agentmux-light.svg"]) {
            for (const r of readRects(read(f))) {
                for (const v of [r.x, r.y, r.w, r.h]) expect(v % 32).toBe(0);
            }
        }
    });
});

describe("drawing", () => {
    it("at 16 px the dark icon has hard edges: every pixel is the tile's or a block's, none in between", () => {
        const img = decode(out("assets/linux/icons/hicolor/16x16/apps/agentmux.png"));
        const colours = new Set();
        for (let y = 0; y < 16; y++) for (let x = 0; x < 16; x++) colours.add(img.at(x, y).join(","));
        expect([...colours].sort()).toEqual(["107,130,222,255", "17,16,22,255"]);
        expect(img.at(0, 0)).toEqual([17, 16, 22, 255]); // square tile corner, not transparent
        expect(img.at(2, 2)).toEqual([107, 130, 222, 255]); // the bar starts at 2 px
        expect(img.at(2, 4)).toEqual([17, 16, 22, 255]); // then a 2 px gap
    });

    it("keeps the same layout at 48 px (3 px per unit)", () => {
        const img = decode(out("packaging/msix/assets/Square44x44Logo.targetsize-48_altform-unplated.png"));
        expect(img.at(6, 6)).toEqual([107, 130, 222, 255]);
        expect(img.at(5, 6)).toEqual([17, 16, 22, 255]);
    });

    it("the light icon has an edge a white taskbar shows, and a light tile", () => {
        const img = decode(out("packaging/msix/assets/Square44x44Logo.targetsize-32_altform-lightunplated.png"));
        expect(img.at(0, 0)).toEqual([207, 204, 226, 255]); // the edge: 1 px at 16, 2 px at 32
        expect(img.at(1, 1)).toEqual([207, 204, 226, 255]);
        expect(img.at(2, 2)).toEqual([243, 242, 248, 255]);
        expect(img.at(4, 4)).toEqual([75, 95, 207, 255]);
    });

    it("the macOS icon is the dark tile in Apple's rounded template, with a transparent margin", () => {
        const img = decode(out("assets/macos/agentmux-1024.png"));
        expect(img.at(0, 0)[3]).toBe(0); // outside the template
        expect(img.at(100, 100)[3]).toBeLessThan(30); // the rounded corner is cut
        expect(img.at(512, 140)).toEqual([17, 16, 22, 255]); // the tile
        expect(img.at(512, 250)).toEqual([107, 130, 222, 255]); // the bar
    });

    it("the menu-bar glyph is black ink on transparent, 36 x 36", () => {
        const img = decode(out("crates/launcher/resources/menubar-template.png"));
        expect([img.w, img.h]).toEqual([36, 36]);
        expect(img.at(0, 0)[3]).toBe(0);
        expect(img.at(4, 4)).toEqual([0, 0, 0, 255]);
    });

    it("the Wide logo is the square icon centred on a transparent canvas", () => {
        const img = decode(out("packaging/msix/assets/Wide310x150Logo.png"));
        expect([img.w, img.h]).toEqual([310, 150]);
        expect(img.at(0, 75)[3]).toBe(0);
        expect(img.at(155, 75)[3]).toBe(255);
    });
});

describe("encoding", () => {
    it("an ICO lists each image with its size, PNG inside", () => {
        const ico = out("crates/cef/resources/win/agentmux.ico");
        expect(ico.readUInt16LE(2)).toBe(1);
        const n = ico.readUInt16LE(4);
        expect(n).toBe(6);
        const sizes = [];
        for (let i = 0; i < n; i++) {
            sizes.push(ico[6 + 16 * i] || 256);
            const off = ico.readUInt32LE(6 + 16 * i + 12);
            expect(ico.subarray(off, off + 4)).toEqual(Buffer.from([0x89, 0x50, 0x4e, 0x47]));
        }
        expect(sizes).toEqual([16, 24, 32, 48, 64, 256]);
        expect(encodeIco([]).length).toBe(6);
    });

    it("a PNG has its signature, and its pixels read back", () => {
        const png = encodePng({ w: 2, h: 1, px: new Float64Array([1, 0, 0, 1, 0, 0, 0, 0]) });
        expect(png.subarray(0, 8)).toEqual(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]));
        expect(decode(png).at(0, 0)).toEqual([255, 0, 0, 255]);
        expect(decode(png).at(1, 0)[3]).toBe(0);
    });
});

describe("committed files", () => {
    it("every icon in the repo is what the SVGs draw (run node scripts/gen-app-icons.mjs)", () => {
        const stale = outputs()
            .filter((o) => !readFileSync(join(ROOT, o.path)).equals(o.bytes))
            .map((o) => o.path);
        expect(stale).toEqual([]);
    });
});
