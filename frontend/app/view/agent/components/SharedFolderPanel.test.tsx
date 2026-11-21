// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** SharedFolderPanel: names both agents, says what sharing the folder means, and hands the person's choice back. */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SharedFolderPanel } from "./SharedFolderPanel";

afterEach(() => cleanup());

const pair = (container: string) => ({ host_id: "h", host_name: "Korp", container_id: `c-${container}`, container_name: container });

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

    it("lists every container agent that can write the folder, once each", () => {
        show({ overlaps: [pair("Aria"), pair("Aria"), pair("Ula")] });
        expect(screen.getByTestId("shared-folder-text").textContent).toContain("Aria, Ula can write");
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
