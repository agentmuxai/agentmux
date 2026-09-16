// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for muxsh.mjs's argument parsing, request-body building, and
// output rendering — pure, no network, no running instance. Same vitest
// arrangement as muxopen.test.mjs.

import { describe, expect, it } from "vitest";

import { buildRequestBody, parseArgs, renderResult } from "./muxsh.mjs";

describe("muxsh parseArgs", () => {
    it("no arguments is help with a failing exit code", () => {
        expect(parseArgs([])).toEqual({ help: true, exitCode: 1 });
    });

    it("explicit help exits 0", () => {
        expect(parseArgs(["help"])).toEqual({ help: true, exitCode: 0 });
        expect(parseArgs(["--help"])).toEqual({ help: true, exitCode: 0 });
    });

    it("an unknown subcommand is rejected", () => {
        expect(parseArgs(["view", "foo"]).error).toMatch(/unknown subcommand 'view'/);
    });

    it("'open' with a bare file parses with defaults", () => {
        expect(parseArgs(["open", "/tmp/foo.md"])).toEqual({
            subcommand: "open",
            file: "/tmp/foo.md",
            title: null,
            split: null,
            collapseTree: false,
            floating: false,
            focus: true,
        });
    });

    it("'open' with no file is an error, not a silent undefined", () => {
        expect(parseArgs(["open"]).error).toMatch(/requires a file path/);
    });

    it("a flag in the file position is an error rather than a bogus target", () => {
        expect(parseArgs(["open", "--floating"]).error).toMatch(/requires a file path/);
    });

    it("'open' accepts --title/--split/--collapse-tree/--floating/--no-focus", () => {
        expect(
            parseArgs(["open", "/tmp/foo.md", "--title", "Notes", "--split", "down", "--collapse-tree", "--floating", "--no-focus"]),
        ).toEqual({
            subcommand: "open",
            file: "/tmp/foo.md",
            title: "Notes",
            split: "down",
            collapseTree: true,
            floating: true,
            focus: false,
        });
    });

    it("--split rejects an invalid direction", () => {
        expect(parseArgs(["open", "/tmp/foo.md", "--split", "sideways"]).error).toMatch(/must be one of right\/left\/down\/up/);
    });

    it("--split without a value is an error", () => {
        expect(parseArgs(["open", "/tmp/foo.md", "--split"]).error).toMatch(/--split requires/);
    });

    it("'web' with a bare url parses with defaults", () => {
        expect(parseArgs(["web", "https://example.com"])).toEqual({
            subcommand: "web",
            url: "https://example.com",
            title: null,
            floating: false,
            focus: true,
        });
    });

    it("'web' with no url is an error", () => {
        expect(parseArgs(["web"]).error).toMatch(/requires a url/);
    });

    it("'web' rejects editor-only flags: --split", () => {
        expect(parseArgs(["web", "https://example.com", "--split", "right"]).error).toMatch(
            /'--split' is only valid with 'muxsh open'/,
        );
    });

    it("'web' rejects editor-only flags: --collapse-tree", () => {
        expect(parseArgs(["web", "https://example.com", "--collapse-tree"]).error).toMatch(
            /'--collapse-tree' is only valid with 'muxsh open'/,
        );
    });

    it("an unknown flag is rejected loudly", () => {
        expect(parseArgs(["open", "/tmp/foo.md", "--wat"]).error).toMatch(/unknown argument '--wat'/);
    });
});

describe("muxsh buildRequestBody", () => {
    it("'open' defaults produce view=editor with no optional fields", () => {
        expect(buildRequestBody(parseArgs(["open", "/tmp/foo.md"]))).toEqual({
            focus: true,
            floating: false,
            view: "editor",
            file: "/tmp/foo.md",
        });
    });

    it("'open' with all options sets every optional field", () => {
        expect(
            buildRequestBody(
                parseArgs(["open", "/tmp/foo.md", "--title", "Notes", "--split", "down", "--collapse-tree", "--floating"]),
            ),
        ).toEqual({
            focus: true,
            floating: true,
            title: "Notes",
            view: "editor",
            file: "/tmp/foo.md",
            split_direction: "down",
            tree_expanded: false,
        });
    });

    it("'web' defaults produce view=browser with no optional fields", () => {
        expect(buildRequestBody(parseArgs(["web", "https://example.com"]))).toEqual({
            focus: true,
            floating: false,
            view: "browser",
            url: "https://example.com",
        });
    });
});

describe("muxsh renderResult", () => {
    it("prints the pane view, block id, and tab id", () => {
        const out = renderResult({ block_id: "b-123", tab_id: "t-456", view: "editor", created: true });
        expect(out).toBe("opened: editor pane\n  block b-123\n  tab   t-456");
    });
});
