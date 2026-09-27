// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CollapsibleMessage } from "./CollapsibleMessage";

afterEach(() => cleanup());

// The shell JektBubble and AgentMessageBlock share
// (SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md §1).
const mount = (collapsed: boolean, onToggle = vi.fn()) =>
    render(() => (
        <CollapsibleMessage
            rootClass="agent-x-block"
            classPrefix="agent-x"
            classes={{ incoming: true }}
            collapsed={collapsed}
            onToggle={onToggle}
            summary={<span class="agent-x-peer">From a</span>}
            body={<pre class="agent-x-body">hello</pre>}
            peekText="hello"
            timestamp={1_700_000_000_000}
        />
    ));

describe("CollapsibleMessage", () => {
    it("renders the root, summary and chevron under the given class names", () => {
        const { container } = mount(false);
        const root = container.querySelector(".agent-x-block")!;
        expect(root.classList.contains("incoming")).toBe(true);
        expect(root.classList.contains("collapsed")).toBe(false);
        expect(container.querySelector(".agent-x-summary .agent-x-chevron")!.textContent).toBe("▾");
        expect(container.querySelector(".agent-x-summary .agent-x-peer")!.textContent).toBe("From a");
    });

    it("shows the body only when open", () => {
        expect(mount(false).container.querySelector(".agent-x-content .agent-x-body")).not.toBeNull();
        cleanup();
        const closed = mount(true).container;
        expect(closed.querySelector(".agent-x-content")).toBeNull();
        expect(closed.querySelector(".agent-x-chevron")!.textContent).toBe("▸");
        expect(closed.querySelector(".agent-x-block")!.classList.contains("collapsed")).toBe(true);
    });

    it("toggles on a click on the row, but not on a click inside the body", () => {
        const onToggle = vi.fn();
        const { container } = mount(false, onToggle);
        fireEvent.click(container.querySelector(".agent-x-body")!);
        expect(onToggle).not.toHaveBeenCalled();
        fireEvent.click(container.querySelector(".agent-x-summary")!);
        expect(onToggle).toHaveBeenCalledTimes(1);
    });

    it("shows the time + token peek on hover", () => {
        vi.useFakeTimers();
        try {
            const { container } = mount(true);
            fireEvent.mouseEnter(container.querySelector(".agent-x-block")!);
            vi.advanceTimersByTime(100);
            const lines = [...document.body.querySelectorAll(".agent-node-peek-tooltip-meta")].map((e) => e.textContent);
            expect(lines.some((l) => l?.includes("ago"))).toBe(true);
            expect(lines.some((l) => l?.includes("tok (est.)"))).toBe(true);
        } finally {
            vi.useRealTimers();
        }
    });
});
