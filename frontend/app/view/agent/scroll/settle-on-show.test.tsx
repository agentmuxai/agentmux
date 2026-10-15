// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { AgentDormancyProvider } from "../agent-dormancy";
import { useSettleOnShow } from "./settle-on-show";

function mount(following: boolean, dormantAtMount = true) {
    const [dormant, setDormant] = createSignal(dormantAtMount);
    const flow = { settleOnShow: vi.fn() };
    const Probe = () => {
        useSettleOnShow(flow, () => following);
        return null;
    };
    render(() => (
        <AgentDormancyProvider dormant={dormant}>
            <Probe />
        </AgentDormancyProvider>
    ));
    return { flow, setDormant };
}

// SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md §3.1.
describe("useSettleOnShow", () => {
    it("settles when a following pane is shown again", () => {
        const { flow, setDormant } = mount(true);
        expect(flow.settleOnShow).not.toHaveBeenCalled(); // not at mount
        setDormant(false);
        expect(flow.settleOnShow).toHaveBeenCalledTimes(1);
        setDormant(true); // hidden: nothing
        expect(flow.settleOnShow).toHaveBeenCalledTimes(1);
        setDormant(false);
        expect(flow.settleOnShow).toHaveBeenCalledTimes(2);
    });

    it("leaves a pane the user scrolled up alone", () => {
        const { flow, setDormant } = mount(false);
        setDormant(false);
        expect(flow.settleOnShow).not.toHaveBeenCalled();
    });

    it("does nothing without a one-way flow", () => {
        expect(() => render(() => <AgentDormancyProvider dormant={() => false}>{(() => { useSettleOnShow(null, () => true); return null; })()}</AgentDormancyProvider>)).not.toThrow();
    });
});
