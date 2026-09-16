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
            split: null,
            floating: false,
            focus: true,
        });
    });

    it("'web' with no url is an error", () => {
        expect(parseArgs(["web"]).error).toMatch(/requires a url/);
    });

    it("'web' accepts --split (it's not editor-only)", () => {
        expect(parseArgs(["web", "https://example.com", "--split", "right"])).toEqual({
            subcommand: "web",
            url: "https://example.com",
            title: null,
            split: "right",
            floating: false,
            focus: true,
        });
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
    it("with no AGENTMUX_BLOCKID in env, 'open' sends no split fields even if --split was passed", () => {
        // resolve_placement (server/app_api/pane.rs) always falls back to plain
        // "insert" without a split_reference_block_id, regardless of
        // split_direction — so sending split_direction alone would be
        // misleading; omit both together.
        expect(buildRequestBody(parseArgs(["open", "/tmp/foo.md", "--split", "down"]), {})).toEqual({
            focus: true,
            floating: false,
            view: "editor",
            file: "/tmp/foo.md",
        });
    });

    it("with AGENTMUX_BLOCKID in env, 'open' defaults split_direction to 'right' and sets split_reference_block_id", () => {
        expect(buildRequestBody(parseArgs(["open", "/tmp/foo.md"]), { AGENTMUX_BLOCKID: "b-1" })).toEqual({
            focus: true,
            floating: false,
            view: "editor",
            file: "/tmp/foo.md",
            split_direction: "right",
            split_reference_block_id: "b-1",
        });
    });

    it("an explicit --split overrides the 'right' default", () => {
        expect(
            buildRequestBody(parseArgs(["open", "/tmp/foo.md", "--split", "down"]), { AGENTMUX_BLOCKID: "b-1" }),
        ).toMatchObject({ split_direction: "down", split_reference_block_id: "b-1" });
    });

    it("AGENTMUX_TABID in env sets tab_id regardless of block id", () => {
        expect(
            buildRequestBody(parseArgs(["open", "/tmp/foo.md"]), { AGENTMUX_BLOCKID: "b-1", AGENTMUX_TABID: "t-1" }),
        ).toMatchObject({ tab_id: "t-1", split_reference_block_id: "b-1" });
    });

    it("--floating omits split fields even with a known block id (server ignores them when floating)", () => {
        expect(
            buildRequestBody(parseArgs(["open", "/tmp/foo.md", "--floating"]), { AGENTMUX_BLOCKID: "b-1" }),
        ).toEqual({
            focus: true,
            floating: true,
            view: "editor",
            file: "/tmp/foo.md",
        });
    });

    it("--title and --collapse-tree still round-trip", () => {
        expect(
            buildRequestBody(parseArgs(["open", "/tmp/foo.md", "--title", "Notes", "--collapse-tree"]), {}),
        ).toEqual({
            focus: true,
            floating: false,
            title: "Notes",
            view: "editor",
            file: "/tmp/foo.md",
            tree_expanded: false,
        });
    });

    it("'web' defaults produce view=browser with no optional fields when no block id is known", () => {
        expect(buildRequestBody(parseArgs(["web", "https://example.com"]), {})).toEqual({
            focus: true,
            floating: false,
            view: "browser",
            url: "https://example.com",
        });
    });

    it("'web' also defaults split_direction to 'right' when a block id is known", () => {
        expect(
            buildRequestBody(parseArgs(["web", "https://example.com"]), { AGENTMUX_BLOCKID: "b-1" }),
        ).toEqual({
            focus: true,
            floating: false,
            view: "browser",
            url: "https://example.com",
            split_direction: "right",
            split_reference_block_id: "b-1",
        });
    });
});

describe("muxsh renderResult", () => {
    it("prints the pane view, block id, and tab id", () => {
        const out = renderResult({ block_id: "b-123", tab_id: "t-456", view: "editor", created: true });
        expect(out).toBe("opened: editor pane\n  block b-123\n  tab   t-456");
    });
});
