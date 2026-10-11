// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The recorder's parts — see docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md
// §12: a cursor drawn into the page (input sent over CDP moves no real
// pointer, so a recording would otherwise show things happening by
// themselves), the page's frames from CDP's screencast with the time each was
// painted, and an encoder that turns them into a GIF (and WebP or MP4) with
// ffmpeg.

import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";

const CURSOR_ID = "amrec-cursor";

/** In the page: adds the drawn cursor (an arrow) at `x, y`, above everything
 *  and never in the way of a click. */
export const cursorExpression = (x, y) => `(() => {
    document.getElementById(${JSON.stringify(CURSOR_ID)})?.remove();
    const c = document.createElement("div");
    c.id = ${JSON.stringify(CURSOR_ID)};
    c.style.cssText = "position:fixed;left:0;top:0;width:22px;height:22px;z-index:2147483647;pointer-events:none;" +
        "transition:transform 450ms cubic-bezier(.3,.7,.3,1);transform:translate(${x}px,${y}px)";
    c.innerHTML = '<svg width="22" height="22" viewBox="0 0 22 22"><path d="M3 2 L3 18 L7.5 14 L10.5 20.5 L13 19.4 L10 13 L16 13 Z" ' +
        'fill="#fff" stroke="#000" stroke-width="1.4" stroke-linejoin="round"/></svg>';
    document.body.appendChild(c);
    const style = document.createElement("style");
    style.id = ${JSON.stringify(`${CURSOR_ID}-style`)};
    style.textContent = "@keyframes amrec-ripple{from{transform:scale(.2);opacity:.9}to{transform:scale(1.6);opacity:0}}" +
        ".amrec-ripple{position:fixed;width:34px;height:34px;margin:-17px 0 0 -17px;border-radius:50%;" +
        "border:3px solid #4aa3ff;pointer-events:none;z-index:2147483646;animation:amrec-ripple 500ms ease-out forwards}";
    document.head.appendChild(style);
    return true;
})()`;

export const removeCursorExpression = `(() => {
    document.getElementById(${JSON.stringify(CURSOR_ID)})?.remove();
    document.getElementById(${JSON.stringify(`${CURSOR_ID}-style`)})?.remove();
    document.querySelectorAll(".amrec-ripple").forEach((e) => e.remove());
})()`;

/** Drives the drawn cursor and the real (CDP) pointer together. */
export class Cursor {
    constructor(session) {
        this.session = session;
        this.at = { x: 40, y: 40 };
    }

    async show(at = this.at) {
        this.at = at;
        await this.session.evaluate(cursorExpression(at.x, at.y));
    }

    async hide() {
        await this.session.evaluate(removeCursorExpression);
    }

    /** Glides to `pt` (the CSS transition draws the path), then waits. */
    async moveTo(pt, settleMs = 550) {
        this.at = pt;
        await this.session.evaluate(
            `(() => { const c = document.getElementById(${JSON.stringify(CURSOR_ID)}); if (c) c.style.transform = "translate(${pt.x}px,${pt.y}px)"; })()`
        );
        await this.session.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: pt.x, y: pt.y });
        await this.session.wait(settleMs);
    }

    /** Moves to `pt` and clicks there, with a ripple. */
    async click(pt) {
        await this.moveTo(pt);
        await this.session.evaluate(`(() => {
            const r = document.createElement("div");
            r.className = "amrec-ripple";
            r.style.left = "${pt.x}px";
            r.style.top = "${pt.y}px";
            document.body.appendChild(r);
            setTimeout(() => r.remove(), 600);
        })()`);
        await this.session.clickAt(pt);
    }

    /** Clicks the centre of the first visible element matching `selector`,
     *  optionally the one whose text includes `text`. */
    async clickSelector(selector, text) {
        const pt = await this.session.evaluate(`(() => {
            const el = [...document.querySelectorAll(${JSON.stringify(selector)})].find((e) => {
                const r = e.getBoundingClientRect();
                if (r.width === 0 || r.height === 0) return false;
                return ${JSON.stringify(text ?? "")} === "" || e.innerText.includes(${JSON.stringify(text ?? "")});
            });
            if (!el) return null;
            const r = el.getBoundingClientRect();
            return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) };
        })()`);
        if (!pt) throw new Error(`nothing to click: ${selector}${text ? ` with "${text}"` : ""}`);
        await this.click(pt);
    }
}

/** Collects the page's frames from CDP's screencast, each with the time it
 *  was painted (a still page sends none, so a pause keeps its length). */
export class Screencast {
    constructor(session) {
        this.session = session;
        this.frames = [];
        this.onFrame = (p) => {
            this.frames.push({ data: p.data, t: p.metadata.timestamp, width: p.metadata.deviceWidth, height: p.metadata.deviceHeight });
            this.session.send("Page.screencastFrameAck", { sessionId: p.sessionId }).catch(() => {});
        };
    }

    async start() {
        this.frames = [];
        this.session.on("Page.screencastFrame", this.onFrame);
        await this.session.send("Page.startScreencast", { format: "png", everyNthFrame: 1 });
    }

