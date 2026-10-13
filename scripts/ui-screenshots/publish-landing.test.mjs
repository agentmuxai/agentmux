// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// publish-landing.mjs: the landing manifest, which run file each entry comes
// from, the ffmpeg commands, and that only listed files are written.

import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { describe, expect, it } from "vitest";
import { encodeCommands, landingPlan, publishLanding, readManifest } from "./publish-landing.mjs";
import { runImages } from "./publish.mjs";

const run = runImages(
    {
        shots: [
            { id: "agent-conversation", file: "04-agent-conversation.png", containsWorkspaceData: "review" },
            { id: "widget-tower", size: "small", file: "widget-tower-small.png", containsWorkspaceData: "review" },
            { id: "widget-tower", size: "large", file: "widget-tower-large.png", containsWorkspaceData: "review" },
            { id: "widget-toolchain", file: "widget-toolchain.png", containsWorkspaceData: true },
        ],
        recordings: [{ id: "swarm-follows-agent", files: ["swarm-follows-agent.gif", "swarm-follows-agent.mp4"] }],
    },
    "/run",
);

describe("readManifest", () => {
    it("accepts shots as .webp and clips as .mp4 with a .webp poster", () => {
        const entries = readManifest({
            images: [
                { file: "ui-agent.webp", shot: "agent-conversation", width: 960 },
                { file: "ui-swarm-follows.mp4", clip: "swarm-follows-agent", poster: "ui-swarm-follows-poster.webp" },
            ],
        });
        expect(entries).toHaveLength(2);
    });

    it("lists every problem: paths, duplicates, both or neither source, wrong formats", () => {
        expect(() =>
            readManifest({
                images: [
                    { file: "../../src/App.tsx", shot: "x" },
                    { file: "a.webp", shot: "x", clip: "y" },
                    { file: "a.webp" },
                    { file: "b.png", shot: "x" },
                    { file: "c.webp", clip: "y" },
                    { file: "d.webp", shot: "x", poster: "d-poster.webp" },
                ],
            }),
        ).toThrow(/App\.tsx" isn't a plain[\s\S]*exactly one[\s\S]*listed twice[\s\S]*b\.png" isn't a plain[\s\S]*a clip is published as \.mp4[\s\S]*only a clip has a poster/);
        expect(() => readManifest({})).toThrow(/must be an array/);
    });
});

describe("landingPlan", () => {
    it("finds the shot (at its size) and the clip's mp4, and skips what isn't publishable", () => {
        const steps = landingPlan(
            [
                { file: "ui-agent.webp", shot: "agent-conversation" },
                { file: "ui-tower.webp", shot: "widget-tower", size: "small" },
                { file: "ui-swarm-follows.mp4", clip: "swarm-follows-agent" },
                { file: "ui-toolchain.webp", shot: "widget-toolchain" },
                { file: "ui-none.webp", shot: "no-such-shot" },
                { file: "ui-tower-nosize.webp", shot: "widget-tower" },
            ],
            run,
        );
        // The run's files don't exist on disk here, so a found source is "missing" for that reason.
        expect(steps.map((s) => [s.entry.file, s.action, s.why ?? ""])).toEqual([
            ["ui-agent.webp", "missing", "shot agent-conversation is in the manifest but not on disk"],
            ["ui-tower.webp", "missing", "shot widget-tower (small) is in the manifest but not on disk"],
            ["ui-swarm-follows.mp4", "missing", "clip swarm-follows-agent (.mp4) is in the manifest but not on disk"],
            ["ui-toolchain.webp", "skipped", "shot widget-toolchain shows this machine's data, or isn't marked publishable"],
            ["ui-none.webp", "missing", "no run has shot no-such-shot"],
            // Taken at several sizes and none named: there is no plain widget-tower.png.
            ["ui-tower-nosize.webp", "missing", "no run has shot widget-tower"],
        ]);
    });

    it("refuses a shot two runs both have", () => {
        const twice = [...run, ...run];
        expect(landingPlan([{ file: "ui-agent.webp", shot: "agent-conversation" }], twice)[0].action).toBe("ambiguous");
    });
});

describe("encodeCommands", () => {
    it("a shot: one lossy WebP frame, scaled when a width is given", () => {
        const [c] = encodeCommands({ file: "ui-agent.webp", shot: "s", width: 960, quality: 80 }, "/run/s.png", "/tmp");
        expect(c.file).toBe("ui-agent.webp");
        expect(c.args.join(" ")).toContain("-vf scale=960:-2:flags=lanczos -frames:v 1 -c:v libwebp -quality 80");
        expect(c.args.at(-1)).toBe(join("/tmp", "ui-agent.webp"));
    });

    it("a clip: muted faststart H.264 with even dimensions, and its first frame as the poster", () => {
        const cmds = encodeCommands({ file: "c.mp4", clip: "c", width: 1280, poster: "c-poster.webp" }, "/run/c.mp4", "/tmp");
        expect(cmds.map((c) => c.file)).toEqual(["c.mp4", "c-poster.webp"]);
        const mp4 = cmds[0].args.join(" ");
        expect(mp4).toContain("-an -vf scale=1280:-2:flags=lanczos,scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p");
        expect(mp4).toContain("-c:v libx264");
        expect(mp4).toContain("-movflags +faststart");
        expect(cmds[1].args.join(" ")).toContain("-frames:v 1 -c:v libwebp");
    });
});

/** A landing clone and a run on disk, in a temp dir. */
function fixture(manifest) {
    const root = mkdtempSync(join(tmpdir(), "landing-test-"));
    const landing = join(root, "landing");
    const shots = join(landing, "public", "screenshots");
    mkdirSync(shots, { recursive: true });
    writeFileSync(join(shots, "manifest.json"), JSON.stringify(manifest));
    writeFileSync(join(shots, "ui-agent.webp"), "old");
    writeFileSync(join(shots, "not-in-manifest.webp"), "keep me");
    const runDir = join(root, "run");
    mkdirSync(runDir);
    writeFileSync(join(runDir, "04-agent-conversation.png"), "png bytes");
    writeFileSync(
        join(runDir, "manifest.json"),
        JSON.stringify({ shots: [{ id: "agent-conversation", file: "04-agent-conversation.png", containsWorkspaceData: "review" }] }),
    );
    return { landing, shots, runDir };
}

/** Stands in for ffmpeg: writes "encoded:<input>" to the output path (the last argument). */
const fakeFfmpeg = (_bin, args) => {
    writeFileSync(args.at(-1), `encoded:${args[args.indexOf("-i") + 1]}`);
    return { status: 0 };
};

describe("publishLanding", () => {
    const manifest = { images: [{ file: "ui-agent.webp", shot: "agent-conversation" }] };

    it("writes only the listed file, and leaves every other file alone", () => {
        const { landing, shots, runDir } = fixture(manifest);
        publishLanding({ landing, from: [runDir], ffmpeg: "ffmpeg", exec: fakeFfmpeg, log: () => {} });
        expect(readFileSync(join(shots, "ui-agent.webp"), "utf8")).toMatch(/^encoded:.*04-agent-conversation\.png$/);
        expect(readFileSync(join(shots, "not-in-manifest.webp"), "utf8")).toBe("keep me");
        expect(readdirSync(shots).sort()).toEqual(["manifest.json", "not-in-manifest.webp", "ui-agent.webp"]);
    });

    it("--dry-run writes nothing", () => {
        const { landing, shots, runDir } = fixture(manifest);
        const log = [];
        publishLanding({ landing, from: [runDir], ffmpeg: "ffmpeg", exec: fakeFfmpeg, dryRun: true, log: (l) => log.push(l) });
        expect(readFileSync(join(shots, "ui-agent.webp"), "utf8")).toBe("old");
        expect(log.join("\n")).toContain("would update  public");
    });

    it("refuses a clone without a manifest, and stops on an ffmpeg failure", () => {
        const { landing, runDir } = fixture(manifest);
        expect(() => publishLanding({ landing: join(landing, "nope"), from: [runDir], ffmpeg: "ffmpeg", log: () => {} })).toThrow(/agentmux-landing clone/);
        expect(() =>
            publishLanding({ landing, from: [runDir], ffmpeg: "ffmpeg", exec: () => ({ status: 1, stderr: "bad input" }), log: () => {} }),
        ).toThrow(/ffmpeg failed for ui-agent\.webp: bad input/);
    });

    // With a real ffmpeg (FFMPEG=<path>): a real PNG becomes a real WebP.
    it.skipIf(!process.env.FFMPEG)("encodes a real WebP with ffmpeg", () => {
        const { landing, shots, runDir } = fixture({ images: [{ file: "ui-agent.webp", shot: "agent-conversation", width: 64 }] });
        const png = join(runDir, "04-agent-conversation.png");
        const r = spawnSync(process.env.FFMPEG, ["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "color=c=blue:s=128x96", "-frames:v", "1", png]);
        expect(r.status).toBe(0);
        publishLanding({ landing, from: [runDir], ffmpeg: process.env.FFMPEG, log: () => {} });
        const out = readFileSync(join(shots, "ui-agent.webp"));
        expect(out.subarray(0, 4).toString()).toBe("RIFF");
        expect(out.subarray(8, 12).toString()).toBe("WEBP");
        expect(existsSync(join(shots, "not-in-manifest.webp"))).toBe(true);
    });
});
