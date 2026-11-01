// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { classifyHref } from "./link-target";

describe("classifyHref", () => {
    it("takes http(s) links as web pages", () => {
        expect(classifyHref("https://github.com/a5af")).toEqual({ kind: "web", url: "https://github.com/a5af" });
        expect(classifyHref("HTTP://example.com")).toEqual({ kind: "web", url: "HTTP://example.com" });
    });

    it("takes absolute paths as local files", () => {
        expect(classifyHref("C:/Users/me/notes.md")).toEqual({ kind: "file", path: "C:/Users/me/notes.md" });
        expect(classifyHref(String.raw`C:\Users\me\notes.md`)).toEqual({ kind: "file", path: String.raw`C:\Users\me\notes.md` });
        // The markdown pipeline percent-encodes a backslash.
        expect(classifyHref("C:%5CUsers%5Cme%5Cnotes.md")).toEqual({ kind: "file", path: String.raw`C:\Users\me\notes.md` });
        expect(classifyHref("/home/me/notes.md")).toEqual({ kind: "file", path: "/home/me/notes.md" });
        expect(classifyHref("~/notes.md")).toEqual({ kind: "file", path: "~/notes.md" });
        expect(classifyHref("~")).toEqual({ kind: "file", path: "~" });
        expect(classifyHref(String.raw`\\server\share\x.md`)).toEqual({ kind: "file", path: String.raw`\\server\share\x.md` });
        expect(classifyHref("/home/me/a%20b.md")).toEqual({ kind: "file", path: "/home/me/a b.md" });
    });

    it("takes file: URLs as local files, drive letters and spaces included", () => {
        expect(classifyHref("file:///C:/Users/me/a%20b.md")).toEqual({ kind: "file", path: "C:/Users/me/a b.md" });
        expect(classifyHref("file:///home/me/notes.md")).toEqual({ kind: "file", path: "/home/me/notes.md" });
        expect(classifyHref("file://localhost/home/me/notes.md")).toEqual({ kind: "file", path: "/home/me/notes.md" });
        expect(classifyHref("file://server/share/x.md")).toEqual({ kind: "file", path: "//server/share/x.md" });
        expect(classifyHref("file:///home/me/notes.md#section")).toEqual({ kind: "file", path: "/home/me/notes.md" });
        expect(classifyHref("file:")).toEqual({ kind: "blocked" });
    });

    it("takes agentmux://task/<n> as a task, and no other agentmux: URL as anything to open", () => {
        expect(classifyHref("agentmux://task/14995")).toEqual({ kind: "task", num: 14995 });
        expect(classifyHref("agentmux://task/12/")).toEqual({ kind: "task", num: 12 });
        for (const href of [
            "agentmux://task/",
            "agentmux://task/12/close",
            "agentmux://task/12?do=cancel",
            "agentmux://task/abc",
            "agentmux://agents/x/stop",
            "agentmux:task/12",
        ]) {
            expect(classifyHref(href)).toEqual({ kind: "blocked" });
        }
    });

    it("leaves everything else to the system handler", () => {
        for (const href of ["mailto:a@b.co", "ssh://host", "vscode://file/x", "docs/readme.md", "//cdn.example.com/x.js", "#top", "", "  "]) {
            expect(classifyHref(href)).toEqual({ kind: "other" });
        }
        expect(classifyHref(undefined)).toEqual({ kind: "other" });
    });
});
