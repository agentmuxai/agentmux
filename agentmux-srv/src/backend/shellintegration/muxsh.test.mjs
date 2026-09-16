// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for muxsh.mjs's argument parsing, request-body building, and
// output rendering — pure, no network, no running instance. Same vitest
// arrangement as muxopen.test.mjs.

import { describe, expect, it } from "vitest";

import {
    buildRequestBody,
    guessView,
    parseArgs,
    renderAgentList,
    renderAgentSend,
    renderResult,
    renderShellCreate,
    renderShellStatus,
    renderShellStop,
    renderTabs,
} from "./muxsh.mjs";

describe("muxsh parseArgs — open/web/edit", () => {
    it("no arguments is help with a failing exit code", () => {
        expect(parseArgs([])).toEqual({ help: true, exitCode: 1 });
    });

    it("explicit help exits 0", () => {
        expect(parseArgs(["help"])).toEqual({ help: true, exitCode: 0 });
        expect(parseArgs(["--help"])).toEqual({ help: true, exitCode: 0 });
    });

    it("an unknown command is rejected", () => {
        expect(parseArgs(["bogus", "foo"]).error).toMatch(/unknown command 'bogus'/);
    });

    it("'open' with a bare file parses with defaults", () => {
        expect(parseArgs(["open", "/tmp/foo.md"])).toEqual({
            command: "open",
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
        expect(parseArgs(["open"]).error).toMatch(/requires a .*file path/);
    });

    it("a flag in the file position is an error rather than a bogus target", () => {
        expect(parseArgs(["open", "--floating"]).error).toMatch(/requires a .*file path/);
    });

    it("'open' accepts --title/--split/--collapse-tree/--floating/--no-focus", () => {
        expect(
            parseArgs(["open", "/tmp/foo.md", "--title", "Notes", "--split", "down", "--collapse-tree", "--floating", "--no-focus"]),
        ).toEqual({
            command: "open",
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
            command: "web",
            subcommand: "web",
            url: "https://example.com",
            title: null,
            split: null,
            collapseTree: false,
            floating: false,
            focus: true,
        });
    });

    it("'web' with no url is an error", () => {
        expect(parseArgs(["web"]).error).toMatch(/requires a .*url/);
    });

    it("'web' accepts --split (it's not editor-only)", () => {
        expect(parseArgs(["web", "https://example.com", "--split", "right"])).toEqual({
            command: "web",
            subcommand: "web",
            url: "https://example.com",
            title: null,
            split: "right",
            collapseTree: false,
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

    it("'edit' parses like 'open' (command/subcommand both 'open')", () => {
        expect(parseArgs(["edit", "/tmp/foo.md"])).toEqual({
            command: "open",
            subcommand: "open",
            file: "/tmp/foo.md",
            title: null,
            split: null,
            collapseTree: false,
            floating: false,
            focus: true,
        });
    });

    it("'edit' with no file is an error", () => {
        expect(parseArgs(["edit"]).error).toMatch(/requires a .*file path/);
    });
});

describe("muxsh guessView", () => {
    it("a URL guesses browser", () => {
        expect(guessView("https://example.com")).toBe("browser");
        expect(guessView("ftp://example.com/x")).toBe("browser");
    });

    it("a media extension guesses media", () => {
        expect(guessView("/tmp/photo.PNG")).toBe("media");
        expect(guessView("/tmp/clip.mp4")).toBe("media");
        expect(guessView("/tmp/report.pdf")).toBe("media");
    });

    it("anything else guesses editor", () => {
        expect(guessView("/tmp/notes.md")).toBe("editor");
        expect(guessView("/tmp/no-extension")).toBe("editor");
    });
});

describe("muxsh parseArgs — view", () => {
    it("a url dispatches to the 'web' shape", () => {
        expect(parseArgs(["view", "https://example.com"])).toEqual({
            command: "web",
            subcommand: "web",
            url: "https://example.com",
            title: null,
            split: null,
            collapseTree: false,
            floating: false,
            focus: true,
        });
    });

    it("a plain file dispatches to the 'open' shape with view='editor'", () => {
        expect(parseArgs(["view", "/tmp/notes.md"])).toEqual({
            command: "open",
            subcommand: "open",
            file: "/tmp/notes.md",
            view: "editor",
            title: null,
            split: null,
            collapseTree: false,
            floating: false,
            focus: true,
        });
    });

    it("a media file dispatches to the 'open' shape with view='media'", () => {
        expect(parseArgs(["view", "/tmp/photo.png"])).toMatchObject({ command: "open", view: "media" });
    });

    it("'view' with no target is an error", () => {
        expect(parseArgs(["view"]).error).toMatch(/requires a file path or url/);
    });
});

describe("muxsh parseArgs — pane list", () => {
    it("bare 'pane list' parses with json=false", () => {
        expect(parseArgs(["pane", "list"])).toEqual({ command: "pane-list", json: false });
    });

    it("'pane list --json' sets json=true", () => {
        expect(parseArgs(["pane", "list", "--json"])).toEqual({ command: "pane-list", json: true });
    });

    it("an unknown 'pane' verb is rejected", () => {
        expect(parseArgs(["pane", "close"]).error).toMatch(/unknown 'muxsh pane' verb 'close'/);
    });
});

describe("muxsh parseArgs — run", () => {
    it("'run <cmd>' joins remaining args with spaces", () => {
        expect(parseArgs(["run", "ls", "-la", "/tmp"])).toEqual({ command: "run-create", cmd: "ls -la /tmp" });
    });

    it("'run' with no command is an error", () => {
        expect(parseArgs(["run"]).error).toMatch(/requires a command/);
    });

    it("'run --status <id>' parses", () => {
        expect(parseArgs(["run", "--status", "shell-1"])).toEqual({ command: "run-status", shellId: "shell-1" });
    });

    it("'run --stop <id>' parses", () => {
        expect(parseArgs(["run", "--stop", "shell-1"])).toEqual({ command: "run-stop", shellId: "shell-1" });
    });

    it("'run --status' with no id is an error", () => {
        expect(parseArgs(["run", "--status"]).error).toMatch(/requires a shell id/);
    });
});

describe("muxsh parseArgs — config", () => {
    it("'config edit' parses", () => {
        expect(parseArgs(["config", "edit"])).toEqual({ command: "config-edit" });
    });

    it("bare 'config path' defaults to kind='data'", () => {
        expect(parseArgs(["config", "path"])).toEqual({ command: "config-path", kind: "data" });
    });

    it("'config path logs' parses the explicit kind", () => {
        expect(parseArgs(["config", "path", "logs"])).toEqual({ command: "config-path", kind: "logs" });
    });

    it("an invalid 'config path' kind is rejected", () => {
        expect(parseArgs(["config", "path", "bogus"]).error).toMatch(/unknown 'muxsh config path' kind 'bogus'/);
    });

    it("an unknown 'config' verb is rejected", () => {
        expect(parseArgs(["config", "bogus"]).error).toMatch(/unknown 'muxsh config' verb 'bogus'/);
    });
});

describe("muxsh parseArgs — agent", () => {
    it("bare 'agent list' parses with json=false", () => {
        expect(parseArgs(["agent", "list"])).toEqual({ command: "agent-list", json: false });
    });

    it("'agent list --json' sets json=true", () => {
        expect(parseArgs(["agent", "list", "--json"])).toEqual({ command: "agent-list", json: true });
    });

    it("'agent send <name> <message...>' joins the message with spaces", () => {
        expect(parseArgs(["agent", "send", "Scouto", "hello", "there"])).toEqual({
            command: "agent-send",
            name: "Scouto",
            message: "hello there",
        });
    });

    it("'agent send' with no name is an error", () => {
        expect(parseArgs(["agent", "send"]).error).toMatch(/requires an agent name/);
    });

    it("'agent send <name>' with no message is an error", () => {
        expect(parseArgs(["agent", "send", "Scouto"]).error).toMatch(/requires a message/);
    });

    it("an unknown 'agent' verb is rejected", () => {
        expect(parseArgs(["agent", "bogus"]).error).toMatch(/unknown 'muxsh agent' verb 'bogus'/);
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

    it("'view' dispatched to media sends view='media', not 'editor'", () => {
        expect(buildRequestBody(parseArgs(["view", "/tmp/photo.png"]), {})).toEqual({
            focus: true,
            floating: false,
            view: "media",
            file: "/tmp/photo.png",
        });
    });
});

describe("muxsh renderResult", () => {
    it("prints the pane view, block id, and tab id", () => {
        const out = renderResult({ block_id: "b-123", tab_id: "t-456", view: "editor", created: true });
        expect(out).toBe("opened: editor pane\n  block b-123\n  tab   t-456");
    });
});

describe("muxsh renderTabs", () => {
    it("says 'no tabs found' for an empty list", () => {
        expect(renderTabs([])).toBe("no tabs found");
    });

    it("renders a tab with its panes indented underneath", () => {
        const out = renderTabs([
            {
                tab_id: "tab-1",
                tab_name: "Tab 1",
                active: true,
                panes: [{ block_id: "b-1", view: "editor", title: "notes.md" }],
            },
        ]);
        expect(out).toBe("tab Tab 1 (active)\n  editor  b-1  notes.md");
    });
});

describe("muxsh renderShellCreate/Status/Stop", () => {
    it("renderShellCreate prints the shell_id", () => {
        expect(renderShellCreate({ shell_id: "s-1" })).toBe("shell_id: s-1");
    });

    it("renderShellStatus prints running + line_count, and exit_code only when present", () => {
        expect(renderShellStatus({ running: true, line_count: 12 })).toBe("running: true\nline_count: 12");
        expect(renderShellStatus({ running: false, exit_code: 0, line_count: 12 })).toBe(
            "running: false\nexit_code: 0\nline_count: 12",
        );
    });

    it("renderShellStop prints stopped", () => {
        expect(renderShellStop({ stopped: true })).toBe("stopped: true");
    });
});

describe("muxsh renderAgentList/Send", () => {
    it("says 'no agents found' for an empty list", () => {
        expect(renderAgentList({ host: { agents: [] } })).toBe("no agents found");
    });

    it("marks addressable agents with a leading '*'", () => {
        const out = renderAgentList({
            host: { agents: [{ name: "Scouto", addressable: true }, { name: "Idle1", addressable: false }] },
        });
        expect(out).toBe("* Scouto\n  Idle1  (not addressable)");
    });

    it("renderAgentSend reports success", () => {
        expect(renderAgentSend({ success: true })).toBe("sent");
    });

    it("renderAgentSend reports the error on failure", () => {
        expect(renderAgentSend({ success: false, error: "agent not found" })).toBe("not sent: agent not found");
    });
});
