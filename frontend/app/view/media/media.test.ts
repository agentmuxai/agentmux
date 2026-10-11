// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { basenameOf, createMediaDropHook, dirnameOf, extOf, mediaDropVerdict, mediaPaneTab, mediaSplitBlockDef, mediaTitle } from "./media";

describe("extOf", () => {
    it("returns the lowercase extension without a dot", () => {
        expect(extOf("clips/shot-06.WEBM")).toBe("webm");
        expect(extOf("/a/b/c.png")).toBe("png");
    });

    it("returns empty string when there's no extension", () => {
        expect(extOf("clips/README")).toBe("");
        expect(extOf("")).toBe("");
    });

    it("uses the last dot for a multi-dot filename", () => {
        expect(extOf("shot.v2.final.mp4")).toBe("mp4");
    });
});

describe("dirnameOf", () => {
    it("strips the last posix segment", () => {
        expect(dirnameOf("/home/user/clips/shot.webm")).toBe("/home/user/clips");
    });

    it("strips the last windows segment", () => {
        expect(dirnameOf("C:\\Users\\user\\clips\\shot.webm")).toBe("C:\\Users\\user\\clips");
    });

    it("returns empty string when there's no separator", () => {
        expect(dirnameOf("shot.webm")).toBe("");
    });
});

describe("basenameOf", () => {
    it("returns the last posix segment", () => {
        expect(basenameOf("/home/user/clips/shot.webm")).toBe("shot.webm");
    });

    it("returns the last windows segment", () => {
        expect(basenameOf("C:\\Users\\user\\clips\\shot.webm")).toBe("shot.webm");
    });

    it("returns the path unchanged when there's no separator", () => {
        expect(basenameOf("shot.webm")).toBe("shot.webm");
    });
});

// Media as a native pane tab (Pane Tab contract Phase 2c).
describe("mediaPaneTab", () => {
    it("is a native pane tab", () => {
        expect(mediaPaneTab.view).toBe("media");
        expect(mediaPaneTab.create).toBeTypeOf("function");
    });

    it("titles the pane with the picked file's name, or Media before one is picked", () => {
        expect(mediaTitle({ "media:path": "C:\\clips\\demo.mp4" } as any)).toBe("demo.mp4");
        expect(mediaTitle({ "media:path": "/home/me/cat.png" } as any)).toBe("cat.png");
        expect(mediaTitle({} as any)).toBe("Media");
        expect(mediaTitle(undefined)).toBe("Media");
    });
});

describe("media pane file drop", () => {
    const drag = (names?: string[], types: string[] = []) => ({ count: names?.length ?? types.length, types, names });

    it("accepts one supported image, video or audio file", () => {
        expect(mediaDropVerdict(drag(["C:/pics/cat.PNG"]))).toMatchObject({ ok: true, message: "Open here" });
        expect(mediaDropVerdict(drag(["clip.mp4"]))).toMatchObject({ ok: true });
        expect(mediaDropVerdict(drag(["tone.wav"]))).toMatchObject({ ok: true });
    });

    it("blocks several files, and types the pane can't show", () => {
        expect(mediaDropVerdict(drag(["a.png", "b.png"]))).toEqual({
            ok: false,
            reason: "Media panes open one image, video or audio file",
        });
        expect(mediaDropVerdict(drag(["notes.txt"]))).toMatchObject({ ok: false });
        expect(mediaDropVerdict(drag(["movie.mkv"]))).toMatchObject({ ok: false });
    });

    it("uses the MIME type before names are known, and lets an unknown type through", () => {
        expect(mediaDropVerdict(drag(undefined, ["image/png"]))).toMatchObject({ ok: true });
        expect(mediaDropVerdict(drag(undefined, ["image/bmp"]))).toMatchObject({ ok: false });
        expect(mediaDropVerdict(drag(undefined, ["application/pdf"]))).toMatchObject({ ok: false });
        expect(mediaDropVerdict(drag(undefined, [""]))).toMatchObject({ ok: true, message: "Open here" });
    });

    it("a drop with a path shows that path; without one, the file itself", async () => {
        const actions = { showPath: vi.fn(), showFile: vi.fn(), cantOpen: vi.fn() };
        const hook = createMediaDropHook(actions);
        await hook.drop({ paths: ["C:/pics/cat.png"], files: [] });
        expect(actions.showPath).toHaveBeenCalledWith("C:/pics/cat.png");
        const file = new File(["x"], "clip.webm", { type: "video/webm" });
        await hook.drop({ paths: [], files: [file] });
        expect(actions.showFile).toHaveBeenCalledWith(file);
        expect(actions.cantOpen).not.toHaveBeenCalled();
    });

    it("a dropped file it can't show is reported, not shown", async () => {
        const actions = { showPath: vi.fn(), showFile: vi.fn(), cantOpen: vi.fn() };
        await createMediaDropHook(actions).drop({ paths: ["C:/docs/report.pdf"], files: [] });
        expect(actions.showPath).not.toHaveBeenCalled();
        expect(actions.cantOpen).toHaveBeenCalledWith("report.pdf");
    });
});

// SPEC_EDITOR_MEDIA_SPLIT_OPENS_EMPTY_2026_10_10.md
describe("splitting a media pane", () => {
    it("opens an empty media pane, not a copy of this one's files", () => {
        expect(mediaPaneTab.capabilities?.splitBlockDef).toBe(mediaSplitBlockDef);
        const source = {
            oid: "b1",
            meta: { view: "media", doctabs: { tabs: [{ path: "/a.png" }] }, "media:path": "/a.png", "media:open": [{ id: "1", path: "/b.png" }] },
        } as unknown as Block;
        expect(mediaPaneTab.capabilities!.splitBlockDef!(source)).toEqual({ meta: { view: "media" } });
    });
});
