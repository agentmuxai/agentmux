// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { InfoCard } from "./info-card";

const probe = { size: 1024, mtime: 0, kind: "other", mime: "application/octet-stream", read_only: false };

function renderCard(path: string, open: () => void) {
    render(() => (
        <InfoCard
            path={path}
            probe={probe}
            bar={{ viewer: "info", options: ["info"], onReopenWith: () => {}, onOpenWithSystem: open }}
            onOpenWithSystem={open}
        />
    ));
    return document.querySelector<HTMLButtonElement>(".editor-viewer-bar [aria-label=\"Open with the system's app\"]")!;
}

describe("the info card's open with the system's app", () => {
    afterEach(() => cleanup());

    it("asks before the toolbar button runs a program", () => {
        const open = vi.fn();
        const toolbarOpen = renderCard("C:/x/setup.exe", open);
        fireEvent.click(toolbarOpen);
        expect(screen.getByRole("dialog")).toBeTruthy();
        expect(open).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Run" }));
        expect(open).toHaveBeenCalledOnce();
        expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("doesn't run the program when the user cancels", () => {
        const open = vi.fn();
        fireEvent.click(renderCard("C:/x/setup.exe", open));
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(open).not.toHaveBeenCalled();
    });

    it("opens anything else straight away", () => {
        const open = vi.fn();
        fireEvent.click(renderCard("C:/x/archive.zip", open));
        expect(screen.queryByRole("dialog")).toBeNull();
        expect(open).toHaveBeenCalledOnce();
    });
});
