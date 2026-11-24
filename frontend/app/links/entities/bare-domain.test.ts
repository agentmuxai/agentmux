// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const checkFileNow = vi.fn();
const openLink = vi.fn();
const openPathLink = vi.fn();
vi.mock("../file-check-store", () => ({ checkFileNow: (...a: unknown[]) => checkFileNow(...a) }));
vi.mock("@/app/store/misc-utils", () => ({ openLink: (...a: unknown[]) => openLink(...a) }));
vi.mock("@/app/element/link-destinations", () => ({ openPathLink: (...a: unknown[]) => openPathLink(...a) }));

import { isWebName, plainWhenMissing, siteOf } from "./bare-domain";
import { openFileEntity } from "./openers";

// [what is written, the site it is when no such file is there, or null]
const SITES: [string, string | null][] = [
    // The short list of well-known sites.
    ["docs.rs", "https://docs.rs"],
    ["lib.rs", "https://lib.rs"],
    ["rustup.rs", "https://rustup.rs"],
    ["bun.sh", "https://bun.sh"],
    ["astral.sh", "https://astral.sh"],
    ["Docs.rs", "https://Docs.rs"],
    // Any other name with those endings is not a site.
    ["pkg.sh", null],
    ["parser.rs", null],
    ["nope.rs", null],
    ["analyze.py", null],
    ["main.rs", null],
    ["findings.md", null],
    ["docs/docs.rs", null],
    ["./docs.rs", null],
    ["C:/docs.rs", null],
    ["a.tsx", null],
];

describe("a bare name that is a well-known site", () => {
    it.each(SITES)("%s", (written, site) => {
        expect(siteOf(written)).toBe(site);
    });
    it("is kept short", () => {
        expect(SITES.filter(([, site]) => site).length).toBeLessThan(10);
    });
});

// [what is written, whether it is plain text when the file check says there is no such file]
const PLAIN: [string, boolean][] = [
    ["parser.rs", true],
    ["nope.rs", true],
    ["pkg.sh", true],
    ["analyze.py", true],
    ["tool.pl", true],
    ["parse.ml", true],
    // A site, a file by its stem or its case or its characters, another ending, or a path: not plain.
    ["docs.rs", false],
    ["main.rs", false],
    ["setup.py", false],
    ["README.rs", false],
    ["lib_utils.rs", false],
    ["my-script.sh", false],
    ["gone.md", false],
    ["gone.ts", false],
    ["www.example.rs", false],
    ["src/parser.rs", false],
];

describe("a bare name with no such file is plain text, not a muted file and not a link", () => {
    it.each(PLAIN)("%s", (written, plain) => {
        expect(plainWhenMissing(written)).toBe(plain);
    });
});

describe("a name written with www.", () => {
    it.each([
        ["www.example.rs", true],
        ["www.pkg.sh", true],
        ["WWW.Example.py", true],
        ["example.rs", false],
        ["www.example.ts", false],
        ["www.example.md", false],
    ] as [string, boolean][])("%s", (written, web) => {
        expect(isWebName(written)).toBe(web);
    });
});

describe("opening it", () => {
    const dest = (path: string) => ({ kind: "file" as const, path, blockId: "pane-1", confined: false });
    beforeEach(() => {
        checkFileNow.mockReset();
        openLink.mockReset();
        openPathLink.mockReset();
    });

    it("opens the file when it is in the agent's folders", async () => {
        checkFileNow.mockResolvedValue({ state: "found", reason: "", resolved: "/w/docs.rs" });
        expect(await openFileEntity(dest("docs.rs"), {})).toEqual({ ok: true });
        expect(openPathLink).toHaveBeenCalledTimes(1);
        expect(openLink).not.toHaveBeenCalled();
    });

    it("opens the site when no such file is there", async () => {
        checkFileNow.mockResolvedValue({ state: "missing", reason: "Not found.", resolved: "" });
        expect(await openFileEntity(dest("docs.rs"), {})).toEqual({ ok: true });
        expect(openLink).toHaveBeenCalledWith("https://docs.rs");
        expect(openPathLink).not.toHaveBeenCalled();
    });

    it("says why, and does not open the site, when the check could not be made or the path is outside", async () => {
        checkFileNow.mockResolvedValueOnce({ state: "limited", reason: "The file couldn't be checked right now.", resolved: "" });
        expect(await openFileEntity(dest("parser.rs"), {})).toEqual({ ok: false, reason: "The file couldn't be checked right now." });
        checkFileNow.mockResolvedValueOnce({ state: "outside", reason: "Outside this agent's folders.", resolved: "" });
        expect(await openFileEntity(dest("parser.rs"), {})).toEqual({ ok: false, reason: "Outside this agent's folders." });
        expect(openLink).not.toHaveBeenCalled();
    });

    it("says why when a file name that is only a file is not there", async () => {
        checkFileNow.mockResolvedValue({ state: "missing", reason: "Not found.", resolved: "" });
        expect(await openFileEntity(dest("gone.md"), {})).toEqual({ ok: false, reason: "Not found." });
        expect(openLink).not.toHaveBeenCalled();
    });
});
