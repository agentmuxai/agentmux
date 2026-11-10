// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PersistentShellBlock — the row itself: the elapsed clock, the live tail, the status sigil and
 * classes, the pin-to-open panel, the Stop button, the log and the peek's inputs. The peek's own
 * show/hide and the log's linger are in PersistentShellBlock.test.tsx.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createEffect, createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ShellNode } from "../../../agent-model/types";

const ShellStopCommand = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ShellStopCommand: (...a: unknown[]) => ShellStopCommand(...a) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: { client: true } }));

// The peek overlay is Portal-rendered and positioned; here only what the row hands it matters.
const overlay = { show: false, rowEl: undefined as HTMLElement | undefined };
vi.mock("./PeekOverlay", () => ({
    PeekOverlay: (props: { show: boolean; rowEl: () => HTMLElement | undefined; children: JSX.Element }) => {
        createEffect(() => {
            overlay.show = props.show;
            overlay.rowEl = props.rowEl();
        });
        return (
            <div class="test-peek" data-show={props.show ? "yes" : "no"}>
                {props.show ? props.children : null}
            </div>
        );
    },
}));

import { PersistentShellBlock } from "./PersistentShellBlock";

const T0 = new Date("2026-11-08T12:00:00Z").getTime();

const shell = (over: Partial<ShellNode> = {}): ShellNode => ({
    type: "shell",
    id: "sh-1",
    cmd: "npm run dev",
    title: "dev server",
    status: "running",
    spawnedAt: T0 - 5_000,
    log: { chunks: [], open: false },
    ...over,
});

const mount = (node: ShellNode, pinned = false, onTogglePin: () => void = () => {}) =>
    render(() => <PersistentShellBlock node={node} pinned={pinned} onTogglePin={onTogglePin} />);

const q = (c: HTMLElement, sel: string) => c.querySelector<HTMLElement>(sel);
const isInert = (el: HTMLElement) =>
    el.hasAttribute("inert") || (el as HTMLElement & { inert?: boolean }).inert === true;
const text = (c: HTMLElement, sel: string) => q(c, sel)?.textContent;
const hover = (container: HTMLElement) => {
    fireEvent.mouseEnter(q(container, ".agent-shell-summary")!);
    vi.advanceTimersByTime(100);
};

beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(T0);
    ShellStopCommand.mockReset();
    ShellStopCommand.mockResolvedValue({});
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("PersistentShellBlock — the elapsed clock", () => {
    it("a running shell counts up every second", () => {
        const { container } = mount(shell());
        expect(text(container, ".agent-shell-elapsed")).toBe("[0:05]");
        vi.advanceTimersByTime(1000);
        expect(text(container, ".agent-shell-elapsed")).toBe("[0:06]");
        vi.advanceTimersByTime(999);
        expect(text(container, ".agent-shell-elapsed")).toBe("[0:06]");
        vi.advanceTimersByTime(1);
        expect(text(container, ".agent-shell-elapsed")).toBe("[0:07]");
    });

    it("an exited shell shows how long it ran, and stops counting", () => {
        const { container } = mount(
            shell({ status: "exited-ok", exitCode: 0, spawnedAt: T0 - 65_000, exitedAt: T0 - 5_000 })
        );
        expect(text(container, ".agent-shell-elapsed")).toBe("[1:00]");
        vi.advanceTimersByTime(10_000);
        expect(text(container, ".agent-shell-elapsed")).toBe("[1:00]");
    });

    it("counts hours as h:mm:ss", () => {
        const { container } = mount(shell({ spawnedAt: T0 - 3_661_000 }));
        expect(text(container, ".agent-shell-elapsed")).toBe("[1:01:01]");
    });
});

