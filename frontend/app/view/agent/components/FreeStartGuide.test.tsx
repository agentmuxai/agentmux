// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { makeTestHostApi } from "@/app/host/test-host";
import { FreeStartGuide } from "./FreeStartGuide";

afterEach(() => cleanup());

const NOW = () => new Date("2026-10-29T12:00:00Z");

function renderGuide(provider: string, openExternal = vi.fn(), now = NOW) {
    window.api = makeTestHostApi({ openExternal } as never);
    return render(() => <FreeStartGuide provider={provider} name="Antigravity" now={now} />);
}

describe("FreeStartGuide", () => {
    it("shows the lead, the steps and the day the claims were checked", () => {
        const { getByTestId, container } = renderGuide("antigravity");
        expect(getByTestId("agent-free-guide").textContent).toContain("Free with a Google sign-in");
        expect(container.querySelectorAll(".agent-free-guide-steps li").length).toBeGreaterThanOrEqual(2);
        expect(getByTestId("agent-free-guide-checked").textContent).toBe("checked 2026-10-29");
    });

    it("opens the sign-up page from the data in the system browser, and only that page", () => {
        const openExternal = vi.fn();
        const { getByTestId } = renderGuide("antigravity", openExternal);
        fireEvent.click(getByTestId("agent-free-guide-open"));
        expect(openExternal).toHaveBeenCalledTimes(1);
        expect(openExternal).toHaveBeenCalledWith("https://antigravity.google/docs/getting-started?tab=cli");
    });

    it("marks a stale claim in words", () => {
        const { getByTestId } = renderGuide("antigravity", vi.fn(), () => new Date("2027-03-01T12:00:00Z"));
        expect(getByTestId("agent-free-guide-checked").textContent).toBe("Not re-checked since 2026-10-29");
    });

    it("renders nothing for a provider without a free sign-in route", () => {
        const { queryByTestId } = renderGuide("codex");
        expect(queryByTestId("agent-free-guide")).toBeNull();
    });
});
