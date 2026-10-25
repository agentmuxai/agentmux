// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import { setPrewarmLease } from "@/app/store/agent-prewarm";
import { AgentDormancyProvider } from "./agent-dormancy";
import { ToolBlock } from "./components/ToolBlock";
import { useRenderDormant, useWindowHidden } from "./render-dormant";
import type { ToolLogChunk, ToolNode } from "./types";

/** Hides or shows the page the way Chromium does for a minimized or covered window. */
function setDocumentHidden(hidden: boolean): void {
    Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
    document.dispatchEvent(new Event("visibilitychange"));
}

afterEach(() => {
    setDocumentHidden(false);
    cleanup();
});

describe("useRenderDormant", () => {
    it("follows the page's visibility", () => {
        createRoot((dispose) => {
            const hidden = useWindowHidden();
            expect(hidden()).toBe(false);
            setDocumentHidden(true);
            expect(hidden()).toBe(true);
            setDocumentHidden(false);
            expect(hidden()).toBe(false);
            dispose();
        });
    });

    it("lets a hidden pane draw while it holds the prewarm lease, and only that pane", () => {
        createRoot((dispose) => {
            const mine = useRenderDormant(() => true, "blk-1");
            const other = useRenderDormant(() => true, "blk-2");
            expect(mine()).toBe(true);
            setPrewarmLease("blk-1");
            expect(mine()).toBe(false);
            expect(other()).toBe(true);
            setDocumentHidden(true); // a hidden window stays dormant whatever the lease says
            expect(mine()).toBe(true);
            setDocumentHidden(false);
            setPrewarmLease(null);
            expect(mine()).toBe(true);
            dispose();
        });
    });

    it("is dormant when the pane's tab is hidden, or when the window is", () => {
        createRoot((dispose) => {
            const [tabHidden, setTabHidden] = createSignal(false);
            const dormant = useRenderDormant(tabHidden);
            expect(dormant()).toBe(false);
            setTabHidden(true);
            expect(dormant()).toBe(true);
            setTabHidden(false);
            setDocumentHidden(true);
            expect(dormant()).toBe(true);
            setDocumentHidden(false);
            expect(dormant()).toBe(false);
            dispose();
        });
    });
});

// A minimized window mounted ~4,000 preview lines per agent in 2 minutes because a tool's
// live output kept building there (no frame or geometry event ever trimmed it). Dormant,
// the body keeps what it showed and builds nothing new.
describe("a hidden window's tool output", () => {
    const chunk = (n: number): ToolLogChunk => ({ kind: "stdout", content: `line ${n}\n`, timestamp: n });
    const tool: ToolNode = {
        type: "tool",
        id: "tc-1",
        tool: "Bash",
        params: { command: "build" },
        status: "running",
        collapsed: false,
        summary: "Bash build",
        log: { open: true, chunks: [chunk(0)] },
    };

    it("stays bounded across many chunks while the window is hidden, and catches up when it is shown", () => {
        const [node, setNode] = createSignal<ToolNode>(tool);
        const [tabHidden] = createSignal(false);
        const { container } = render(() => {
            const dormant = useRenderDormant(tabHidden);
            return (
                <AgentDormancyProvider dormant={dormant}>
                    <ToolBlock node={node()} pinned={false} onTogglePin={() => {}} />
                </AgentDormancyProvider>
            );
        });
        const lines = (): number => (container.querySelector(".agent-tool-overlay-log")?.textContent ?? "").split("line").length - 1;
        expect(lines()).toBe(1);

        setDocumentHidden(true);
        const chunks = [chunk(0)];
        for (let i = 1; i <= 500; i++) {
            chunks.push(chunk(i));
            setNode({ ...tool, log: { open: true, chunks: [...chunks] } });
        }
        expect(lines()).toBe(1);

        setDocumentHidden(false);
        expect(lines()).toBeGreaterThan(1);
    });
});
