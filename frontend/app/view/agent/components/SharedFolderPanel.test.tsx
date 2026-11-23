// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** SharedFolderPanel: names both agents, says what sharing the folder means, and hands the person's choice back. */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SharedFolderPanel } from "./SharedFolderPanel";

afterEach(() => cleanup());

const pair = (container: string, untrusted = false) => ({ host_id: "h", host_name: "Korp", container_id: `c-${container}`, container_name: container, untrusted });

function show(over: Partial<Parameters<typeof SharedFolderPanel>[0]> = {}) {
    const onGoAhead = vi.fn();
    const onKeepApart = vi.fn();
    render(() => <SharedFolderPanel hostName="Korp" overlaps={[pair("Aria")]} busy={false} onGoAhead={onGoAhead} onKeepApart={onKeepApart} {...over} />);
    return { onGoAhead, onKeepApart };
}

describe("SharedFolderPanel", () => {
    it("names the host agent and the container agent and says the container agent can change what the host agent runs", () => {
        show();
        const text = screen.getByTestId("shared-folder-text").textContent ?? "";
        expect(text).toContain("Aria");
        expect(text).toContain("Korp");
        expect(text).toMatch(/can change what Korp runs/);
    });

    it("says why it asks again when a choice is kept that nothing records the person made, and says nothing of it otherwise", () => {
        show({ overlaps: [pair("Aria", true)] });
        expect(screen.getByTestId("shared-folder-untrusted").textContent).toMatch(/no record that you made it, so it asks again/);
        cleanup();
        show({ overlaps: [pair("Aria")] });
        expect(screen.queryByTestId("shared-folder-untrusted")).toBeNull();
    });

    it("lists every container agent that can write the folder, once each", () => {
        show({ overlaps: [pair("Aria"), pair("Aria"), pair("Ula")] });
        expect(screen.getByTestId("shared-folder-text").textContent).toContain("Aria, Ula can write");
    });

    it("brings itself into view when it appears, without moving what is already in view", () => {
        const scroll = vi.fn();
        const had = Element.prototype.scrollIntoView;
        Element.prototype.scrollIntoView = scroll as never;
        try {
            show();
        } finally {
            Element.prototype.scrollIntoView = had;
        }
        expect(scroll).toHaveBeenCalledTimes(1);
        expect(scroll).toHaveBeenCalledWith({ block: "nearest", inline: "nearest" });
        expect(scroll.mock.contexts[0]).toBe(screen.getByTestId("shared-folder-panel"));
    });

    it("looks again each time what is above it changes, and stops at the first thing the person does", async () => {
        const scroll = vi.fn();
        const had = Element.prototype.scrollIntoView;
        Element.prototype.scrollIntoView = scroll as never;
        try {
            show();
            const around = screen.getByTestId("shared-folder-panel").parentElement!;
            around.prepend(document.createElement("div")); // a card finishes loading above it
            await vi.waitFor(() => expect(scroll).toHaveBeenCalledTimes(2));
            fireEvent.wheel(document);
            around.prepend(document.createElement("div"));
            await new Promise((resolve) => setTimeout(resolve, 20));
            expect(scroll).toHaveBeenCalledTimes(2);
        } finally {
            Element.prototype.scrollIntoView = had;
        }
    });

    it("looks again when the panel, what is around it or the list changes size, with no change to the tree", () => {
        const watching: Element[] = [];
        let again: () => void = () => {};
        let disconnected = 0;
        vi.stubGlobal(
            "ResizeObserver",
            class {
                constructor(callback: () => void) {
                    again = callback;
                }
                observe(target: Element) {
                    watching.push(target);
                }
                disconnect() {
                    disconnected++;
                }
            }
        );
        const scroll = vi.fn();
        const had = Element.prototype.scrollIntoView;
        Element.prototype.scrollIntoView = scroll as never;
        try {
            const list = document.createElement("div");
            list.style.overflowY = "auto";
            document.body.append(list);
            render(() => <SharedFolderPanel hostName="Korp" overlaps={[pair("Aria")]} busy={false} onGoAhead={vi.fn()} onKeepApart={vi.fn()} />, { container: list.appendChild(document.createElement("div")) });
            const panel = screen.getByTestId("shared-folder-panel");
            expect(watching).toEqual([panel, panel.parentElement, list]);
            expect(scroll).toHaveBeenCalledTimes(1);
            again();
            expect(scroll).toHaveBeenCalledTimes(2);
            fireEvent.wheel(document);
            expect(disconnected).toBe(1);
        } finally {
            Element.prototype.scrollIntoView = had;
            vi.unstubAllGlobals();
            document.body.replaceChildren();
        }
    });

    it("stops looking after ten seconds when the person does nothing", async () => {
        vi.useFakeTimers();
        const scroll = vi.fn();
        const had = Element.prototype.scrollIntoView;
        Element.prototype.scrollIntoView = scroll as never;
        try {
            show();
            const around = screen.getByTestId("shared-folder-panel").parentElement!;
            vi.advanceTimersByTime(10_001);
            around.prepend(document.createElement("div"));
            await Promise.resolve();
            expect(scroll).toHaveBeenCalledTimes(1);
        } finally {
            Element.prototype.scrollIntoView = had;
            vi.useRealTimers();
        }
    });

    it("says what to do instead", () => {
        show();
        const panel = screen.getByTestId("shared-folder-panel").textContent ?? "";
        expect(panel).toMatch(/separate workspaces/);
        expect(panel).toMatch(/read-only/);
    });

    it("goes ahead or keeps them apart as the person chooses, and the buttons wait while it launches", () => {
        const { onGoAhead, onKeepApart } = show();
        fireEvent.click(screen.getByTestId("shared-folder-go-ahead"));
        expect(onGoAhead).toHaveBeenCalledTimes(1);
        fireEvent.click(screen.getByTestId("shared-folder-cancel"));
        expect(onKeepApart).toHaveBeenCalledTimes(1);
        cleanup();
        const busy = show({ busy: true });
        fireEvent.click(screen.getByTestId("shared-folder-cancel"));
        expect(busy.onKeepApart).not.toHaveBeenCalled();
    });
});
