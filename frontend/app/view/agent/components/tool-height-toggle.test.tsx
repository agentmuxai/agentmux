// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The taller / shorter toggle on a tool preview, and when it is there
 * (SPEC_AGENT_PANE_TOOL_PEEK_ON_LINE_AND_MD_OPEN_IN_EDITOR_2026_10_11.md §3, §3.1):
 * only while the preview scrolls, or is already tall.
 *
 * jsdom does no layout, so the box's sizes are set by hand and a fake
 * ResizeObserver stands in for the browser telling the box it changed.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentPaneFilesProvider } from "../agent-open-in-editor";
import type { ToolNode } from "../types";
import { ToolBlock } from "./ToolBlock";

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { rpcCall: vi.fn() } }));
vi.mock("@/app/util/reveal-block", () => ({ showBlockWithoutFocus: vi.fn() }));

let roCallbacks: Array<() => void> = [];
class FakeResizeObserver {
    constructor(private cb: () => void) {
        roCallbacks.push(cb);
    }
    observe() {}
    unobserve() {}
    disconnect() {
        roCallbacks = roCallbacks.filter((c) => c !== this.cb);
    }
}
const resized = () => roCallbacks.slice().forEach((cb) => cb());

/** The scrolling box's sizes; mutate to model output growing or the box getting taller. */
const box = { scrollHeight: 100, clientHeight: 100 };

beforeEach(() => {
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.spyOn(HTMLDivElement.prototype, "scrollHeight", "get").mockImplementation(() => box.scrollHeight);
    vi.spyOn(HTMLDivElement.prototype, "clientHeight", "get").mockImplementation(() => box.clientHeight);
});

afterEach(() => {
    cleanup();
    roCallbacks = [];
    box.scrollHeight = 100;
    box.clientHeight = 100;
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    delete (Element.prototype as { getAnimations?: unknown }).getAnimations;
});

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

const readMd: ToolNode = {
    type: "tool",
    id: "r1",
    tool: "Read",
    toolName: "Read",
    params: { file_path: "C:/repo/docs/notes.md" },
    result: { content: "# Notes\n\nSome text." },
    status: "success",
    collapsed: false,
    summary: "",
};

const show = (node: ToolNode) =>
    render(() => (
        <AgentPaneFilesProvider files={() => ({ blockId: "agent-1", baseDir: "C:/repo" })}>
            <ToolBlock node={node} pinned={true} onTogglePin={() => {}} />
        </AgentPaneFilesProvider>
    ));

const toggle = (c: HTMLElement) => c.querySelector(".agent-tool-panel-actions .agent-tool-panel-height") as HTMLElement | null;
const openInEditor = (c: HTMLElement) =>
    c.querySelector(".agent-tool-panel-actions [aria-label='Open in editor']") as HTMLElement | null;

describe("the taller / shorter toggle on a tool preview", () => {
    it("is there when the preview scrolls", () => {
        box.scrollHeight = 500;
        const { container } = show(bash);
        resized();
        expect(toggle(container)).not.toBeNull();
    });

    it("isn't there when the whole preview fits", () => {
        const { container } = show(bash);
        resized();
        expect(toggle(container)).toBeNull();
        // And no empty actions strip left behind.
        expect(container.querySelector(".agent-tool-panel-actions")).toBeNull();
    });

    it("appears when output grows past the box, and goes when it shrinks back", () => {
        const { container } = show(bash);
        resized();
        expect(toggle(container)).toBeNull();
        box.scrollHeight = 500;
        resized();
        expect(toggle(container)).not.toBeNull();
        box.scrollHeight = 100;
        resized();
        expect(toggle(container)).toBeNull();
    });

    it("makes the preview three times as tall, and back", () => {
        box.scrollHeight = 900;
        const { container } = show(bash);
        resized();
        const panel = container.querySelector(".agent-tool-panel")!;
        expect(toggle(container)!.getAttribute("aria-label")).toBe("Taller preview");
        fireEvent.click(toggle(container)!);
        expect(panel.classList.contains("agent-tool-panel--tall")).toBe(true);
        expect(toggle(container)!.getAttribute("aria-label")).toBe("Shorter preview");
        fireEvent.click(toggle(container)!);
        expect(panel.classList.contains("agent-tool-panel--tall")).toBe(false);
    });

    it("stays while the preview is tall, even once everything fits, so it can shrink back", () => {
        box.scrollHeight = 500;
        box.clientHeight = 100;
        const { container } = show(bash);
        resized();
        fireEvent.click(toggle(container)!);
        // Tall: the box now shows all of it, nothing left to scroll.
        box.clientHeight = 500;
        resized();
        expect(toggle(container)).not.toBeNull();
        expect(toggle(container)!.getAttribute("aria-label")).toBe("Shorter preview");
        // Back to short: it scrolls again, so the toggle is still wanted.
        fireEvent.click(toggle(container)!);
        box.clientHeight = 100;
        resized();
        expect(toggle(container)!.getAttribute("aria-label")).toBe("Taller preview");
    });

    it("isn't kept once the preview is short again and fits", () => {
        box.scrollHeight = 500;
        box.clientHeight = 100;
        const { container } = show(bash);
        resized();
        fireEvent.click(toggle(container)!);
        fireEvent.click(toggle(container)!);
        box.scrollHeight = 100;
        resized();
        expect(toggle(container)).toBeNull();
    });

    it("waits out the panel opening: a box clipped mid-transition isn't a scrolling one", async () => {
        // Content that fits, read while the panel's max-height is still easing open.
        box.scrollHeight = 100;
        box.clientHeight = 20;
        let end!: () => void;
        const finished = new Promise<void>((r) => (end = r));
        let running = true;
        // jsdom has no getAnimations; the browser's reports the panel's max-height transition.
        Object.defineProperty(Element.prototype, "getAnimations", {
            configurable: true,
            value: () =>
                running ? [{ playState: "running", effect: { getComputedTiming: () => ({ endTime: 120 }) }, finished }] : [],
        });
        const { container } = show(bash);
        resized();
        expect(toggle(container)).toBeNull();
        running = false;
        box.clientHeight = 100;
        end();
        await finished;
        await Promise.resolve();
        await Promise.resolve();
        expect(toggle(container)).toBeNull();
    });

    it("a Markdown preview that fits keeps Open in editor, and just doesn't have the toggle", () => {
        const { container } = show(readMd);
        resized();
        expect(openInEditor(container)).not.toBeNull();
        expect(toggle(container)).toBeNull();
        box.scrollHeight = 500;
        resized();
        expect(openInEditor(container)).not.toBeNull();
        expect(toggle(container)).not.toBeNull();
    });
});
