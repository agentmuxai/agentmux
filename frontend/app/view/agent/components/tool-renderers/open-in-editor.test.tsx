// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Open in editor" on a Markdown file preview
 * (SPEC_AGENT_PANE_TOOL_PEEK_ON_LINE_AND_MD_OPEN_IN_EDITOR_2026_10_11.md §2).
 */

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import type { JSX } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    rpcCall: vi.fn(async (..._args: unknown[]) => ({ block_id: "editor-9" })),
    show: vi.fn(async (_blockId: string) => true),
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { rpcCall: h.rpcCall } }));
vi.mock("@/app/util/reveal-block", () => ({ showBlockWithoutFocus: h.show }));

import { AgentPaneProviders } from "../../agent-media";
import { AgentPaneFilesProvider, resolveAgentPath } from "../../agent-open-in-editor";
import type { ToolNode } from "../../types";
import { ToolBlock } from "../ToolBlock";

afterEach(() => {
    cleanup();
    h.rpcCall.mockClear();
    h.show.mockClear();
});

const read = (path: string): ToolNode => ({
    type: "tool",
    id: "r1",
    tool: "Read",
    toolName: "Read",
    params: { file_path: path },
    result: { content: "# Notes\n\nSome text." },
    status: "success",
    collapsed: false,
    summary: "",
});

const write = (path: string): ToolNode => ({
    type: "tool",
    id: "w1",
    tool: "Write",
    toolName: "Write",
    params: { file_path: path, content: "# Plan\n\nSteps." },
    result: { bytesWritten: 15 },
    status: "success",
    collapsed: false,
    summary: "",
});

// `block` is a function, so the ToolBlock is created inside the wrapper (and
// its context): Solid creates a component where its JSX is evaluated.
const inPane = (node: ToolNode, wrap?: (block: () => JSX.Element) => JSX.Element) =>
    render(() => {
        const block = () => <ToolBlock node={node} pinned={true} onTogglePin={() => {}} />;
        return wrap ? (
            wrap(block)
        ) : (
            <AgentPaneFilesProvider files={() => ({ blockId: "agent-1", baseDir: "C:/repo" })}>{block()}</AgentPaneFilesProvider>
        );
    });

const button = (container: HTMLElement) =>
    container.querySelector(".agent-tool-panel-actions [aria-label='Open in editor']") as HTMLElement | null;

describe("Open in editor on a Markdown file preview", () => {
    it("a Read of a .md file opens it in the tab's editor, or a new one to the right", () => {
        const { container } = inPane(read("C:/repo/docs/notes.md"));
        const b = button(container);
        expect(b).not.toBeNull();
        fireEvent.click(b!);
        expect(h.rpcCall).toHaveBeenCalledWith(
            "pane.open",
            {
                view: "editor",
                file: "C:/repo/docs/notes.md",
                split_direction: "right",
                split_reference_block_id: "agent-1",
                // srv puts it in the editor the tab shows, if any; a new pane's
                // tree is collapsed by srv's Markdown default, and an explicit
                // tree_expanded would stop the reuse.
                reuse_editor_pane: true,
            },
            {}
        );
    });

    it("a Write of a relative .md path opens it under the pane's working directory", () => {
        const { container } = inPane(write("docs/plan.md"));
        fireEvent.click(button(container)!);
        expect(h.rpcCall.mock.calls[0][1]).toMatchObject({ file: "C:/repo/docs/plan.md" });
    });

    it("isn't on a file that isn't Markdown", () => {
        const { container } = inPane(read("C:/repo/src/main.ts"));
        expect(container.querySelector(".agent-tool-read-md")).toBeNull();
        expect(button(container)).toBeNull();
    });

    it("isn't there outside an agent pane", () => {
        const { container } = inPane(read("C:/repo/docs/notes.md"), (block) => block());
        expect(container.querySelector(".agent-tool-read-md")).not.toBeNull();
        expect(button(container)).toBeNull();
    });

    it("is only there for an agent known to run on this host", () => {
        const pane = (agentMode: string | undefined, agentType: string | undefined) =>
            inPane(read("/work/notes.md"), (block) => (
                <AgentPaneProviders
                    dormant={() => false}
                    block={() => ({ oid: "agent-1", meta: { "cmd:cwd": "/work", ...(agentMode ? { agentMode } : {}) } })}
                    agent={() => (agentType ? { working_directory: "/work", agent_type: agentType } : undefined)}
                >
                    {block()}
                </AgentPaneProviders>
            ));
        const shown = (mode: string | undefined, type: string | undefined) => {
            const has = button(pane(mode, type).container) != null;
            cleanup();
            return has;
        };
        // A container agent's paths are inside its container.
        expect(shown("container", undefined)).toBe(false);
        expect(shown(undefined, "container")).toBe(false);
        // The pane's own agentMode wins over a definition that hasn't loaded.
        expect(shown("host", undefined)).toBe(true);
        expect(shown(undefined, "host")).toBe(true);
        // Neither known yet (definitions still loading, or deleted): no button.
        expect(shown(undefined, undefined)).toBe(false);
    });

    it("shows the editor it opened, so a magnified agent pane doesn't hide it", async () => {
        const { container } = inPane(read("C:/repo/docs/notes.md"));
        fireEvent.click(button(container)!);
        await waitFor(() => expect(h.show).toHaveBeenCalledWith("editor-9"));
    });
});

describe("resolveAgentPath", () => {
    it("keeps an absolute path", () => {
        expect(resolveAgentPath("/home/a/x.md", "/work")).toBe("/home/a/x.md");
        expect(resolveAgentPath("C:\\repo\\x.md", "D:/other")).toBe("C:\\repo\\x.md");
        expect(resolveAgentPath("C:/repo/x.md", "D:/other")).toBe("C:/repo/x.md");
        expect(resolveAgentPath("\\\\server\\share\\x.md", "D:/other")).toBe("\\\\server\\share\\x.md");
        expect(resolveAgentPath("~/x.md", "/work")).toBe("~/x.md");
    });

    it("puts a relative path under the working directory, in its separator", () => {
        expect(resolveAgentPath("docs/x.md", "/work/")).toBe("/work/docs/x.md");
        expect(resolveAgentPath("x.md", "C:\\repo")).toBe("C:\\repo\\x.md");
        expect(resolveAgentPath("x.md", "")).toBe("x.md");
    });
});
