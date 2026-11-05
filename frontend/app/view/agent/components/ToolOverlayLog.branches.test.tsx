// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The corners of ToolOverlayLog the main test file leaves out, pinned as ordered logs: a stream
 * that is open while a result already exists (it shows nothing), a different tool node reusing
 * the slot (the follow state and the scroll position start over), and the frames it asks for
 * when the host has no ResizeObserver. The main file (ToolOverlayLog.test.tsx) holds the rest. A
 * refactor of ToolOverlayLog must leave the committed snapshots as they are; regenerate with
 * `vitest -u` only for a change meant to behave differently.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ToolOverlayLog } from "./ToolOverlayLog";
import type { ToolNode } from "../../../agent-model/types";

vi.mock("@/app/motion/motion", () => ({ shouldReduceMotion: () => false }));
vi.mock("@/app/store/global", () => ({
    getSettingsKeyAtom: () => () => undefined,
    isDev: () => false,
}));

const lines: string[] = [];
const log = (line: string) => lines.push(line);
const out = () => lines.join("\n").replace(/\n+$/, "") + "\n";
const snap = (name: string) => `./__snapshots__/ToolOverlayLog.branches.${name}.txt`;

type ROCallback = () => void;
let roCallbacks: ROCallback[] = [];
class FakeResizeObserver {
    constructor(private cb: ROCallback) {
        roCallbacks.push(cb);
    }
    observe() {}
    unobserve() {}
    disconnect() {
        roCallbacks = roCallbacks.filter((c) => c !== this.cb);
    }
}
const fireResize = () => roCallbacks.forEach((cb) => cb());

beforeEach(() => {
    lines.length = 0;
});
afterEach(() => {
    cleanup();
    roCallbacks = [];
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

const bash = (id: string, over: Partial<ToolNode> = {}): ToolNode => ({
    type: "tool",
    id,
    tool: "Bash",
    params: { command: "make" },
    status: "running",
    collapsed: false,
    summary: "Bash make",
    log: { open: true, chunks: Array.from({ length: 40 }, (_, i) => ({ kind: "stdout", content: `line ${i + 1}`, timestamp: i + 1 })) },
    ...over,
});
const readNode: ToolNode = {
    type: "tool",
    id: "tc-read",
    tool: "Read",
    params: { file_path: "/tmp/a.ts" },
    status: "success",
    collapsed: false,
    summary: "Read /tmp/a.ts",
    result: { content: "const a = 1;\nconst b = 2;" } as any,
};

/** Give the scroller real-ish geometry; `g` is live. */
function withGeometry(el: HTMLElement, g: { scrollHeight: number; clientHeight: number }) {
    let top = 0;
    Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => g.scrollHeight });
    Object.defineProperty(el, "clientHeight", { configurable: true, get: () => g.clientHeight });
    Object.defineProperty(el, "scrollTop", {
        configurable: true,
        get: () => top,
        set: (v: number) => {
            top = Math.max(0, Math.min(v, g.scrollHeight - g.clientHeight));
            log(`scrollTop set ${v} -> ${top}`);
        },
    });
    return { bottom: () => g.scrollHeight - g.clientHeight, top: () => top };
}

describe("an open stream that already has a result", () => {
    it("shows nothing while the stream is open, and the result once it closes", async () => {
        const [node, setNode] = createSignal<ToolNode>(
            bash("tc-open", {
                status: "success",
                log: { open: true, chunks: [] },
                result: { exitCode: 0, stdout: "all done", stderr: "" } as any,
            }),
        );
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        const content = () => container.querySelector(".agent-tool-overlay-log-content")!;
        log(`open stream: html=${JSON.stringify(content().innerHTML)}`);
        setNode((n) => ({ ...n, log: { open: false, chunks: [] } }));
        log(`closed stream: text=${JSON.stringify(content().textContent)}`);
        await expect(out()).toMatchFileSnapshot(snap("open-stream-with-result"));
    });
});

describe("a different tool node reusing the slot", () => {
    it("starts its follow state and scroll position over, both ways", async () => {
        vi.stubGlobal("ResizeObserver", FakeResizeObserver);
        const [node, setNode] = createSignal<ToolNode>(bash("tc-a"));
        const { container } = render(() => <ToolOverlayLog node={node()} />);
        const el = container.querySelector(".agent-tool-overlay-log") as HTMLElement;
        const g = { scrollHeight: 1000, clientHeight: 200 };
        const geo = withGeometry(el, g);
        fireResize();
        log(`bash: top=${geo.top()} bottom=${geo.bottom()}`);
        setNode(readNode);
        log(`swapped to a Read: top=${geo.top()}`);
        g.scrollHeight = 1200;
        fireResize();
        log(`Read grows: top=${geo.top()}`);
        setNode(bash("tc-b"));
        fireResize();
        log(`swapped back to Bash: top=${geo.top()} bottom=${geo.bottom()}`);
        g.scrollHeight = 1400;
        fireResize();
        log(`Bash grows: top=${geo.top()} bottom=${geo.bottom()}`);
        await expect(out()).toMatchFileSnapshot(snap("node-swap"));
    });
});

describe("a host without ResizeObserver", () => {
    it("asks for a pin frame only while following, and a frame for the overflow check always", async () => {
        const frames: string[] = [];
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
            frames.push("frame");
            cb(0);
            return frames.length;
        });
        const [node, setNode] = createSignal<ToolNode>(bash("tc-a"));
        render(() => <ToolOverlayLog node={node()} />);
        log(`following bash: frames=${frames.length}`);
        frames.length = 0;
        setNode(readNode);
        log(`Read swapped in: frames=${frames.length}`);
        frames.length = 0;
        setNode({ ...readNode, summary: "Read /tmp/a.ts again" });
        log(`Read updated (not following): frames=${frames.length}`);
        await expect(out()).toMatchFileSnapshot(snap("no-resize-observer"));
    });
});
