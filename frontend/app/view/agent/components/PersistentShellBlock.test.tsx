// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PersistentShellBlock — peek tooltip added by
 * SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25. Suppressed once
 * pinned/expanded — the full command is already visible in the panel header.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PersistentShellBlock } from "./PersistentShellBlock";
import type { ShellNode } from "../types";

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

const hover = (container: HTMLElement) => {
    const root = container.querySelector(".agent-shell-block") as HTMLElement;
    fireEvent.mouseEnter(root);
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
            expect(metaLines.length).toBe(2);
            expect(metaLines[0].textContent).toMatch(/\d{1,2}:\d{2}:\d{2} (?:AM|PM) · 1m ago/);
            expect(metaLines[1].textContent).toMatch(/~\d+ tok \(est\.\)/);
            const body = document.body.querySelector(".agent-node-peek-tooltip-body");
            expect(body?.textContent).toBe("npm run dev");
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
            fireEvent.mouseLeave(container.querySelector(".agent-shell-block") as HTMLElement);
            expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });
});
