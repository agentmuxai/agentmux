// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which viewer a file opens in, from srv's probe, and what "Reopen with…"
 * offers. SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.2.
 */

import { describe, expect, it } from "vitest";
import { chooseViewer, droppedViewerOptions, isProgramPath, TEXT_MAX_BYTES, viewerForDropped, viewerOptions, type FileProbe } from "./viewer-choice";

const probe = (kind: string, size = 1000, extra: Partial<FileProbe> = {}): FileProbe => ({
    size,
    mtime: 0,
    kind,
    mime: "application/octet-stream",
    read_only: false,
    ...(kind === "text" || kind === "svg" ? { text: { encoding: "utf-8", bom: "none" } } : {}),
    ...extra,
});

describe("chooseViewer", () => {
    it("opens each kind in its viewer", () => {
        expect(chooseViewer(probe("text"))).toBe("text");
        expect(chooseViewer(probe("svg"))).toBe("image");
        expect(chooseViewer(probe("image"))).toBe("image");
        expect(chooseViewer(probe("audio"))).toBe("media");
        expect(chooseViewer(probe("video"))).toBe("media");
        expect(chooseViewer(probe("pdf"))).toBe("pdf");
        // A kind with a name and no viewer yet: the card, which says what it is.
        for (const kind of ["archive", "sqlite", "font", "word", "image_file"]) {
            expect(chooseViewer(probe(kind)), kind).toBe("info");
        }
        // Binary of no known kind: its bytes.
        expect(chooseViewer(probe("other"))).toBe("hex");
    });

    it("never loads a text file over the editor's cap into CodeMirror", () => {
        expect(chooseViewer(probe("text", TEXT_MAX_BYTES + 1))).toBe("info");
        expect(chooseViewer(probe("text", TEXT_MAX_BYTES + 1), "text")).toBe("info");
        // Hex reads it in parts, whatever its size.
        expect(chooseViewer(probe("text", TEXT_MAX_BYTES + 1), "hex")).toBe("hex");
        expect(chooseViewer(probe("other", 5_000_000_000))).toBe("hex");
    });

    it("honours the user's choice when the file can be shown that way", () => {
        expect(chooseViewer(probe("svg"), "text")).toBe("text");
        expect(chooseViewer(probe("other"), "text")).toBe("text");
        expect(chooseViewer(probe("image"), "info")).toBe("info");
        for (const kind of ["text", "svg", "image", "video", "pdf", "archive", "other"]) {
            expect(chooseViewer(probe(kind), "hex"), kind).toBe("hex");
        }
        // Not a way the file can be shown: the default instead.
        expect(chooseViewer(probe("image"), "media")).toBe("image");
        expect(chooseViewer(probe("text"), "bogus")).toBe("text");
    });
});

describe("viewerOptions", () => {
    it("offers the default first, then the others", () => {
        expect(viewerOptions(probe("text"))).toEqual(["text", "info", "hex"]);
        expect(viewerOptions(probe("svg"))).toEqual(["image", "text", "info", "hex"]);
        expect(viewerOptions(probe("video"))).toEqual(["media", "info", "hex"]);
        expect(viewerOptions(probe("image"))).toEqual(["image", "info", "text", "hex"]);
        expect(viewerOptions(probe("pdf"))).toEqual(["pdf", "info", "text", "hex"]);
        expect(viewerOptions(probe("archive"))).toEqual(["info", "text", "hex"]);
        expect(viewerOptions(probe("other"))).toEqual(["hex", "info", "text"]);
        // Too large for the editor: no text option.
        expect(viewerOptions(probe("other", TEXT_MAX_BYTES + 1))).toEqual(["hex", "info"]);
        expect(viewerOptions(probe("text", TEXT_MAX_BYTES + 1))).toEqual(["info", "hex"]);
    });
});

describe("viewerForDropped", () => {
    it("shows dropped bytes by name or type", () => {
        expect(viewerForDropped("shot.png", "")).toBe("image");
        expect(viewerForDropped("blob", "image/webp")).toBe("image");
        expect(viewerForDropped("clip.mp4", "")).toBe("media");
        expect(viewerForDropped("voice", "audio/ogg")).toBe("media");
        expect(viewerForDropped("app.exe", "application/x-msdownload")).toBe("info");
        expect(viewerForDropped("paper.pdf", "")).toBe("pdf");
    });

    it("keeps the user's choice of the card or of hex, and no other", () => {
        expect(viewerForDropped("shot.png", "", "hex")).toBe("hex");
        expect(viewerForDropped("shot.png", "", "info")).toBe("info");
        expect(viewerForDropped("app.exe", "", "hex")).toBe("hex");
        // Not a way dropped bytes are shown: the file's own viewer.
        expect(viewerForDropped("shot.png", "", "text")).toBe("image");
        expect(viewerForDropped("shot.png", "", "media")).toBe("image");
    });

    it("offers a dropped file its own viewer, the card and hex", () => {
        expect(droppedViewerOptions("shot.png", "")).toEqual(["image", "info", "hex"]);
        expect(droppedViewerOptions("paper.pdf", "")).toEqual(["pdf", "info", "hex"]);
        expect(droppedViewerOptions("app.exe", "")).toEqual(["info", "hex"]);
    });
});

describe("isProgramPath", () => {
    it("is true for what the system would run", () => {
        expect(isProgramPath("C:/x/setup.exe")).toBe(true);
        expect(isProgramPath("/home/u/build.SH")).toBe(true);
        expect(isProgramPath("C:/x/photo.png")).toBe(false);
        expect(isProgramPath("C:/x/exe")).toBe(false);
    });
});