describe("PersistentShellBlock — the live tail", () => {
    const tail = (container: HTMLElement) => text(container, ".agent-shell-live-tail");
    const chunk = (kind: string, content: string) =>
        ({ kind, content, timestamp: 1 }) as ShellNode["log"]["chunks"][number];

    it("shows the last non-empty stdout or stderr line, trimmed, after an arrow", () => {
        const { container } = mount(
            shell({
                log: {
                    open: true,
                    chunks: [
                        chunk("stdout", "first"),
                        chunk("stderr", "  second  "),
                        chunk("system", "sys note"),
                        chunk("stdout", "   "),
                    ],
                },
            })
        );
        expect(tail(container)).toBe("→ second");
    });

    it("takes the last line, not the first", () => {
        const { container } = mount(
            shell({ log: { open: true, chunks: [chunk("stdout", "a"), chunk("stdout", "b")] } })
        );
        expect(tail(container)).toBe("→ b");
    });

    it("finds a line in the first chunk", () => {
        const { container } = mount(shell({ log: { open: true, chunks: [chunk("stdout", "only")] } }));
        expect(tail(container)).toBe("→ only");
    });

    it("counts stdout and stderr, each on its own", () => {
        expect(tail(mount(shell({ log: { open: true, chunks: [chunk("stderr", "oops")] } })).container)).toBe("→ oops");
        expect(tail(mount(shell({ log: { open: true, chunks: [chunk("stdout", "fine")] } })).container)).toBe("→ fine");
    });

    it("shows nothing for no chunks, system chunks only, or blank lines", () => {
        expect(tail(mount(shell()).container)).toBeUndefined();
        expect(
            tail(mount(shell({ log: { open: true, chunks: [chunk("system", "starting")] } })).container)
        ).toBeUndefined();
        expect(
            tail(mount(shell({ log: { open: true, chunks: [chunk("stdout", "  \n ")] } })).container)
        ).toBeUndefined();
    });
});

describe("PersistentShellBlock — status", () => {
    it.each([
        ["running", "⟩"],
        ["exited-ok", "✓"],
        ["exited-err", "✗"],
        ["stopped", "■"],
    ] as const)("%s has its own sigil and its own class only", (status, sigil) => {
        const { container } = mount(shell({ status }));
        expect(text(container, ".agent-shell-sigil")).toBe(sigil);
        const block = q(container, ".agent-shell-block")!;
        const states = ["running", "exited-ok", "exited-err", "stopped"].filter((s) => block.classList.contains(s));
        expect(states).toEqual([status]);
    });

    it("shows the title in the summary and the command in the header", () => {
        const { container } = mount(shell({ title: "My server", cmd: "node server.js" }));
        expect(text(container, ".agent-shell-title")).toBe("My server");
        expect(text(container, ".agent-shell-header-cmd")).toBe("node server.js");
    });

    it("shows the exit code, including 0, and nothing while the shell is running", () => {
        expect(
            text(mount(shell({ status: "exited-ok", exitCode: 0 })).container, ".agent-tool-overlay-status-label")
        ).toBe("exit 0");
        expect(
            text(mount(shell({ status: "exited-err", exitCode: 2 })).container, ".agent-tool-overlay-status-label")
        ).toBe("exit 2");
        expect(q(mount(shell()).container, ".agent-tool-overlay-status-label")).toBeNull();
    });
});

