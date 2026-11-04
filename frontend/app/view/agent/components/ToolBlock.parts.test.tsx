// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a ToolBlock row does, pinned as ordered logs and HTML: the live tail and its elapsed
 * timer, the result pill (including the dispatch-driven one for Agent, Task and Workflow), the
 * hold-open calls, the hover hold, the peek popover, the header variants, the panel's actions
 * (the preview's height toggle, Open in Editor), and the read-back of an unloaded result. The
 * panel's body is a stand-in that reports whether it is open and lets a test say that it
 * scrolls; everything else is the real component. A refactor of ToolBlock must leave the
 * committed snapshots as they are; regenerate with `vitest -u` only for a change meant to
 * behave differently.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ToolNode } from "../../../agent-model/types";
import type { AgentDispatch } from "../../swarm/swarm-model";

const h = vi.hoisted(() => ({
    log: [] as string[],
    overlay: null as null | { onOverflowChange: (scrolls: boolean) => void },
}));

vi.mock("./ToolBlockOverlay", async () => {
    const { createRenderEffect } = await import("solid-js");
    return {
        ToolBlockOverlay: (p: { node: { id: string }; open: boolean; dormant: boolean; onOverflowChange: (s: boolean) => void }) => {
            h.overlay = p;
            const el = document.createElement("div");
            el.className = "agent-tool-overlay";
            el.textContent = `body of ${p.node.id}`;
            createRenderEffect(() => {
                el.dataset.open = String(p.open);
                el.dataset.dormant = String(p.dormant);
            });
            return el;
        },
    };
});

import { ToolBlock } from "./ToolBlock";
import { BODY_LINGER_MS } from "./body-mount";
import { PEEK_ENTER_DELAY_MS } from "./hover-anchor";
import { AgentDormancyProvider } from "../agent-dormancy";
import { AgentPaneFilesProvider } from "../agent-open-in-editor";
import { ToolResultLoaderProvider } from "../tool-result-loader";
import { rememberBodyHeights } from "../body-height-cache";

const T0 = 1_800_000_000_000;
const baseTool: ToolNode = {
    type: "tool",
    id: "tc-1",
    tool: "Bash",
    params: { command: "ls -la" },
    status: "success",
    collapsed: true,
    summary: "Bash ls -la",
};
const dispatch = (over: Partial<AgentDispatch>): AgentDispatch =>
    ({ dispatch_id: "d1", kind: "solo", parent_agent: "a", parent_block_id: "b", session_id: "s", member_count: 3, members_done: 1, status: "running", last_event_at: T0, ...over }) as AgentDispatch;

/** The markup with one element per line, so a snapshot diff reads. */
const pretty = (html: string): string => html.replace(/></g, ">\n<") + "\n";
const out = () => h.log.join("\n").replace(/\n+$/, "") + "\n";
const snap = (name: string) => `./__snapshots__/ToolBlock.parts.${name}.txt`;
const noop = () => {};

beforeEach(() => {
    h.log.length = 0;
    h.overlay = null;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date", "performance"] });
    vi.setSystemTime(T0);
});
afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
    vi.useRealTimers();
});

const tailOf = (c: HTMLElement): string => c.querySelector(".agent-tool-live-tail")?.outerHTML ?? "(no live tail)";

