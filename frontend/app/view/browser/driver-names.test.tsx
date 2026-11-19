// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A browser pane names the agent that drives it, and the agent it can be given back to, by its name ("Tess"), not
 * by the id srv records it under ("tess-2"): in the strip, its tooltips, the Auto-approve confirmation and strip,
 * the Log, the Give back strip and the sign-in notice.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/agent-names", () => ({
    followAgentNames: () => {},
    agentDisplayName: (id: string) => (id.toLowerCase() === "tess-2" ? "Tess" : id),
}));
vi.mock("@/app/store/browser-pause-all", () => ({
    browserPauseAll: () => ({ paused: false, since_ms: null, by: null }),
    pausedLabel: () => "",
    setAgentBrowsingPaused: async () => {},
}));

import type { BrowserViewModel } from "./browser-model";
import { ActionLogStrip, DrivenByStrip } from "./driven-by-strip";
import { GiveBackStrip } from "./give-back-strip";

afterEach(cleanup);

function fakeModel(over: Partial<Record<string, unknown>> = {}) {
    const [bypass, setBypass] = createSignal<BrowserBypass | undefined>(undefined);
    const [actions, setActions] = createSignal<BrowserActions>({ count: 0, entries: [] });
    const model = {
        actionsAtom: actions,
        pausedAtom: () => false,
        allowedOriginsAtom: () => [] as string[],
        tookControlAtom: () => false,
        bypassAtom: bypass,
        bypassEndedAtom: () => undefined,
        bypassPaused: () => false,
        bypassMinutesLeft: () => 42,
        bypassOriginsPreview: () => ["https://example.com"],
        takeOver: async () => {},
        ...over,
    } as unknown as BrowserViewModel;
    return { model, setBypass, setActions };
}

describe("the agent a browser pane names", () => {
    it("is the driver's name in the strip and its tooltips, not its id", () => {
        const { model } = fakeModel();
        const { container } = render(() => <DrivenByStrip model={model} agent="tess-2" />);
        expect(container.textContent).toContain("Driven by Tess");
        expect(container.textContent).not.toContain("tess-2");
        const titles = [...container.querySelectorAll("[title]")].map((e) => e.getAttribute("title"));
        expect(titles.join("\n")).toContain("What Tess did in this pane");
        expect(titles.join("\n")).not.toContain("tess-2");
    });

    it("is the driver's name in the Auto-approve strip", () => {
        const { model, setBypass } = fakeModel();
        setBypass({ agent: "tess-2", origins: ["https://example.com"], minutes: 60, expires_at: Date.now() + 3_600_000 } as BrowserBypass);
        const { container } = render(() => <DrivenByStrip model={model} agent="tess-2" />);
        expect(container.textContent).toContain("Auto-approve on: Tess's clicks");
        expect(container.textContent).not.toContain("tess-2");
    });

    it("is the driver's name in the Log, after the person took the pane over", () => {
        const { model, setActions } = fakeModel();
        setActions({ count: 1, entries: [{ ts: 1, agent: "tess-2", kind: "click", target: "e1", how: "allowed" } as BrowserActions["entries"][number]] });
        const { container } = render(() => <ActionLogStrip model={model} />);
        expect(container.textContent).toContain("Tess's actions here");
        expect(container.textContent).not.toContain("tess-2");
    });

    it("is the name of the agent the pane goes back to, in the Give back strip", () => {
        const { container, getByText } = render(() => <GiveBackStrip model={fakeModel().model} agent="tess-2" />);
        expect(getByText("Give back to Tess")).toBeTruthy();
        expect(container.textContent).not.toContain("tess-2");
    });

    it("stays the id where no agent has that id", () => {
        const { container } = render(() => <DrivenByStrip model={fakeModel().model} agent="ghost" />);
        expect(container.textContent).toContain("Driven by ghost");
    });
});
