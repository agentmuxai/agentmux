// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ConfirmPanel } from "./ConfirmPanel";

afterEach(cleanup);

function panel(over: Partial<Parameters<typeof ConfirmPanel>[0]> = {}) {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(() => (
        <ConfirmPanel title="Delete me@example.com?" confirmLabel="Delete account" cancelLabel="Cancel" destructive onConfirm={onConfirm} onCancel={onCancel} {...over}>
            <p>AgentMux forgets this account.</p>
        </ConfirmPanel>
    ));
    return { onConfirm, onCancel };
}

describe("ConfirmPanel", () => {
    it("is a named group with the question, what it does, Cancel and the confirm, and takes focus when it opens", () => {
        panel();
        const group = screen.getByRole("group", { name: "Delete me@example.com?" });
        expect(group.textContent).toContain("AgentMux forgets this account.");
        expect(document.activeElement).toBe(group);
        const buttons = [...group.querySelectorAll("button")].map((b) => b.textContent);
        expect(buttons).toEqual(["Cancel", "Delete account"]);
    });

    it("cancels from Cancel and from Escape, and asks nothing of the confirm", () => {
        const { onConfirm, onCancel } = panel();
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        fireEvent.keyDown(screen.getByRole("group"), { key: "Escape" });
        expect(onCancel).toHaveBeenCalledTimes(2);
        expect(onConfirm).not.toHaveBeenCalled();
    });

    it("keeps Escape to itself: the element around it does not get it", () => {
        const outside = vi.fn();
        render(() => (
            <div onKeyDown={outside}>
                <ConfirmPanel title="Delete?" confirmLabel="Delete" cancelLabel="Cancel" onConfirm={() => {}} onCancel={() => {}} />
            </div>
        ));
        fireEvent.keyDown(screen.getByRole("group"), { key: "Escape" });
        expect(outside).not.toHaveBeenCalled();
        fireEvent.keyDown(screen.getByRole("group"), { key: "a" });
        expect(outside).toHaveBeenCalledTimes(1);
    });

    it("confirms once, shows busy while it runs, and does not cancel meanwhile", async () => {
        let done: () => void = () => {};
        const onConfirm = vi.fn(() => new Promise<void>((resolve) => (done = resolve)));
        const onCancel = vi.fn();
        panel({ onConfirm, onCancel });
        const confirm = screen.getByRole("button", { name: "Delete account" });
        fireEvent.click(confirm);
        fireEvent.click(confirm);
        expect(onConfirm).toHaveBeenCalledTimes(1);
        fireEvent.keyDown(screen.getByRole("group"), { key: "Escape" });
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(onCancel).not.toHaveBeenCalled();
        done();
        await waitFor(() => expect((screen.getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled).toBe(false));
    });
});