describe("the live tail on a running row", () => {
    it("shows the last real output line, else an elapsed timer that counts and never goes negative", () => {
        const chunk = (kind: string, content: string) => ({ kind, content, timestamp: T0 }) as never;
        const run = (label: string, node: ToolNode) => {
            const [n, setN] = createSignal(node);
            const view = render(() => <ToolBlock node={n()} pinned={false} onTogglePin={noop} />);
            h.log.push(`${label}: ${tailOf(view.container)}`);
            return { view, setN };
        };
        run("log not open", { ...baseTool, status: "running", log: { open: false, chunks: [] } as never });
        const waiting = run("no chunks yet", { ...baseTool, status: "running", timestamp: T0, log: { open: true, chunks: [] } as never });
        vi.advanceTimersByTime(3000);
        h.log.push(`after 3 s: ${tailOf(waiting.view.container)}`);
        run("only a system chunk", { ...baseTool, id: "tc-2", status: "running", timestamp: T0 - 7000, log: { open: true, chunks: [chunk("system", "[bashwrap] starting")] } as never });
        run("clock stepped back", { ...baseTool, id: "tc-3", status: "running", timestamp: T0 + 90_000, log: { open: true, chunks: [] } as never });
        run("no stamp", { ...baseTool, id: "tc-4", status: "running", log: { open: true, chunks: [] } as never });
        const live = run("stdout then stderr", {
            ...baseTool, id: "tc-5", status: "running", log: { open: true, chunks: [chunk("stdout", "first"), chunk("system", "x"), chunk("stderr", "oops")] } as never,
        });
        h.log.push(`newest output wins: ${tailOf(live.view.container)}`);
        live.setN({ ...baseTool, id: "tc-5", status: "running", log: { open: true, chunks: [chunk("stdout", "first"), chunk("stdout", "second")] } as never });
        h.log.push(`a new chunk: ${tailOf(live.view.container)}`);
        return expect(out()).toMatchFileSnapshot(snap("live-tail"));
    });
});

describe("the result pill", () => {
    it("follows the status and result, and for Agent, Task and Workflow the matched dispatch", () => {
        const pillOf = (node: ToolNode, d?: AgentDispatch): string => {
            const view = render(() => <ToolBlock node={node} pinned={false} dispatchMatch={d} onTogglePin={noop} />);
            const pill = view.container.querySelector<HTMLElement>(".agent-tool-result-pill");
            const text = pill ? `${pill.className} | ${pill.textContent}` : "none";
            view.unmount();
            return text;
        };
        const withResult = { result: { content: "ok" } as never };
        for (const tool of ["Agent", "Task", "Workflow"] as const) {
            const base: ToolNode = { ...baseTool, tool, params: {}, summary: tool };
            h.log.push(`${tool} + solo dispatch running: ${pillOf({ ...base, status: "running" }, dispatch({ status: "running" }))}`);
            h.log.push(`${tool} + workflow dispatch running: ${pillOf({ ...base, status: "running" }, dispatch({ kind: "workflow", status: "running", members_done: 2, member_count: 5 }))}`);
            h.log.push(`${tool} + dispatch completed: ${pillOf({ ...base, status: "running" }, dispatch({ status: "completed" }))}`);
            h.log.push(`${tool} + dispatch abandoned: ${pillOf({ ...base, status: "success" }, dispatch({ status: "abandoned" }))}`);
            h.log.push(`${tool} no match, running: ${pillOf({ ...base, status: "running", ...withResult })}`);
            h.log.push(`${tool} no match, pending approval: ${pillOf({ ...base, status: "pending_approval", ...withResult })}`);
            h.log.push(`${tool} no match, success without result: ${pillOf({ ...base, status: "success" })}`);
            h.log.push(`${tool} no match, success with result: ${pillOf({ ...base, status: "success", ...withResult })}`);
            h.log.push(`${tool} no match, failed with result: ${pillOf({ ...base, status: "failed", ...withResult })}`);
        }
        const grep: ToolNode = { ...baseTool, tool: "Grep", toolName: "Grep", params: { pattern: "x" }, summary: "x", result: { content: "a.ts:1:x\nb.ts:2:x\n" } as never };
        h.log.push(`Grep success: ${pillOf(grep)}`);
        h.log.push(`Grep running: ${pillOf({ ...grep, status: "running" })}`);
        h.log.push(`Grep success, no result: ${pillOf({ ...grep, result: undefined })}`);
        const search: ToolNode = { ...baseTool, tool: "Other", toolName: "WebSearch", params: { query: "q" }, summary: "q", result: { content: 'Web search results for query: "q"\n\nLinks: [{"title":"A","url":"https://a.com"}]\n\nSummary.' } as never };
        h.log.push(`WebSearch success: ${pillOf(search)}`);
        h.log.push(`Bash running: ${pillOf({ ...baseTool, status: "running", ...withResult })}`);
        h.log.push(`Bash success, no result: ${pillOf(baseTool)}`);
        h.log.push(`Bash success with result: ${pillOf({ ...baseTool, ...withResult })}`);
        h.log.push(`Bash failed with result: ${pillOf({ ...baseTool, status: "failed", ...withResult })}`);
        const unloaded = (pill: unknown) => ({ ...baseTool, resultUnloaded: { pill, bytes: 10, tokens: 3 } }) as ToolNode;
        h.log.push(`unloaded keeps its pill: ${pillOf(unloaded({ label: "12 lines", variant: "ok" }))}`);
        h.log.push(`unloaded without a pill: ${pillOf(unloaded(null))}`);
        return expect(out()).toMatchFileSnapshot(snap("result-pill"));
    });
});

