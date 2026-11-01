// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/notification/usenotification", () => ({ setNotificationAction: vi.fn() }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: vi.fn(() => () => {}) }));
vi.mock("@/app/store/global", () => ({ atoms: { activeTabId: () => "t-here" }, pushNotification: vi.fn() }));
vi.mock("@/app/util/reveal-block", () => ({ loadObject: vi.fn(), revealBlock: vi.fn() }));

import { handleAgentOpened, type AgentOpenedDeps } from "./agent-opened-notice";

const event = { block_id: "b1", tab_id: "t-agent", view: "settings" };

function fakes(over: Partial<AgentOpenedDeps> = {}) {
    const pushed: NotificationType[] = [];
    const actions = new Map<string, () => void>();
    const shown: string[][] = [];
    const deps: AgentOpenedDeps = {
        focused: () => true,
        shownTab: () => "t-here",
        nameOf: async () => undefined,
        show: (b, t) => void shown.push([b, t]),
        push: (n) => void pushed.push(n),
        onAction: (k, fn) => void actions.set(k, fn),
        ...over,
    };
    return { deps, pushed, actions, shown };
}

describe("handleAgentOpened", () => {
    it("puts a quiet notice with a Show button in the window the person is in", async () => {
        const { deps, pushed, actions, shown } = fakes();
        expect(await handleAgentOpened({ ...event, title: "Vault" }, deps)).toBe(true);
        expect(pushed).toHaveLength(1);
        expect(pushed[0]).toMatchObject({ id: "agent-opened:b1", title: "Vault opened", type: "info" });
        expect(pushed[0].persistent).toBeFalsy();
        expect(pushed[0].actions?.map((a) => a.label)).toEqual(["Show"]);
        actions.get("agent-opened:b1:show")!();
        expect(shown).toEqual([["b1", "t-agent"]]);
    });

    it("says nothing in a window the person is not in, or when the pane is already in front", async () => {
        const away = fakes({ focused: () => false });
        expect(await handleAgentOpened(event, away.deps)).toBe(false);
        const here = fakes();
        expect(await handleAgentOpened({ ...event, tab_id: "t-here" }, here.deps)).toBe(false);
        expect(await handleAgentOpened(undefined, here.deps)).toBe(false);
        expect([...away.pushed, ...here.pushed]).toEqual([]);
    });

    it("names the pane by the agent's title, else its own name, else its view", async () => {
        const named = fakes({ nameOf: async () => "Parlo" });
        await handleAgentOpened({ ...event, view: "agent" }, named.deps);
        expect(named.pushed[0].title).toBe("Parlo opened");
        const plain = fakes();
        await handleAgentOpened({ ...event, title: "  " }, plain.deps);
        expect(plain.pushed[0].title).toBe("Settings opened");
    });
});
