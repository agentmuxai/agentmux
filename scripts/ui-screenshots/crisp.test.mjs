// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What counts as crisp (lib/crisp.mjs) and the check that fails what is not (check-crisp.mjs).

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { checkDir, checkRun, DEFAULT_EXEMPT } from "./check-crisp.mjs";
import { CAPTURE_SCALE, clipProblems, clipSize, deviceCrop, readImageSize, scaleProblem, shotProblems, slotProblem } from "./lib/crisp.mjs";
import { SIZES } from "./sizes.mjs";

/** A PNG header with the given size (enough for the size reader; not a decodable image). */
function png(width, height) {
    const b = Buffer.alloc(33);
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(b, 0);
    b.writeUInt32BE(13, 8);
    b.write("IHDR", 12, "ascii");
    b.writeUInt32BE(width, 16);
    b.writeUInt32BE(height, 20);
    return b;
}

/** A lossy WebP header (VP8) with the given size. */
function webp(width, height) {
    const b = Buffer.alloc(40);
    b.write("RIFF", 0, "ascii");
    b.write("WEBP", 8, "ascii");
    b.write("VP8 ", 12, "ascii");
    b.writeUInt16LE(width, 26);
    b.writeUInt16LE(height, 28);
    return b;
}

const dirs = [];
function tmp() {
    const d = mkdtempSync(join(tmpdir(), "amcrisp-"));
    dirs.push(d);
    return d;
}
afterEach(() => {
    for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

describe("the capture scale", () => {
    it("is 2, and every named size is captured at it", () => {
        expect(CAPTURE_SCALE).toBe(2);
        for (const [name, size] of Object.entries(SIZES)) expect([name, size.scale]).toEqual([name, 2]);
    });

    it("refuses a scale under 2 and one that was never recorded", () => {
        expect(scaleProblem(2)).toBeNull();
        expect(scaleProblem(3)).toBeNull();
        expect(scaleProblem(1)).toMatch(/scale 1; at least 2/);
        expect(scaleProblem(1.5)).not.toBeNull();
        expect(scaleProblem(undefined)).toMatch(/no capture scale/);
        expect(scaleProblem(Number.NaN)).not.toBeNull();
    });
});

describe("readImageSize", () => {
    it("reads PNG and WebP headers, and nothing else", () => {
        expect(readImageSize(png(2560, 1488))).toEqual({ width: 2560, height: 1488 });
        expect(readImageSize(webp(1440, 900))).toEqual({ width: 1440, height: 900 });
        expect(readImageSize(Buffer.from("not an image at all, just text of some length here"))).toBeNull();
    });
});

describe("shotProblems", () => {
    const ok = { id: "x", file: "x.png", capture_scale: 2, css_width: 1280, css_height: 744, width: 2560, height: 1488 };

    it("passes a shot captured at 2 whose pixels are its CSS size times 2", () => {
        expect(shotProblems(ok)).toEqual([]);
        expect(shotProblems({ ...ok, css_width: 330.4, width: 661 })).toEqual([]);
    });

    it("fails a shot captured at 1x, which is what every older run is", () => {
        const old = { id: "x", file: "x.png", width: 1280, height: 744 };
        const problems = shotProblems(old);
        expect(problems.some((p) => /no capture scale/.test(p))).toBe(true);
        expect(shotProblems({ ...ok, capture_scale: 1, width: 1280, height: 744 })[0]).toMatch(/captured at scale 1/);
    });

    it("fails a shot whose pixels are not its CSS size times the scale", () => {
        const [p] = shotProblems({ ...ok, width: 1280, height: 744 });
        expect(p).toMatch(/is 1280x744 px, but 1280x744 CSS px at scale 2 is 2560x1488/);
    });
});

describe("slotProblem", () => {
    it("needs at least twice the slot's width", () => {
        expect(slotProblem(1440, 720)).toBeNull();
        expect(slotProblem(2560, 720)).toBeNull();
        expect(slotProblem(1280, 720)).toBe("1280 px wide, needs 1440 for a 720 px slot (0.89)");
        expect(slotProblem(800, 720)).toMatch(/\(0\.56\)/);
    });
});

describe("clipProblems", () => {
    it("wants scale 2, 1080 lines and no enlargement", () => {
        expect(clipProblems({ captureScale: 2, width: 1920, height: 1080, source: 2560 })).toEqual([]);
        expect(clipProblems({ captureScale: 1, width: 800, height: 500 }).length).toBe(2);
        expect(clipProblems({ captureScale: 2, width: 1920, height: 1080, source: 1280 })).toEqual(["1920 px wide from 1280 px frames (enlarged)"]);
    });

    it("fails a video under 1080 lines, naming its size", () => {
        expect(clipProblems({ captureScale: 2, width: 1400, height: 1078, source: 1400 })).toEqual(["1400x1078: under 1080 lines high"]);
    });
});

describe("deviceCrop", () => {
    const frame = { width: 2560, height: 1440 };
    it("turns a CSS box into the frame's pixels, with an even size", () => {
        expect(deviceCrop({ x: 10.4, y: 20, width: 600.3, height: 500 }, 2, frame)).toEqual({ x: 21, y: 40, width: 1200, height: 1000 });
    });

    it("keeps the box inside the frame, and gives nothing when none of it is", () => {
        expect(deviceCrop({ x: 1000, y: 0, width: 600, height: 720 }, 2, frame)).toEqual({ x: 2000, y: 0, width: 560, height: 1440 });
        expect(deviceCrop({ x: 1400, y: 0, width: 50, height: 50 }, 2, frame)).toBeNull();
        expect(deviceCrop({ x: -20, y: -20, width: 100, height: 100 }, 2, frame)).toEqual({ x: 0, y: 0, width: 200, height: 200 });
    });
});

describe("clipSize", () => {
    const frame = { width: 2560, height: 1440 };
    it("keeps the captured size, or the crop's, unless told to scale down", () => {
        expect(clipSize({ frame })).toEqual({ width: 2560, height: 1440 });
        expect(clipSize({ frame, crop: { x: 0, y: 0, width: 1200, height: 1000 } })).toEqual({ width: 1200, height: 1000 });
        expect(clipSize({ frame, width: 1920 })).toEqual({ width: 1920, height: 1080 });
    });

    it("reports a width over the captured one, so clipProblems can refuse the enlargement", () => {
        const size = clipSize({ frame, width: 3000 });
        expect(clipProblems({ captureScale: 2, ...size, source: frame.width })).toEqual(["3000 px wide from 2560 px frames (enlarged)"]);
    });
});

describe("check-crisp", () => {
    it("checks a run: the manifest's scale and sizes, and the PNG on disk", () => {
        const dir = tmp();
        writeFileSync(join(dir, "a.png"), png(2560, 1488));
        writeFileSync(join(dir, "b.png"), png(1280, 744));
        writeFileSync(join(dir, "c.png"), png(1000, 700));
        writeFileSync(
            join(dir, "manifest.json"),
            JSON.stringify({
                shots: [
                    { id: "a", file: "a.png", capture_scale: 2, css_width: 1280, css_height: 744, width: 2560, height: 1488 },
                    { id: "b", file: "b.png", width: 1280, height: 744 },
                    { id: "c", file: "c.png", capture_scale: 2, css_width: 500, css_height: 350, width: 2000, height: 1400 },
                ],
            })
        );
        const failures = checkRun(dir);
        expect(failures.map((f) => f.where.replaceAll("\\", "/").split("/").pop())).toEqual(["b.png", "c.png"]);
        expect(failures[1].problems.join(" ")).toMatch(/the file is 1000x700 px; the manifest says 2000x1400/);
    });

    it("checks the clips of a recordings manifest: scale, enlargement and, for a video, 1080 lines", () => {
        const dir = tmp();
        writeFileSync(
            join(dir, "manifest.json"),
            JSON.stringify({
                recordings: [
                    { id: "good", formats: ["mp4", "poster"], capture_scale: 2, width: 2560, height: 1440, source_width: 2560 },
                    { id: "short", formats: ["mp4"], capture_scale: 2, width: 1200, height: 1000, source_width: 1200 },
                    { id: "gif-only", formats: ["gif"], capture_scale: 2, width: 1200, height: 1000, source_width: 1200 },
                    { id: "old", formats: ["mp4"], width: 2560, height: 1440, source_width: 2560 },
                ],
            })
        );
        const failures = checkRun(dir);
        expect(failures.map((f) => f.where)).toEqual(["short", "old"]);
        expect(failures[0].problems).toEqual(["1200x1000: under 1080 lines high"]);
        expect(failures[1].problems).toEqual(["no capture scale was recorded"]);
    });

    it("checks a folder against a slot, skipping vector files, icons and link-preview images", () => {
        const dir = tmp();
        mkdirSync(join(dir, "widgets"));
        writeFileSync(join(dir, "widgets", "wide.png"), png(1440, 900));
        writeFileSync(join(dir, "widgets", "narrow.png"), png(800, 522));
        writeFileSync(join(dir, "diagram.svg"), "<svg/>");
        writeFileSync(join(dir, "favicon-32.png"), png(32, 32));
        writeFileSync(join(dir, "og-image-v2.png"), png(1200, 630));
        writeFileSync(join(dir, "shot.webp"), webp(2000, 1200));
        const rows = checkDir(dir, { slot: 720 });
        expect(rows.map((r) => [r.file, r.problem === null]).sort((a, b) => a[0].localeCompare(b[0]))).toEqual([
            ["shot.webp", true],
            ["widgets/narrow.png", false],
            ["widgets/wide.png", true],
        ]);
        expect(DEFAULT_EXEMPT.test("brand/stores/google-play-badge.png")).toBe(true);
        expect(DEFAULT_EXEMPT.test("widgets/widget-agent-small.png")).toBe(false);
    });
});