describe("holding a finished row open", () => {
    it("asks once when a running tool finishes, on arrival of a live finish, and not for loaded history", () => {
        const run = (label: string, make: (n: ToolNode, set: (n: ToolNode) => void) => void, start: ToolNode) => {
            const onHoldOpen = vi.fn();
            const [n, setN] = createSignal(start);
            render(() => <ToolBlock node={n()} pinned={false} onTogglePin={noop} onHoldOpen={onHoldOpen} />);
            const calls = [onHoldOpen.mock.calls.length];
            make(start, (next) => {
                setN(next);
                calls.push(onHoldOpen.mock.calls.length);
            });
            h.log.push(`${label}: calls after mount and each change = ${calls.join(", ")}`);
            cleanup();
        };
        const t = (status: ToolNode["status"], extra: Partial<ToolNode> = {}): ToolNode => ({ ...baseTool, status, ...extra });
        run("running then success", (_s, set) => set(t("success")), t("running"));
        run("pending approval, running, success", (_s, set) => { set(t("running")); set(t("success")); }, t("pending_approval"));
        run("running then failed", (_s, set) => set(t("failed")), t("running"));
        run("running then denied", (_s, set) => set(t("denied")), t("running"));
        run("running then canceled", (_s, set) => set(t("canceled")), t("running"));
        run("loaded success, then a note", (_s, set) => set(t("success", { statusNote: "n" })), t("success"));
        run("finished a second ago on arrival", () => {}, t("success", { timestamp: T0 - 1000 }));
        run("finished a minute ago on arrival", () => {}, t("success", { timestamp: T0 - 60_000 }));
        run("finished without a stamp on arrival", () => {}, t("success"));
        run("denied a second ago on arrival", () => {}, t("denied", { timestamp: T0 - 1000 }));
        run("running on arrival", () => {}, t("running", { timestamp: T0 - 1000 }));
        run("another node takes the slot, finished live", (_s, set) => set(t("success", { id: "tc-9", timestamp: T0 - 1000 })), t("running"));
        run("another node takes the slot, running then finished", (_s, set) => { set(t("running", { id: "tc-9" })); set(t("success", { id: "tc-9" })); }, t("success"));
        run("the slot's old status is not the new node's baseline", (_s, set) => set(t("success", { id: "tc-9" })), t("running", { id: "tc-1" }));
        return expect(out()).toMatchFileSnapshot(snap("hold-open"));
    });
});

