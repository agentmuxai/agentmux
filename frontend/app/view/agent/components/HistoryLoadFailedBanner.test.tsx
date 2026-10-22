// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import type { InitPhase } from "@/app/store/agent-pane-state/types";
import { HistoryLoadFailedBanner } from "./HistoryLoadFailedBanner";

afterEach(() => {
    cleanup();
});

describe("HistoryLoadFailedBanner", () => {
    it("renders nothing while the history loads or once it has loaded", () => {
        const [phase, setPhase] = createSignal<InitPhase>({ kind: "InitPending" });
        render(() => <HistoryLoadFailedBanner phase={phase} />);
        expect(screen.queryByRole("status")).toBeNull();
        setPhase({ kind: "InitReady", at: 1 });
        expect(screen.queryByRole("status")).toBeNull();
    });

    it("says the earlier messages couldn't be loaded, why, and what still works", () => {
        const [phase] = createSignal<InitPhase>({
            kind: "InitFailed",
            at: 1,
            reason: "the transcript index for b1 is unavailable right now; retry",
        });
        render(() => <HistoryLoadFailedBanner phase={phase} />);
        const banner = screen.getByRole("status");
        expect(banner.textContent).toContain("Couldn't load this conversation's earlier messages");
        expect(banner.textContent).toContain("unavailable right now");
        expect(banner.textContent).toContain("New messages still work");
        expect(banner.textContent).toContain("reopen the pane");
    });

    it("appears when a load that was pending fails", () => {
        const [phase, setPhase] = createSignal<InitPhase>({ kind: "InitPending" });
        render(() => <HistoryLoadFailedBanner phase={phase} />);
        setPhase({ kind: "InitFailed", at: 2, reason: "boom" });
        expect(screen.getByRole("status").textContent).toContain("(boom)");
    });
});
