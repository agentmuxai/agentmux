// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's right-click menu: Seat… and Personal memory for
 * the pane's own agent, on a live agent pane and nowhere else.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const openMemory = vi.fn();
const openHistory = vi.fn();
vi.mock("@/app/store/section-panes", () => ({
    openMemory: (...args: unknown[]) => openMemory(...args),
}));
vi.mock("./open-history-tab", () => ({
    HISTORY_TAB_FOR_META_KEY: "agent:historyTabFor",
    openOrFocusHistoryTab: (...args: unknown[]) => openHistory(...args),
}));

const setGrant = vi.fn((..._args: unknown[]) => Promise.resolve());
const setGoogle = vi.fn((..._args: unknown[]) => Promise.resolve());
let granted = false;
let googleGranted = false;
vi.mock("@/app/store/agent-grants", () => ({
    startsAgentsWithoutAsking: () => granted,
    setStartsAgentsWithoutAsking: (...args: unknown[]) => setGrant(...args),
    usesGoogleWithoutAsking: () => googleGranted,
    setUsesGoogleWithoutAsking: (...args: unknown[]) => setGoogle(...args),
}));
import { MENU_ICON } from "@/app/util/menu-icons";
import { agentBodyMenuItems, type AgentBodyMenuHost } from "./agent-body-menu";

const labels = (items: ContextMenuItem[]) => items.filter((i) => i.type !== "separator").map((i) => i.label);
const item = (items: ContextMenuItem[], label: string) => items.find((i) => i.label === label)!;

function host(overrides: Partial<AgentBodyMenuHost> = {}): AgentBodyMenuHost {
    return {
        blockId: "block-1",
        meta: { agentId: "uid-x", agentName: "AgentX" },
        hasDefinition: true,
        openModal: vi.fn(),
        quickFork: vi.fn(),
        shutDown: vi.fn(),
        ...overrides,
    };
}

describe("agentBodyMenuItems", () => {
    beforeEach(() => {
        openMemory.mockReset();
        openHistory.mockReset();
        setGrant.mockClear();
        setGoogle.mockClear();
        granted = false;
        googleGranted = false;
    });

    it("shows Seat and Personal memory after History and Quick Fork on an agent pane, with no View and no Bundles", () => {
        const items = agentBodyMenuItems(host());
        expect(labels(items)).toEqual(["Agent History", "Quick Fork", "Seat…", "Personal memory", "Starts agents without asking", "Shut down"]);
        expect(labels(items)).not.toContain("View…");
        expect(labels(items)).not.toContain("Bundles…");
        expect(item(items, "Seat…").icon).toBe(MENU_ICON.seat);
        expect(item(items, "Personal memory").icon).toBe(MENU_ICON.memory);
        // Ends with the separator that sets the pane's own items off from the shared ones.
        expect(items[items.length - 1]).toEqual({ type: "separator" });
    });

    it("opens the seat modal for the pane's own agent", () => {
        const h = host();
        item(agentBodyMenuItems(h), "Seat…").click!();
        expect(h.openModal).toHaveBeenLastCalledWith({ kind: "agent-seat", agentId: "uid-x", agentName: "AgentX" });
    });

    it("opens Memory → Personal on the pane's own agent", () => {
        item(agentBodyMenuItems(host()), "Personal memory").click!();
        expect(openMemory).toHaveBeenCalledWith("personal", { agentId: "uid-x" });
    });

    it("shows whether the agent starts agents without asking, and flips it", () => {
        expect(item(agentBodyMenuItems(host()), "Starts agents without asking")).toMatchObject({ type: "checkbox", checked: false });
        granted = true;
        const entry = item(agentBodyMenuItems(host()), "Starts agents without asking");
        expect(entry.checked).toBe(true);
        entry.click!();
        expect(setGrant).toHaveBeenCalledWith("uid-x", false);
    });

    it("lists a Google Always only once it was given, and takes it back", () => {
        expect(labels(agentBodyMenuItems(host()))).not.toContain("Uses Google without asking");
        googleGranted = true;
        const entry = item(agentBodyMenuItems(host()), "Uses Google without asking");
        expect(entry).toMatchObject({ type: "checkbox", checked: true });
        entry.click!();
        expect(setGoogle).toHaveBeenCalledWith("uid-x", false);
    });

    it("keeps History and Quick Fork working", () => {
        const h = host();
        const items = agentBodyMenuItems(h);
        item(items, "Agent History").click!();
        expect(openHistory).toHaveBeenCalledWith({ currentBlockId: "block-1", agentId: "uid-x" });
        item(items, "Quick Fork").click!();
        expect(h.quickFork).toHaveBeenCalled();
    });

    it("shuts the pane's agent down from the last item, on a quick-launch pane too", () => {
        const h = host();
        item(agentBodyMenuItems(h), "Shut down").click!();
        expect(h.shutDown).toHaveBeenCalledTimes(1);
        const quick = host({ meta: { agentId: "claude" }, hasDefinition: false });
        expect(item(agentBodyMenuItems(quick), "Shut down").icon).toBe(MENU_ICON.shutdown);
    });

    it("has no menu on a history tab", () => {
        expect(agentBodyMenuItems(host({ meta: { agentId: "uid-x", "agent:historyTabFor": "uid-x" } }))).toEqual([]);
    });

    it("has no menu before an agent is loaded", () => {
        expect(agentBodyMenuItems(host({ meta: {} }))).toEqual([]);
        expect(agentBodyMenuItems(host({ meta: undefined }))).toEqual([]);
    });

    it("leaves out the agent items on a quick-launch pane, which has no definition to edit", () => {
        const items = agentBodyMenuItems(host({ meta: { agentId: "claude" }, hasDefinition: false }));
        expect(labels(items)).toEqual(["Agent History", "Quick Fork", "Shut down"]);
    });

    it("names the agent by id when the pane has no name", () => {
        const h = host({ meta: { agentId: "uid-x" } });
        item(agentBodyMenuItems(h), "Seat…").click!();
        expect(h.openModal).toHaveBeenCalledWith({ kind: "agent-seat", agentId: "uid-x", agentName: "uid-x" });
    });
});