describe("the hover hold", () => {
    it("keeps an open, pin-toggled row open while the pointer is in it, but not a content-first row or a closed one", () => {
        const classes = (c: HTMLElement) => c.querySelector(".agent-tool-block")!.className;
        const panel = (c: HTMLElement) => c.querySelector(".agent-tool-panel")!.className;
        const [pinned, setPinned] = createSignal(true);
        const a = render(() => <ToolBlock node={baseTool} pinned={pinned()} onTogglePin={noop} />);
        const block = a.container.querySelector(".agent-tool-block")!;
        h.log.push(`pinned: ${panel(a.container)}`);
        fireEvent.mouseEnter(block);
        setPinned(false);
        h.log.push(`unpinned with the pointer in: ${panel(a.container)}`);
        fireEvent.mouseLeave(block);
        h.log.push(`pointer left: ${panel(a.container)}`);
        a.unmount();

        const search: ToolNode = { ...baseTool, tool: "Other", toolName: "WebSearch", params: { query: "solid docs" }, summary: "WebSearch" };
        const [collapsed, setCollapsed] = createSignal(false);
        const b = render(() => <ToolBlock node={search} pinned={false} userCollapsed={collapsed()} onTogglePin={noop} onToggleCollapse={() => h.log.push("toggle collapse")} />);
        h.log.push(`content-first, open: ${panel(b.container)}`);
        fireEvent.mouseEnter(b.container.querySelector(".agent-tool-block")!);
        setCollapsed(true);
        h.log.push(`content-first, collapsed with the pointer in: ${panel(b.container)}`);
        fireEvent.click(b.container.querySelector(".agent-tool-summary")!);
        b.unmount();

        const c = render(() => (
            <div onClick={() => h.log.push("click reached the transcript row")}>
                <ToolBlock node={baseTool} pinned={false} onTogglePin={() => h.log.push("toggle pin")} />
            </div>
        ));
        fireEvent.mouseEnter(c.container.querySelector(".agent-tool-block")!);
        h.log.push(`closed row, pointer in: ${classes(c.container)} / ${panel(c.container)}`);
        fireEvent.click(c.container.querySelector(".agent-tool-summary")!);
        fireEvent.click(c.container.querySelector(".agent-tool-panel")!);
        h.log.push("(a click on the panel stops there; one on the summary goes on)");
        return expect(out()).toMatchFileSnapshot(snap("hover-hold"));
    });
});

describe("the peek popover", () => {
    it("names the tool with its command or detail, shows shell colours for Bash, and an estimate", () => {
        const peek = (label: string, node: ToolNode) => {
            const view = render(() => <ToolBlock node={node} pinned={false} onTogglePin={noop} />);
            const row = view.container.querySelector(".agent-tool-summary")!;
            fireEvent.mouseEnter(row);
            vi.advanceTimersByTime(PEEK_ENTER_DELAY_MS + 50);
            h.log.push(`${label}: ${pretty((document.body.querySelector(".agent-node-peek-overlay")?.outerHTML ?? "(no popover)").replace(/ style="[^"]*"/g, ""))}`);
            fireEvent.mouseLeave(row);
            vi.advanceTimersByTime(1000);
            h.log.push(`${label}, after leaving: ${document.body.querySelector(".agent-node-peek-overlay") ? "still shown" : "gone"}`);
            view.unmount();
        };
        peek("Bash", baseTool);
        peek("Read a path", { ...baseTool, tool: "Read", params: { file_path: "/a/b.ts" }, summary: "Read /a/b.ts" });
        peek("an MCP tool", { ...baseTool, tool: "Other", toolName: "mcp__srv__lookup", params: { q: "x" }, summary: "lookup" } as ToolNode);
        peek("nothing to show", { ...baseTool, tool: "Other", params: {}, summary: "x" });
        return expect(out()).toMatchFileSnapshot(snap("peek"));
    });
});

describe("the header", () => {
    it("draws the composed parts, an authored summary, the note, the duration and the status", () => {
        const draw = (label: string, node: ToolNode, pinned = false) => {
            const view = render(() => <ToolBlock node={node} pinned={pinned} onTogglePin={noop} />);
            h.log.push(`${label}:\n${pretty(view.container.querySelector(".agent-tool-block")!.outerHTML)}`);
            view.unmount();
        };
        draw("Bash command", baseTool);
        draw("Bash with a note and a duration", { ...baseTool, statusNote: "cleared by muxspect", duration: 1.26 });
        draw("Read a path with lines", { ...baseTool, tool: "Read", params: { file_path: "/a/b.ts", offset: 3, limit: 10 }, summary: "Read" });
        draw("Grep pattern", { ...baseTool, tool: "Grep", params: { pattern: "needle" }, summary: "Grep" });
        draw("running", { ...baseTool, status: "running" });
        draw("pending approval, pinned", { ...baseTool, status: "pending_approval" }, true);
        draw("failed", { ...baseTool, status: "failed" });
        draw("awaiting an answer", { ...baseTool, status: "awaiting_answer", tool: "Other", toolName: "AskUserQuestion", summary: "Waiting for your answer", statusNote: "ignored" } as ToolNode);
        return expect(out()).toMatchFileSnapshot(snap("header"));
    });
});

