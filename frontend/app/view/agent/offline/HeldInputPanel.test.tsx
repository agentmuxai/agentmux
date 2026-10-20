// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { HeldInputPanel } from "./HeldInputPanel";

afterEach(cleanup);

const held = [
    { id: "a", text: "u there?", heldAt: 1 },
    { id: "b", text: "and then this", heldAt: 2 },
];

describe("HeldInputPanel", () => {
    it("lists held messages with their time, and Send now and Cancel", () => {
        const onSendNow = vi.fn();
        const onCancel = vi.fn();
        render(() => (
            <HeldInputPanel
                held={held}
                notices={[]}
                onSendNow={onSendNow}
                onCancel={onCancel}
                onDismissNotice={() => {}}
                clockTime={(ms) => `t${ms}`}
            />
        ));
        expect(screen.getByText("Held: will send when the API is reachable")).toBeTruthy();
        expect(screen.getByText("u there?")).toBeTruthy();
        expect(screen.getByText("t2")).toBeTruthy();
        fireEvent.click(screen.getByText("Send now"));
        expect(onSendNow).toHaveBeenCalledTimes(1);
        fireEvent.click(screen.getAllByText("Cancel")[1]);
        expect(onCancel).toHaveBeenCalledWith("b");
    });

    it("shows nothing held when nothing is", () => {
        render(() => (
            <HeldInputPanel held={[]} notices={[]} onSendNow={() => {}} onCancel={() => {}} onDismissNotice={() => {}} clockTime={String} />
        ));
        expect(screen.queryByText("Held: will send when the API is reachable")).toBeNull();
    });

    it("shows what the pane did, with Retry when it offers one, and Dismiss", () => {
        const retry = vi.fn();
        const onDismissNotice = vi.fn();
        render(() => (
            <HeldInputPanel
                held={[]}
                notices={[
                    { id: 1, text: "Back online: retried the turn that failed at 14:32." },
                    { id: 2, text: "Back online. The turn that failed at 14:32 wasn't retried.", retry },
                ]}
                onSendNow={() => {}}
                onCancel={() => {}}
                onDismissNotice={onDismissNotice}
                clockTime={String}
            />
        ));
        expect(screen.getByText("Back online: retried the turn that failed at 14:32.")).toBeTruthy();
        expect(screen.getAllByText("Retry")).toHaveLength(1);
        fireEvent.click(screen.getByText("Retry"));
        expect(retry).toHaveBeenCalledTimes(1);
        fireEvent.click(screen.getAllByLabelText("Dismiss")[0]);
        expect(onDismissNotice).toHaveBeenCalledWith(1);
    });
});
