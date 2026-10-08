// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The core invariant of SPEC_AGENT_PANE_ROW_DISCLOSURE_2026_09_26.md §5: for
 * every row type and state, what the row RENDERS as open equals
 * `rowDisclosureIn()`, the layout slice's `currentExpansion()` agrees, and
 * the `e` key flips the same set the rule names.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", async (importOriginal) => ({
    ...(await importOriginal<Record<string, unknown>>()),
    getApi: () => ({ showContextMenu: vi.fn(), openExternal: vi.fn() }),
}));

import type { AgentMessageNode, DocumentNode, DocumentState, MarkdownNode, ShellNode, ToolNode, UserMessageNode } from "../types";
import { DocumentRow } from "./DocumentRow";
import { rowDisclosureIn } from "./disclosure";
import { currentExpansion } from "./expansion-source";

afterEach(() => cleanup());

const docState = (s: { pinned?: string[]; collapsed?: string[]; held?: string[] } = {}): DocumentState => ({
    collapsedNodes: new Set(s.collapsed ?? []),
    pinnedNodes: new Set(s.pinned ?? []),
    heldOpenNodes: new Set(s.held ?? []),
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

const tool = (status: ToolNode["status"], toolName = "Bash"): ToolNode => ({
    type: "tool", id: "n", tool: toolName === "Bash" ? "Bash" : "Other", toolName, params: { command: "ls" }, status,
    collapsed: true, summary: "x", result: status === "running" ? undefined : ({ stdout: "ok" } as any),
});
const msg: AgentMessageNode = {
    type: "agent_message", id: "n", from: "a", to: "b", message: "hello", method: "mux", direction: "incoming",
    timestamp: 0, collapsed: false, summary: "📨 a → b",
};
const startup: UserMessageNode = { type: "user_message", id: "n", message: "# Session Context\nx", timestamp: 0, isStartup: true };
const canceled: MarkdownNode = { type: "markdown", id: "n", content: "partial", metadata: { canceled: true } };
const shell: ShellNode = {
    type: "shell", id: "n", cmd: "npm run dev", title: "dev", status: "exited-ok", exitCode: 0,
    spawnedAt: Date.now() - 5000, exitedAt: Date.now(), log: { chunks: [], open: false },
};

/** What the row actually shows as open, read from its DOM. */
function renderedOpen(node: DocumentNode, container: HTMLElement): boolean {
    switch (node.type) {
        case "tool":
        case "shell":
            return container.querySelector(".agent-tool-panel--flow") !== null;
        case "agent_message":
            return container.querySelector(".agent-message-content") !== null;
        case "user_message":
            return container.querySelector(".agent-user-message-content--flow") !== null;
        case "markdown":
            return container.querySelector(".markdown-canceled-header")?.getAttribute("aria-expanded") === "true";
        default:
            throw new Error(`no DOM probe for ${node.type}`);
    }
}

// prettier-ignore
const CASES: Array<[string, DocumentNode, DocumentState]> = [
    ["tool closed", tool("success"), docState()],
    ["tool pinned", tool("success"), docState({ pinned: ["n"] })],
    ["tool running", tool("running"), docState()],
    ["tool held", tool("success"), docState({ held: ["n"] })],
    ["tool failed + held", tool("failed"), docState({ held: ["n"] })],
    ["tool canceled + held", tool("canceled"), docState({ held: ["n"] })],
    ["tool denied + held", tool("denied"), docState({ held: ["n"] })],
    ["content-first open", tool("success", "WebSearch"), docState()],
    ["content-first collapsed", tool("success", "WebSearch"), docState({ collapsed: ["n"] })],
    ["agent message open", msg, docState()],
    ["agent message collapsed", msg, docState({ collapsed: ["n"] })],
    ["startup payload closed", startup, docState()],
    ["startup payload pinned", startup, docState({ pinned: ["n"] })],
    ["shell closed", shell, docState()],
    ["shell pinned", shell, docState({ pinned: ["n"] })],
    ["canceled thinking closed", canceled, docState()],
    ["canceled thinking opened", canceled, docState({ pinned: ["n"] })],
];

describe("disclosure parity: rendered = rule = layout slice", () => {
    for (const [name, node, s] of CASES) {
        it(name, () => {
            const [n] = createSignal<DocumentNode>(node);
            const [st] = createSignal<DocumentState>(s);
            const { container } = render(() => (
                <DocumentRow node={n} documentState={st} onToggleCollapse={() => {}} onTogglePin={() => {}} />
            ));
            const rule = rowDisclosureIn(node, s);
            expect(renderedOpen(node, container)).toBe(rule.open);
            expect(currentExpansion(node, s).open).toBe(rule.open);
        });
    }
});

describe("the `e` key flips the set the rule names", () => {
    const press = (node: DocumentNode, s: DocumentState) => {
        const onTogglePin = vi.fn();
        const onToggleCollapse = vi.fn();
        const [n] = createSignal<DocumentNode>(node);
        const [st] = createSignal<DocumentState>(s);
        const { container, unmount } = render(() => (
            <DocumentRow node={n} documentState={st} onToggleCollapse={onToggleCollapse} onTogglePin={onTogglePin} />
        ));
        fireEvent.keyDown(container.querySelector(".agent-document-row")!, { key: "e" });
        unmount();
        return { pin: onTogglePin.mock.calls.length, collapse: onToggleCollapse.mock.calls.length };
    };

    it("pins a panel tool, a shell, a startup payload and canceled thinking", () => {
        for (const node of [tool("success"), shell, startup, canceled] as DocumentNode[]) {
            expect(press(node, docState())).toEqual({ pin: 1, collapse: 0 });
        }
    });

    it("collapses an agent message and a content-first tool", () => {
        for (const node of [msg, tool("success", "WebSearch")] as DocumentNode[]) {
            expect(press(node, docState())).toEqual({ pin: 0, collapse: 1 });
        }
    });

    it("does nothing on a normal user message", () => {
        const normal: UserMessageNode = { type: "user_message", id: "n", message: "hi", timestamp: 0 };
        expect(press(normal, docState())).toEqual({ pin: 0, collapse: 0 });
    });
});

describe("canceled thinking opens through the pin", () => {
    it("its header click calls onTogglePin", () => {
        const onTogglePin = vi.fn();
        const [n] = createSignal<DocumentNode>(canceled);
        const [st] = createSignal<DocumentState>(docState());
        const { container } = render(() => (
            <DocumentRow node={n} documentState={st} onToggleCollapse={() => {}} onTogglePin={onTogglePin} />
        ));
        fireEvent.click(container.querySelector(".markdown-canceled-header")!);
        expect(onTogglePin).toHaveBeenCalledTimes(1);
    });
});