    /** Stops, and returns the frames with the time the recording ended. */
    async stop() {
        const end = Date.now() / 1000;
        await this.session.send("Page.stopScreencast").catch(() => {});
        this.session.off("Page.screencastFrame", this.onFrame);
        return { frames: this.frames, end };
    }
}

/** ffmpeg: `--ffmpeg`, else $FFMPEG, else the first `ffmpeg` on PATH. */
export function findFfmpeg(explicit) {
    const candidates = [explicit, process.env.FFMPEG];
    for (const dir of (process.env.PATH ?? "").split(delimiter)) {
        candidates.push(join(dir, process.platform === "win32" ? "ffmpeg.exe" : "ffmpeg"));
    }
    const found = candidates.find((c) => c && existsSync(c));
    if (!found) throw new Error("ffmpeg not found: pass --ffmpeg <path>, set FFMPEG, or put it on PATH");
    return found;
}

/** The concat list ffmpeg reads: each frame shown until the next one was
 *  painted, the last until `end`, `endHoldS` more at the end. A frame shown
 *  under 1/fps is folded into its neighbour by ffmpeg's fps filter. */
export function concatList(frames, end, endHoldS = 0) {
    const lines = ["ffconcat version 1.0"];
    frames.forEach((f, i) => {
        const next = i + 1 < frames.length ? frames[i + 1].t : end;
        const d = Math.max(0.001, next - f.t) + (i + 1 === frames.length ? endHoldS : 0);
        lines.push(`file 'frame_${String(i).padStart(5, "0")}.png'`, `duration ${d.toFixed(3)}`);
    });
    // The concat demuxer drops the last entry's duration without a repeat.
    if (frames.length) lines.push(`file 'frame_${String(frames.length - 1).padStart(5, "0")}.png'`);
    return lines.join("\n") + "\n";
}

/** Rounds the width and height down to even numbers (for H.264). */
export const EVEN_DIMENSIONS = "scale=trunc(iw/2)*2:trunc(ih/2)*2";

/** The ffmpeg filter for `crop` (CSS pixels of the frame), scaled to `width`. */
export function filterFor({ crop, width, fps }) {
    const parts = [`fps=${fps}`];
    if (crop) parts.push(`crop=${Math.round(crop.width)}:${Math.round(crop.height)}:${Math.round(crop.x)}:${Math.round(crop.y)}`);
    if (width) parts.push(`scale=${width}:-2:flags=lanczos`);
    return parts.join(",");
}

/** Encodes `frames` into `out` + each extension in `formats` (gif, webp,
 *  mp4). Returns the files written. */
export function encode({ frames, end, crop, out, formats = ["gif"], width = 800, fps = 15, endHoldS = 1.2, ffmpeg }) {
    if (!frames.length) throw new Error("no frames were recorded");
    const dir = mkdtempSync(join(tmpdir(), "amrec-"));
    try {
        frames.forEach((f, i) => writeFileSync(join(dir, `frame_${String(i).padStart(5, "0")}.png`), Buffer.from(f.data, "base64")));
        writeFileSync(join(dir, "frames.txt"), concatList(frames, end, endHoldS));
        const base = filterFor({ crop, width, fps });
        const input = ["-hide_banner", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i", join(dir, "frames.txt")];
        const written = [];
        for (const fmt of formats) {
            const file = `${out}.${fmt}`;
            let args;
            if (fmt === "gif") {
                // One palette for the whole clip; dithering only where frames change.
                args = [...input, "-vf", `${base},split[a][b];[a]palettegen=stats_mode=diff:max_colors=192[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle`, "-loop", "0", file];
            } else if (fmt === "webp") {
                args = [...input, "-vf", base, "-c:v", "libwebp_anim", "-lossless", "0", "-q:v", "80", "-loop", "0", file];
            } else if (fmt === "mp4") {
                // H.264 in yuv420p needs even dimensions; an odd --width or crop
                // is rounded down a pixel (Codex on #4670).
                args = [...input, "-vf", `${base},${EVEN_DIMENSIONS},format=yuv420p`, "-c:v", "libx264", "-crf", "24", "-preset", "slow", "-movflags", "+faststart", file];
            } else {
                throw new Error(`unknown format "${fmt}" (gif, webp, mp4)`);
            }
            const r = spawnSync(ffmpeg, args, { encoding: "utf8" });
            if (r.status !== 0) throw new Error(`ffmpeg (${fmt}) failed: ${r.stderr || r.error?.message}`);
            written.push(file);
        }
        return written;
    } finally {
        rmSync(dir, { recursive: true, force: true });
    }
}

/** ffprobe-free sanity check: the duration ffmpeg reads back from `file`. */
export function durationOf(ffmpeg, file) {
    try {
        execFileSync(ffmpeg, ["-hide_banner", "-i", file], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    } catch (e) {
        const m = /Duration: (\d+):(\d+):([\d.]+)/.exec(String(e.stderr));
        if (m) return +m[1] * 3600 + +m[2] * 60 + +m[3];
    }
    return null;
}

export function ensureDir(dir) {
    mkdirSync(dir, { recursive: true });
    return dir;
}
