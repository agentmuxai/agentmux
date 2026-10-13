// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PersistentShellBlock — peek tooltip added by
 * SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25. Suppressed once
 * pinned/expanded — the full command is already visible in the panel header.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createSignal } from "solid-js";
import type { ShellNode } from "../types";
import { BODY_LINGER_MS } from "./body-mount";
import { PersistentShellBlock } from "./PersistentShellBlock";

afterEach(() => cleanup());

const node: ShellNode = {
    type: "shell",
    id: "sh-1",
    cmd: "npm run dev",
    title: "dev server",
    status: "exited-ok",
    exitCode: 0,
    spawnedAt: Date.now() - 65_000,
    exitedAt: Date.now() - 5_000,
    log: { chunks: [], open: false },
};

// The summary line peeks; the expanded output doesn't
// (SPEC_AGENT_PANE_TOOL_PEEK_ON_LINE_AND_MD_OPEN_IN_EDITOR_2026_10_11.md §1).
const hover = (container: HTMLElement) => {
    const line = container.querySelector(".agent-shell-summary") as HTMLElement;
    fireEvent.mouseEnter(line);
    vi.advanceTimersByTime(100);
};

describe("PersistentShellBlock — peek tooltip", () => {
    it("shows time + estimate + the command on hover while collapsed", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <PersistentShellBlock node={node} pinned={false} onTogglePin={() => {}} />
            ));
            hover(container);
            const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
            // One line: time and tokens side by side (PeekMetaRow).
            expect(metaLines.length).toBe(1);
            expect(metaLines[0].querySelector(".agent-node-peek-tooltip-time")?.textContent).toMatch(
                /\d{1,2}:\d{2}:\d{2} (?:AM|PM) · 1m ago/
            );
            expect(metaLines[0].querySelector(".agent-node-peek-tooltip-tokens")?.textContent).toMatch(
                /~\d+ tok \(est\.\)/
            );
            const body = document.body.querySelector(".agent-node-peek-tooltip-body");
            expect(body?.textContent).toBe("npm run dev");
            // Takes the shell row's command colour, not a tool's (spec §4.2).
            expect(body?.classList.contains("agent-node-peek-tooltip-body--shell")).toBe(true);
        } finally {
            vi.useRealTimers();
        }
    });

    it("suppresses the overlay while pinned open — the command is already visible in the panel", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <PersistentShellBlock node={node} pinned={true} onTogglePin={() => {}} />
            ));
            hover(container);
            expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });

    it("hides on mouseleave", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <PersistentShellBlock node={node} pinned={false} onTogglePin={() => {}} />
            ));
            hover(container);
            expect(document.body.querySelector(".agent-node-peek-overlay")).not.toBeNull();
            fireEvent.mouseLeave(container.querySelector(".agent-shell-summary") as HTMLElement);
            expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });
});

describe("PersistentShellBlock — hides bashwrap's internal starting-chunk (2026-09-03)", () => {
    it("does not render the leading [bashwrap] starting system chunk in the expanded panel", () => {
        const running: ShellNode = {
            ...node,
            status: "running",
            exitCode: undefined,
            exitedAt: undefined,
            log: {
                open: true,
                chunks: [
                    { kind: "system", content: "[bashwrap] starting: 12 chars", timestamp: 1 },
                    { kind: "stdout", content: "server listening on :3000", timestamp: 2 },
                ],
            },
        };
        const { container } = render(() => (
            <PersistentShellBlock node={running} pinned={true} onTogglePin={() => {}} />
        ));
        expect(container.textContent).not.toContain("bashwrap");
        expect(container.textContent).toContain("server listening on :3000");
        expect(container.querySelectorAll(".agent-preview-line")).toHaveLength(1);
    });
});

// SPEC_TOOL_BODY_UNMOUNT_ON_COLLAPSE_2026_10_12.md §3: the log is built only
// while the panel is open, and for BODY_LINGER_MS after it closes.
describe("PersistentShellBlock — the log mounts while open, lingers, then goes", () => {
    const withOutput: ShellNode = {
        ...node,
        log: { open: false, chunks: [{ kind: "stdout", content: "built in 2.1s", timestamp: 1 }] },
    };
    const logLines = (c: HTMLElement) => c.querySelectorAll(".agent-tool-overlay-log .agent-preview-line").length;

    it("a collapsed shell never opened has no log lines, but keeps its header", () => {
        const { container } = render(() => (
            <PersistentShellBlock node={withOutput} pinned={false} onTogglePin={() => {}} />
        ));
        expect(logLines(container)).toBe(0);
        expect(container.querySelector(".agent-shell-header-cmd")?.textContent).toBe("npm run dev");
    });

    it("opening shows the log; closing drops it after the linger", () => {
        vi.useFakeTimers();
        try {
            const [pinned, setPinned] = createSignal(true);
            const { container } = render(() => (
                <PersistentShellBlock node={withOutput} pinned={pinned()} onTogglePin={() => {}} />
            ));
            expect(logLines(container)).toBe(1);
            setPinned(false);
            vi.advanceTimersByTime(BODY_LINGER_MS - 1);
            expect(logLines(container)).toBe(1);
            vi.advanceTimersByTime(1);
            expect(logLines(container)).toBe(0);
        } finally {
            vi.useRealTimers();
        }
    });
});