describe("the panel's actions", () => {
    const md: ToolNode = { ...baseTool, tool: "Write", params: { file_path: "/notes/a.md", content: "# hi" }, summary: "Write", status: "success" };
    const actions = (c: HTMLElement) => c.querySelector(".agent-tool-panel-actions")?.outerHTML ?? "(no actions)";
    const panel = (c: HTMLElement) => c.querySelector(".agent-tool-panel")!.className;

    it("offers Open in Editor for a Markdown file, and a height toggle only for a preview that scrolls or is tall", () => {
        const [pinned, setPinned] = createSignal(true);
        const view = render(() => <ToolBlock node={baseTool} pinned={pinned()} onTogglePin={noop} />);
        h.log.push(`fits: ${actions(view.container)}`);
        h.overlay!.onOverflowChange(true);
        h.log.push(`scrolls: ${pretty(actions(view.container))}`);
        const toggle = () => view.container.querySelector<HTMLElement>(".agent-tool-panel-height")!;
        const stopped = fireEvent.click(toggle());
        h.log.push(`taller (the click stops there: ${!stopped}): ${panel(view.container)}`);
        h.log.push(pretty(actions(view.container)));
        h.overlay!.onOverflowChange(false);
        h.log.push(`fits again but tall: ${actions(view.container) === "(no actions)" ? "toggle gone" : "toggle stays"}`);
        fireEvent.click(toggle());
        h.log.push(`shorter: ${panel(view.container)} / ${actions(view.container)}`);
        h.overlay!.onOverflowChange(true);
        fireEvent.click(toggle());
        setPinned(false);
        h.log.push(`closed while tall: ${panel(view.container)}`);
        vi.advanceTimersByTime(BODY_LINGER_MS + 50);
        h.log.push(`after the linger: ${view.container.querySelector(".agent-tool-overlay") ? "body kept" : "body gone"}, ${panel(view.container)}`);
        setPinned(true);
        h.log.push(`reopened: ${actions(view.container)}`);
        h.overlay!.onOverflowChange(true);
        h.log.push(`reports again: ${panel(view.container)} / ${actions(view.container) === "(no actions)" ? "no toggle" : "toggle"}`);
        return expect(out()).toMatchFileSnapshot(snap("panel-actions-height"));
    });

    it("hands the preview's open state and dormancy to the body, and the Markdown path to Open in Editor", () => {
        const [dormant, setDormant] = createSignal(false);
        const [pinned, setPinned] = createSignal(true);
        const files = () => ({ blockId: "agent-block", baseDir: "/work" });
        const view = render(() => (
            <AgentPaneFilesProvider files={files}>
                <AgentDormancyProvider dormant={dormant}>
                    <ToolBlock node={md} pinned={pinned()} onTogglePin={noop} />
                </AgentDormancyProvider>
            </AgentPaneFilesProvider>
        ));
        const body = () => view.container.querySelector<HTMLElement>(".agent-tool-overlay");
        h.log.push(`open: open=${body()?.dataset.open} dormant=${body()?.dataset.dormant}`);
        h.log.push(`Markdown write: ${pretty(actions(view.container))}`);
        setDormant(true);
        h.log.push(`dormant: open=${body()?.dataset.open} dormant=${body()?.dataset.dormant}`);
        setDormant(false);
        setPinned(false);
        h.log.push(`closing: open=${body()?.dataset.open}`);
        vi.advanceTimersByTime(BODY_LINGER_MS + 50);
        h.log.push(`closed: ${body() ? "body kept" : "body gone"}; ${actions(view.container)}`);
        const read: ToolNode = { ...baseTool, tool: "Read", params: { file_path: "/notes/b.md" }, result: { content: "# text" } as never };
        const other = render(() => (
            <AgentPaneFilesProvider files={files}>
                <ToolBlock node={read} pinned={true} onTogglePin={noop} />
            </AgentPaneFilesProvider>
        ));
        h.log.push(`Markdown read: ${pretty(actions(other.container))}`);
        const noFiles = render(() => <ToolBlock node={md} pinned={true} onTogglePin={noop} />);
        h.log.push(`outside an agent pane: ${actions(noFiles.container)}`);
        return expect(out()).toMatchFileSnapshot(snap("panel-actions-markdown"));
    });
});

