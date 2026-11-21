// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What makes a screenshot crisp (docs/specs/SPEC_WEBSITE_DOCS_MEDIA_REFRESH_2026_11_20.md §5): captured at
// least two device pixels per CSS pixel, and at least twice as many pixels as the width it is shown at. The
// rules are plain functions so the capture (which refuses a blurry one), `check-crisp.mjs` (which fails one)
// and their tests all use the same ones.

/** Device pixels per CSS pixel every shot and clip is captured at. */
export const CAPTURE_SCALE = 2;
/** The lowest scale a capture may have; a lower one is refused, not warned about. */
export const MIN_CAPTURE_SCALE = 2;
/** The width in CSS pixels a docs page shows an image in (Starlight's content column, 45 rem). */
export const DOCS_SLOT = 720;

/** Reads a raster file's pixel size from its header: PNG, WebP, GIF or JPEG; `null` for anything else. */
export function readImageSize(buf) {
    if (buf.length >= 24 && buf.readUInt32BE(0) === 0x89504e47) return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20) };
    if (buf.length >= 10 && buf.toString("ascii", 0, 3) === "GIF") return { width: buf.readUInt16LE(6), height: buf.readUInt16LE(8) };
    if (buf.length >= 30 && buf.toString("ascii", 0, 4) === "RIFF" && buf.toString("ascii", 8, 12) === "WEBP") {
        const kind = buf.toString("ascii", 12, 16);
        if (kind === "VP8 ") return { width: buf.readUInt16LE(26) & 0x3fff, height: buf.readUInt16LE(28) & 0x3fff };
        if (kind === "VP8L") {
            const bits = buf.readUInt32LE(21);
            return { width: (bits & 0x3fff) + 1, height: ((bits >> 14) & 0x3fff) + 1 };
        }
        if (kind === "VP8X") return { width: buf.readUIntLE(24, 3) + 1, height: buf.readUIntLE(27, 3) + 1 };
        return null;
    }
    if (buf.length > 4 && buf[0] === 0xff && buf[1] === 0xd8) {
        let i = 2;
        while (i + 9 < buf.length) {
            if (buf[i] !== 0xff) {
                i++;
                continue;
            }
            const marker = buf[i + 1];
            if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) {
                return { width: buf.readUInt16BE(i + 7), height: buf.readUInt16BE(i + 5) };
            }
            i += 2 + buf.readUInt16BE(i + 2);
        }
    }
    return null;
}

/** Why `scale` is too low for a capture, or `null` when it is fine. */
export function scaleProblem(scale) {
    if (typeof scale !== "number" || !Number.isFinite(scale)) return "no capture scale was recorded";
    if (scale < MIN_CAPTURE_SCALE) return `captured at scale ${scale}; at least ${MIN_CAPTURE_SCALE} is needed`;
    return null;
}

/** The problems of one shot's entry in a run's manifest.json: its capture scale, and that its pixels are
 *  its CSS size times that scale (to within a pixel of rounding). Empty when it is crisp. */
export function shotProblems(entry) {
    const problems = [];
    const scale = scaleProblem(entry.capture_scale);
    if (scale) problems.push(scale);
    const { css_width: cw, css_height: ch, width, height } = entry;
    if (![cw, ch, width, height].every((n) => typeof n === "number" && n > 0)) {
        problems.push("its CSS size or pixel size is missing");
        return problems;
    }
    if (!scale) {
        const want = { width: Math.round(cw * entry.capture_scale), height: Math.round(ch * entry.capture_scale) };
        if (Math.abs(width - want.width) > 1 || Math.abs(height - want.height) > 1) {
            problems.push(`is ${width}x${height} px, but ${cw}x${ch} CSS px at scale ${entry.capture_scale} is ${want.width}x${want.height}`);
        }
    }
    return problems;
}

/** Why an image `width` pixels wide is too small for a slot `slotWidth` CSS pixels wide, or `null`. It needs at
 *  least twice the slot's width; `ratio` is what it has over what it needs (1 or more passes). */
export function slotProblem(width, slotWidth) {
    const need = slotWidth * 2;
    if (width >= need) return null;
    return `${width} px wide, needs ${need} for a ${slotWidth} px slot (${(width / need).toFixed(2)})`;
}

/** The problems of a finished clip: captured at 2x or more, at least 1080 lines high, and not enlarged from its
 *  source frames. `source` is the pixel width of the captured frames, when known. Empty when it is crisp. */
export function clipProblems({ captureScale, width, height, source }) {
    const problems = [];
    const scale = scaleProblem(captureScale);
    if (scale) problems.push(scale);
    if (!(height >= 1080)) problems.push(`${width}x${height}: under 1080 lines high`);
    if (typeof source === "number" && width > source) problems.push(`${width} px wide from ${source} px frames (enlarged)`);
    return problems;
}
