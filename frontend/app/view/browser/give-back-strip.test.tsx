// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { BrowserViewModel } from "./browser-model";
import { GiveBackStrip } from "./give-back-strip";

afterEach(cleanup);

function model(giveBack: () => Promise<void>) {
    return { giveBack } as unknown as BrowserViewModel;
}

describe("GiveBackStrip", () => {
    it("says the person is driving and names the agent the pane goes back to", () => {
        render(() => <GiveBackStrip model={model(async () => {})} agent="Lark" />);
        expect(screen.getByRole("status").textContent).toContain("You are driving this pane");
        expect(screen.getByText("Give back to Lark")).toBeTruthy();
    });

    it("Give back asks the model to give the pane back", () => {
        const giveBack = vi.fn(async () => {});
        render(() => <GiveBackStrip model={model(giveBack)} agent="Lark" />);
        fireEvent.click(screen.getByText("Give back to Lark"));
        expect(giveBack).toHaveBeenCalledTimes(1);
    });

    it("shows why a failed Give back failed, in the strip", async () => {
        const giveBack = vi.fn(async () => {
            throw new Error("AgentMux didn't answer");
        });
        render(() => <GiveBackStrip model={model(giveBack)} agent="Lark" />);
        fireEvent.click(screen.getByText("Give back to Lark"));
        expect((await screen.findByRole("alert")).textContent).toContain("didn't answer");
    });
});
