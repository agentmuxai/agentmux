// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { decodeFileUri, workspaceCovers } from "./editor-lsp-paths";
import { lspBannerShows, lspStatusChipText } from "./editor-lsp-status";

describe("workspaceCovers", () => {
    it("is false with no root", () => {
        expect(workspaceCovers(null, "/a/b.ts")).toBe(false);
    });

    it("covers the root itself and what is under it, but not a sibling that shares its prefix", () => {
        expect(workspaceCovers("/work/app", "/work/app")).toBe(true);
        expect(workspaceCovers("/work/app", "/work/app/src/a.ts")).toBe(true);
        expect(workspaceCovers("/work/app/", "/work/app/src/a.ts")).toBe(true);
        expect(workspaceCovers("/work/app", "/work/app2/a.ts")).toBe(false);
        expect(workspaceCovers("/work/app", "/work/other/a.ts")).toBe(false);
    });

    it("is case-sensitive on POSIX paths", () => {
        expect(workspaceCovers("/work/App", "/work/app/a.ts")).toBe(false);
    });

    it("takes either separator on a Windows drive, and ignores case there", () => {
        expect(workspaceCovers("C:\\Work\\App", "c:/work/app/src/a.ts")).toBe(true);
        expect(workspaceCovers("c:/work/app", "C:\\WORK\\APP")).toBe(true);
        expect(workspaceCovers("C:\\Work\\App", "C:\\Work\\App2\\a.ts")).toBe(false);
        expect(workspaceCovers("C:\\Work\\App", "D:\\Work\\App\\a.ts")).toBe(false);
    });
});

describe("decodeFileUri", () => {
    it("turns a POSIX file URI into a path, decoding each segment", () => {
        expect(decodeFileUri("file:///home/u/my%20file.ts")).toBe("/home/u/my file.ts");
    });

    it("turns a Windows file URI into a path with backslashes and the drive first", () => {
        expect(decodeFileUri("file:///C:/Users/a/b%23c.ts")).toBe("C:\\Users\\a\\b#c.ts");
    });

    it("keeps a segment that is not valid percent-encoding as it is", () => {
        expect(decodeFileUri("file:///home/u/100%.ts")).toBe("/home/u/100%.ts");
    });

    it("takes a two-slash URI as host and path, and leaves a plain path alone", () => {
        expect(decodeFileUri("file://host/share/a.ts")).toBe("host/share/a.ts");
        expect(decodeFileUri("/already/a/path")).toBe("/already/a/path");
    });
});

describe("the language server's status", () => {
    it("words each state for the chip, and says nothing with none or once it is disposed", () => {
        expect(lspStatusChipText(null, "typescript")).toBe("");
        expect(lspStatusChipText({ kind: "starting" }, "typescript")).toBe("typescript: starting…");
        expect(lspStatusChipText({ kind: "initializing" }, "rust")).toBe("rust: initializing…");
        expect(lspStatusChipText({ kind: "ready", capabilities: {} as never }, "python")).toBe("python: ready");
        expect(lspStatusChipText({ kind: "missing", language: "python" }, "python")).toBe("python: not installed");
        expect(lspStatusChipText({ kind: "crashed", error: "x" }, "go")).toBe("go: error");
        expect(lspStatusChipText({ kind: "disposed" }, "go")).toBe("");
    });

    it("shows the install banner for a missing server until its language is dismissed", () => {
        const missing = { kind: "missing", language: "python" } as const;
        expect(lspBannerShows(missing, new Set())).toBe(true);
        expect(lspBannerShows(missing, new Set(["python"]))).toBe(false);
        expect(lspBannerShows(missing, new Set(["rust"]))).toBe(true);
        expect(lspBannerShows({ kind: "starting" }, new Set())).toBe(false);
        expect(lspBannerShows(null, new Set())).toBe(false);
    });
});