describe("reading an unloaded result back", () => {
    const unloaded: ToolNode = { ...baseTool, resultUnloaded: { pill: null, bytes: 4_000, tokens: 1_000 } };
    const note = (c: HTMLElement) => c.querySelector(".agent-tool-result-unloaded")?.outerHTML ?? "(no placeholder)";
    const flush = async () => {
        for (let i = 0; i < 4; i++) await Promise.resolve();
    };

    it("shows Loading, then a failure message that a later open retries, and loads once per open", async () => {
        const results = [false, true];
        const load = vi.fn(async () => {
            h.log.push("load");
            return results.shift() ?? true;
        });
        const [held, setHeld] = createSignal(true);
        const view = render(() => (
            <ToolResultLoaderProvider load={load}>
                <ToolBlock node={unloaded} pinned={false} heldOpen={held()} onTogglePin={noop} />
            </ToolResultLoaderProvider>
        ));
        h.log.push(`while loading: ${note(view.container)}`);
        await flush();
        h.log.push(`failed: ${note(view.container)}`);
        h.log.push(`loads so far: ${load.mock.calls.length}`);
        setHeld(false);
        await flush();
        h.log.push(`closed: loads=${load.mock.calls.length}`);
        setHeld(true);
        await flush();
        h.log.push(`reopened: loads=${load.mock.calls.length} ${note(view.container)}`);
        return expect(out()).toMatchFileSnapshot(snap("unloaded-read-back"));
    });

    it("does not read back without a loader, and keeps the open body's measured height for the placeholder", () => {
        const height = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
        Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
            configurable: true,
            get() {
                return (this as HTMLElement).classList.contains("agent-tool-overlay") ? 321 : 0;
            },
        });
        try {
            const bare = render(() => <ToolBlock node={unloaded} pinned={false} heldOpen={true} onTogglePin={noop} />);
            h.log.push(`no loader: ${note(bare.container)}`);
            bare.unmount();

            const loaded: ToolNode = { ...baseTool, result: { content: "ok" } as never };
            const [node, setNode] = createSignal<ToolNode>(loaded);
            const view = render(() => (
                <div data-node-id="tc-1">
                    <ToolResultLoaderProvider load={async () => true}>
                        <ToolBlock node={node()} pinned={false} heldOpen={true} onTogglePin={noop} />
                    </ToolResultLoaderProvider>
                </div>
            ));
            rememberBodyHeights(view.container, ["tc-1"]);
            setNode(unloaded);
            h.log.push(`height from the batched pass: ${note(view.container)}`);
            view.unmount();

            const view2 = render(() => (
                <ToolResultLoaderProvider load={async () => true}>
                    <ToolBlock node={node()} pinned={false} heldOpen={true} onTogglePin={noop} />
                </ToolResultLoaderProvider>
            ));
            setNode(loaded);
            setNode(unloaded);
            h.log.push(`height read from the body: ${note(view2.container)}`);
            setNode(loaded);
            h.log.push(`result back: ${note(view2.container)} / ${view2.container.querySelector(".agent-tool-overlay") ? "body" : "no body"}`);
            const gone = render(() => <ToolBlock node={loaded} pinned={false} heldOpen={true} onTogglePin={noop} />);
            gone.unmount();
            h.log.push(`row removed: no placeholder, no error`);
        } finally {
            if (height) Object.defineProperty(HTMLElement.prototype, "offsetHeight", height);
        }
        return expect(out()).toMatchFileSnapshot(snap("unloaded-height"));
    });
});
