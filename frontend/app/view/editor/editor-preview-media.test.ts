// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { previewMedia } from "./editor-preview-media";

describe("previewMedia", () => {
    it("resolves a local document's images against its folder", () => {
        expect(previewMedia("C:/work/docs/README.md", "", false)).toEqual({ baseDir: "C:/work/docs" });
    });

    it("reads no local file for a document on an SSH host", () => {
        expect(previewMedia("/home/u/README.md", "user@build-box", false)).toBeUndefined();
    });

    it("resolves a scratch tab's images locally, even in an editor on an SSH host", () => {
        expect(previewMedia("C:/scratch/notes.md", "user@build-box", true)).toEqual({ baseDir: "C:/scratch" });
    });
});