describe("PersistentShellBlock — the pin-to-open panel", () => {
    it("collapsed: hidden, inert, aria-hidden, and marked collapsed", () => {
        const { container } = mount(shell(), false);
        const block = q(container, ".agent-shell-block")!;
        const panel = q(container, ".agent-tool-panel")!;
        expect(block.classList.contains("collapsed")).toBe(true);
        expect(block.classList.contains("expanded")).toBe(false);
        expect(panel.classList.contains("agent-tool-panel--hidden")).toBe(true);
        expect(panel.classList.contains("agent-tool-panel--flow")).toBe(false);
        expect(isInert(panel)).toBe(true);
        expect(panel.getAttribute("aria-hidden")).toBe("true");
    });

    it("pinned: in the flow, not inert, visible to assistive tech, and marked expanded", () => {
        const { container } = mount(shell(), true);
        const block = q(container, ".agent-shell-block")!;
        const panel = q(container, ".agent-tool-panel")!;
        expect(block.classList.contains("expanded")).toBe(true);
        expect(block.classList.contains("collapsed")).toBe(false);
        expect(panel.classList.contains("agent-tool-panel--flow")).toBe(true);
        expect(panel.classList.contains("agent-tool-panel--hidden")).toBe(false);
        expect(isInert(panel)).toBe(false);
        expect(panel.getAttribute("aria-hidden")).toBe("false");
    });

    it("follows the pin as it changes", () => {
        const [pinned, setPinned] = createSignal(false);
        const { container } = render(() => (
            <PersistentShellBlock node={shell()} pinned={pinned()} onTogglePin={() => {}} />
        ));
        expect(q(container, ".agent-shell-block")!.classList.contains("collapsed")).toBe(true);
        setPinned(true);
        expect(q(container, ".agent-shell-block")!.classList.contains("expanded")).toBe(true);
        setPinned(false);
        expect(q(container, ".agent-shell-block")!.classList.contains("collapsed")).toBe(true);
    });

    it("a click on the summary toggles the pin; a click inside the panel does not reach the row above", () => {
        const onTogglePin = vi.fn();
        const outer = vi.fn();
        const { container } = render(() => (
            <div onClick={outer}>
                <PersistentShellBlock node={shell()} pinned={true} onTogglePin={onTogglePin} />
            </div>
        ));
        fireEvent.click(q(container, ".agent-tool-overlay-header")!);
        expect(onTogglePin).not.toHaveBeenCalled();
        expect(outer).not.toHaveBeenCalled();
        fireEvent.click(q(container, ".agent-shell-summary")!);
        expect(onTogglePin).toHaveBeenCalledTimes(1);
    });
});

describe("PersistentShellBlock — the Stop button", () => {
    const stopBtn = (c: HTMLElement) => q(c, "button.agent-shell-stop") as HTMLButtonElement | null;

    it("is only there while the process runs", () => {
        expect(stopBtn(mount(shell({ status: "running" })).container)).not.toBeNull();
        for (const status of ["exited-ok", "exited-err", "stopped"] as const) {
            expect(stopBtn(mount(shell({ status })).container)).toBeNull();
        }
    });

    it("is a danger-toned, labelled, compact icon button", () => {
        const btn = stopBtn(mount(shell()).container)!;
        expect(btn.getAttribute("aria-label")).toBe("Stop process");
        expect(btn.className).toMatch(/danger/);
        expect(btn.disabled).toBe(false);
    });

    it("stops that shell, without toggling the panel", async () => {
        const onTogglePin = vi.fn();
        const outer = vi.fn();
        const { container } = render(() => (
            <div onClick={outer}>
                <PersistentShellBlock node={shell({ id: "sh-42" })} pinned={false} onTogglePin={onTogglePin} />
            </div>
        ));
        fireEvent.click(stopBtn(container)!);
        await vi.advanceTimersByTimeAsync(0);
        expect(ShellStopCommand).toHaveBeenCalledTimes(1);
        expect(ShellStopCommand).toHaveBeenCalledWith({ client: true }, { shell_id: "sh-42" });
        expect(onTogglePin).not.toHaveBeenCalled();
        expect(outer).not.toHaveBeenCalled();
    });

    it("is disabled while the stop is in flight, ignores a second click, and is back after", async () => {
        let finish!: () => void;
        ShellStopCommand.mockReturnValue(new Promise<void>((r) => (finish = r)));
        const { container } = mount(shell());
        const btn = stopBtn(container)!;
        fireEvent.click(btn);
        await vi.advanceTimersByTimeAsync(0);
        expect(btn.disabled).toBe(true);
        fireEvent.click(btn);
        expect(ShellStopCommand).toHaveBeenCalledTimes(1);
        finish();
        await vi.advanceTimersByTimeAsync(0);
        expect(btn.disabled).toBe(false);
        fireEvent.click(btn);
        await vi.advanceTimersByTimeAsync(0);
        expect(ShellStopCommand).toHaveBeenCalledTimes(2);
    });

    it("swallows a failed stop and is usable again", async () => {
        ShellStopCommand.mockRejectedValue(new Error("no such process"));
        const { container } = mount(shell());
        const btn = stopBtn(container)!;
        fireEvent.click(btn);
        await vi.advanceTimersByTimeAsync(0);
        expect(btn.disabled).toBe(false);
        expect(ShellStopCommand).toHaveBeenCalledTimes(1);
    });
});

