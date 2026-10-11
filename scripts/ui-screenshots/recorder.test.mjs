// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The recorder's pure parts (lib/recorder.mjs), live redaction (redact.mjs)
// and the recordings suite's shape. Recording itself needs a running instance
// and ffmpeg, and isn't tested here.

import { afterEach, describe, expect, it } from "vitest";
import { concatList, filterFor, findFfmpeg } from "./lib/recorder.mjs";
import { liveRedactExpression, stopLiveRedactionExpression } from "./redact.mjs";
import { STASH_TABS, recordings, setup } from "./recordings.mjs";

afterEach(() => {
    (0, eval)(stopLiveRedactionExpression);
    document.body.innerHTML = "";
});

describe("concatList", () => {
    it("shows each frame until the next was painted, the last until the end plus the hold", () => {
        const list = concatList(
            [
                { t: 10.0 },
                { t: 10.5 },
                { t: 12.0 },
            ],
            13.0,
            1.2
        );
        expect(list.split("\n")).toEqual([
            "ffconcat version 1.0",
            "file 'frame_00000.png'",
            "duration 0.500",
            "file 'frame_00001.png'",
            "duration 1.500",
            "file 'frame_00002.png'",
            "duration 2.200",
            // Repeated, or the concat demuxer drops the last duration.
            "file 'frame_00002.png'",
            "",
        ]);
    });
});

describe("filterFor", () => {
    it("resamples, crops to the clip and scales to the width", () => {
        expect(filterFor({ fps: 15, crop: { x: 10.4, y: 0, width: 800.6, height: 600 }, width: 640 })).toBe(
            "fps=15,crop=801:600:10:0,scale=640:-2:flags=lanczos"
        );
        expect(filterFor({ fps: 12 })).toBe("fps=12");
    });
});

describe("findFfmpeg", () => {
    it("explains where it looked when there is none", () => {
        const saved = { PATH: process.env.PATH, FFMPEG: process.env.FFMPEG };
        process.env.PATH = "";
        delete process.env.FFMPEG;
        try {
            expect(() => findFfmpeg("/no/such/ffmpeg")).toThrow(/--ffmpeg.*FFMPEG.*PATH/);
        } finally {
            process.env.PATH = saved.PATH;
            if (saved.FFMPEG !== undefined) process.env.FFMPEG = saved.FFMPEG;
        }
    });
});

describe("live redaction", () => {
    it("redacts what's there, and what the page renders afterwards", async () => {
        document.body.innerHTML = `<p id="a">alice's tab</p>`;
        (0, eval)(liveRedactExpression([{ from: "alice", to: "demo" }]));
        expect(document.getElementById("a").textContent).toBe("demo's tab");
        const later = document.createElement("p");
        later.textContent = "C--Users-alice--agentmux";
        document.body.appendChild(later);
        await new Promise((r) => setTimeout(r, 0));
        expect(later.textContent).toBe("C--Users-demo--agentmux");
    });

    it("stops when told to", async () => {
        (0, eval)(liveRedactExpression([{ from: "alice", to: "demo" }]));
        (0, eval)(stopLiveRedactionExpression);
        const p = document.createElement("p");
        p.textContent = "alice";
        document.body.appendChild(p);
        await new Promise((r) => setTimeout(r, 0));
        expect(p.textContent).toBe("alice");
    });
});

describe("recordings suite", () => {
    it("has unique ids, and a run and cleanup for each clip", () => {
        const ids = recordings.map((r) => r.id);
        expect(new Set(ids).size).toBe(ids.length);
        for (const r of recordings) {
            expect(typeof r.run, r.id).toBe("function");
            expect(typeof r.cleanup, r.id).toBe("function");
        }
        expect(typeof setup).toBe("function");
        expect(STASH_TABS[0]).toBe("Accounts");
    });
});
