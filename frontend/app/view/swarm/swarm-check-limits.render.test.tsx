// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Check now" beside an agent's "Out of quota": it asks srv to look at the agent again, shows that it is
 * checking, and says so when the check could not start. A kept broadcast shows in the result panel as
 * kept, not as a failure.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { CheckNowButton, checkNowText } from "./swarm-check-limits";
import { FleetResultPanel } from "./swarm-fleet-toolbar";
import type { FleetResultEntry, SwarmViewModel } from "./swarm-model";

afterEach(() => cleanup());

const button = () => document.querySelector<HTMLButtonElement>(".swarm-unavailable-check")!;

describe("CheckNowButton", () => {
    it("asks for a check of its own pane and shows that it is checking", async () => {
        const check = vi.fn(async () => null);
        render(() => <CheckNowButton blockId="blk-1" check={check} />);
        expect(button().textContent).toContain("Check now");
        fireEvent.click(button());
        await Promise.resolve();
        expect(check).toHaveBeenCalledWith(["blk-1"]);
        expect(button().textContent).toContain("Checking");
        expect(button().disabled).toBe(true);
    });

    it("is offered again, saying why, when the check could not start", async () => {
        const check = vi.fn(async () => "the agent is not registered");
        render(() => <CheckNowButton blockId="blk-1" check={check} />);
        fireEvent.click(button());
        await vi.waitFor(() => expect(button().textContent).toContain("Check failed"));
        expect(button().disabled).toBe(false);
    });

    it("one click starts one check", async () => {
        const check = vi.fn(async () => null);
        render(() => <CheckNowButton blockId="blk-1" check={check} />);
        fireEvent.click(button());
        fireEvent.click(button());
        await Promise.resolve();
        expect(check).toHaveBeenCalledTimes(1);
    });

    it("words for its states", () => {
        expect(checkNowText(false, null)).toBe("Check now");
        expect(checkNowText(true, null)).toBe("Checking…");
        expect(checkNowText(false, "no")).toBe("Check failed");
    });
});

describe("a broadcast kept for an agent out of quota", () => {
    const entry = (held: { id: string; note: string }[]): FleetResultEntry => ({
        action: "broadcast",
        result: { succeeded: ["blk-ok"], failed: [], aborted_early: false, pending: [], held },
        labels: { "blk-ok": "Loap", "blk-held": "Posa" },
    });
    const panel = (e: FleetResultEntry, quietWhenClean = false) => {
        const model = { lastFleetResultAtom: () => e, dismissFleetResult: () => {} } as unknown as SwarmViewModel;
        render(() => <FleetResultPanel model={model} quietWhenClean={quietWhenClean} />);
    };

    it("is a row of its own with the reason, not a failure, and is counted", () => {
        panel(entry([{ id: "blk-held", note: "Out of quota until Sun 7:00 PM; the check has not cleared it yet. Kept, and sent when it can work" }]));
        const held = document.querySelector(".swarm-fleet-result-row--held")!;
        expect(held.textContent).toContain("Posa");
        expect(held.textContent).toContain("Kept, and sent when it can work");
        expect(document.querySelector(".swarm-fleet-result-row--fail")).toBeNull();
        expect(document.querySelector(".swarm-fleet-result-header")!.textContent).toContain("1 kept");
    });

    it("shows in the broadcast section even though nothing failed: the person has to see it was not sent yet", () => {
        panel(entry([{ id: "blk-held", note: "kept" }]), true);
        expect(document.querySelector(".swarm-fleet-result-row--held")).not.toBeNull();
    });

    it("a clean broadcast stays quiet in the broadcast section", () => {
        panel(entry([]), true);
        expect(document.querySelector(".swarm-fleet-result-panel")).toBeNull();
    });
});