describe("PersistentShellBlock — the log panel", () => {
    const chunk = (content: string) =>
        ({ kind: "stdout", content, timestamp: 1 }) as ShellNode["log"]["chunks"][number];

    it("links a URL in the output", () => {
        const { container } = mount(
            shell({ log: { open: false, chunks: [chunk("ready at https://example.com/app")] } }),
            true
        );
        const link = q(container, ".agent-tool-overlay-log a");
        expect(link).not.toBeNull();
        expect(link!.getAttribute("href")).toBe("https://example.com/app");
    });

    it("shows the streaming indicator only while the log is open and the panel is up", () => {
        const open = mount(shell({ log: { open: true, chunks: [chunk("x")] } }), true).container;
        expect(q(open, ".agent-shell-streaming-indicator")).not.toBeNull();
        cleanup();
        const closed = mount(shell({ log: { open: false, chunks: [chunk("x")] } }), true).container;
        expect(q(closed, ".agent-shell-streaming-indicator")).toBeNull();
        cleanup();
        const collapsed = mount(shell({ log: { open: true, chunks: [chunk("x")] } }), false).container;
        expect(q(collapsed, ".agent-shell-streaming-indicator")).toBeNull();
    });
});

describe("PersistentShellBlock — what the peek is given", () => {
    const tokens = (): string | null | undefined =>
        document.body.querySelector(".agent-node-peek-tooltip-tokens")?.textContent;
    const time = (): string | null | undefined =>
        document.body.querySelector(".agent-node-peek-tooltip-time")?.textContent;

    it("anchors the peek on the summary line, and shows it only while peeking and collapsed", () => {
        const { container } = mount(shell(), false);
        expect(overlay.show).toBe(false);
        expect(overlay.rowEl).toBe(q(container, ".agent-shell-summary"));
        hover(container);
        expect(overlay.show).toBe(true);
        cleanup();
        const pinned = mount(shell(), true).container;
        hover(pinned);
        expect(overlay.show).toBe(false);
    });

    it("estimates the tokens of the command, not the title", () => {
        const { container } = mount(shell({ cmd: "x".repeat(40), title: "t" }), false);
        hover(container);
        expect(tokens()).toBe("~10 tok (est.)");
    });

    it("shows one token for a one-character command, and none for an empty one", () => {
        hover(mount(shell({ cmd: "a" }), false).container);
        expect(tokens()).toBe("~1 tok (est.)");
        cleanup();
        hover(mount(shell({ cmd: "" }), false).container);
        expect(tokens()).toBeUndefined();
        expect(time()).toMatch(/\d/);
    });

    it("writes a long command's estimate compactly", () => {
        hover(mount(shell({ cmd: "x".repeat(4000) }), false).container);
        expect(tokens()).toBe("~1.0k tok (est.)");
    });

    it("shows the time the shell started, then how long ago, and keeps it fresh every second while open", () => {
        const { container } = mount(shell({ spawnedAt: T0 - 59_000 }), false);
        expect(time()).toBeUndefined();
        hover(container);
        expect(time()).toMatch(/^\d{1,2}:\d{2}:\d{2} (?:AM|PM) · just now$/);
        vi.advanceTimersByTime(1000);
        expect(time()).toMatch(/^\d{1,2}:\d{2}:\d{2} (?:AM|PM) · 1m ago$/);
    });

    it("is named for the dev tools", () => {
        expect(PersistentShellBlock.displayName).toBe("PersistentShellBlock");
    });
});
