// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Open in editor" on a Markdown file preview
 * (SPEC_AGENT_PANE_TOOL_PEEK_ON_LINE_AND_MD_OPEN_IN_EDITOR_2026_10_11.md §2).
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import type { JSX } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    rpcCall: vi.fn(async (..._args: unknown[]) => ({})),
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { rpcCall: h.rpcCall } }));

import { AgentPaneProviders } from "../../agent-media";
import { AgentPaneFilesProvider, resolveAgentPath } from "../../agent-open-in-editor";
import type { ToolNode } from "../../types";
import { ToolBlock } from "../ToolBlock";

afterEach(() => {
    cleanup();
    h.rpcCall.mockClear();
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
const heightToggle = (container: HTMLElement) =>
    container.querySelector(".agent-tool-panel-actions .agent-tool-panel-height") as HTMLElement | null;

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

    it("isn't there for a container agent, whose paths are inside its container", () => {
        const pane = (agentType: string) =>
            inPane(read("/work/notes.md"), (block) => (
                <AgentPaneProviders
                    dormant={() => false}
                    block={() => ({ oid: "agent-1", meta: { "cmd:cwd": "/work" } })}
                    agent={() => ({ working_directory: "/work", agent_type: agentType })}
                >
                    {block()}
                </AgentPaneProviders>
            ));
        expect(button(pane("container").container)).toBeNull();
        cleanup();
        expect(button(pane("host").container)).not.toBeNull();
    });
});

describe("Taller / shorter preview", () => {
    const bash: ToolNode = {
        type: "tool",
        id: "b1",
        tool: "Bash",
        toolName: "Bash",
        params: { command: "ls" },
        result: { stdout: "a\nb\nc", stderr: "", exitCode: 0 },
        status: "success",
        collapsed: false,
        summary: "",
    };

    it("every tool's preview has it; a Markdown preview has Open in editor beside it", () => {
        const plain = inPane(bash);
        expect(heightToggle(plain.container)).not.toBeNull();
        expect(button(plain.container)).toBeNull();
        cleanup();
        const md = inPane(read("C:/repo/docs/notes.md"));
        expect(heightToggle(md.container)).not.toBeNull();
        expect(button(md.container)).not.toBeNull();
    });

    it("makes the preview three times as tall, and back", () => {
        const { container } = inPane(bash);
        const panel = container.querySelector(".agent-tool-panel")!;
        const toggle = heightToggle(container)!;
        expect(toggle.getAttribute("aria-label")).toBe("Taller preview");
        fireEvent.click(toggle);
        expect(panel.classList.contains("agent-tool-panel--tall")).toBe(true);
        expect(heightToggle(container)!.getAttribute("aria-label")).toBe("Shorter preview");
        fireEvent.click(heightToggle(container)!);
        expect(panel.classList.contains("agent-tool-panel--tall")).toBe(false);
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
