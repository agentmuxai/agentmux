// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const checkFileNow = vi.fn();
const openLink = vi.fn();
const openPathLink = vi.fn();
vi.mock("../file-check-store", () => ({ checkFileNow: (...a: unknown[]) => checkFileNow(...a) }));
vi.mock("@/app/store/misc-utils", () => ({ openLink: (...a: unknown[]) => openLink(...a) }));
vi.mock("@/app/element/link-destinations", () => ({ openPathLink: (...a: unknown[]) => openPathLink(...a) }));

import { siteOf } from "./bare-domain";
import { openFileEntity } from "./openers";

// [what is written, the site it may be, or null when it is only ever a file]
const CASES: [string, string | null][] = [
    ["docs.rs", "https://docs.rs"],
    ["pkg.sh", "https://pkg.sh"],
    ["www.example.rs", "https://www.example.rs"],
    ["rustup.rs", "https://rustup.rs"],
    // A name that is a file whatever is on disk.
    ["findings.md", null],
    ["notes.md", null],
    ["main.rs", null],
    ["setup.py", null],
    ["run.sh", null],
    ["README.rs", null],
    ["lib_utils.rs", null],
    ["my-script.sh", null],
    ["a.tsx", null],
    // A path, a drive path, a line, or a name with a folder is never a site.
    ["docs/docs.rs", null],
    ["./docs.rs", null],
    ["C:/docs.rs", null],
];

describe("a bare name that may also be a web site", () => {
    it.each(CASES)("%s", (written, site) => {
        expect(siteOf(written)).toBe(site);
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
