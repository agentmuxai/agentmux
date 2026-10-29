// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** "Pin this section to the widget bar" (section-pins.ts): the section's entry is found or added,
 *  then pinned, and a pinned one offers to unpin. */

import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

let config: { widgets: Record<string, WidgetConfigType>; settings: Record<string, unknown> };
vi.mock("@/store/global", () => ({ atoms: { fullConfigAtom: () => config } }));
const { sectionEntry, setConfig } = vi.hoisted(() => ({
    sectionEntry: vi.fn(async (_c: unknown, req: { view: string; section: string }) => ({
        key: `section@${req.view}-${req.section.replace("/", "-")}`,
    })),
    setConfig: vi.fn(async (_c: unknown, _patch: unknown) => {}),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { WidgetsSectionEntryCommand: sectionEntry, SetConfigCommand: setConfig } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { registerPaneTab } from "@/app/block/pane-tab-registry";
import { pinSection, sectionEntryKey, sectionPinMenuItems, shownSections } from "./section-pins";

let unregister = () => {};
let unregisterSwarm = () => {};
beforeAll(() => {
    unregister = registerPaneTab({
        apiVersion: 1,
        view: "warden",
        label: "Warden",
        icon: "shield-halved",
        sections: {
            default: "host",
            items: [
                { id: "jobs", label: "Jobs", icon: "table-columns" },
                {
                    id: "vault",
                    label: "Vault",
                    icon: "lock",
                    sections: { default: "personal", items: [{ id: "personal", label: "Personal data", icon: "id-card" }, { id: "cards", label: "Cards and banks", icon: "credit-card" }] },
                },
                { id: "host", label: "Host", icon: "server" },
                { id: "lan", label: "LAN", icon: "network-wired" },
            ],
        },
        create: () => ({ component: () => null }),
    });
    unregisterSwarm = registerPaneTab({
        apiVersion: 1,
        view: "swarm",
        label: "Swarm",
        icon: "diagram-project",
        sections: { default: "supervisor", items: [{ id: "supervisor", label: "Supervisor", icon: "user-shield" }, { id: "broadcast", label: "Broadcast", icon: "tower-broadcast" }] },
        create: () => ({ component: () => null }),
    });
});
afterAll(() => {
    unregister();
    unregisterSwarm();
});

const jobs: WidgetConfigType = { label: "Jobs", section: "jobs", blockdef: { meta: { view: "warden", "warden:section": "jobs" } } };
const warden: WidgetConfigType = { label: "Warden", blockdef: { meta: { view: "warden" } } };

beforeEach(() => {
    config = { widgets: { "defwidget@jobs": jobs, "defwidget@warden": warden }, settings: { "widget:pinned": ["warden"] } };
    sectionEntry.mockClear();
    setConfig.mockClear();
});

describe("pinning a section to the widget bar", () => {
    it("names the section a pane shows, and a section's own section too", () => {
        expect(shownSections({ view: "warden", "warden:section": "lan" }).map((s) => s.path)).toEqual(["lan"]);
        expect(shownSections({ view: "warden" }).map((s) => s.label)).toEqual(["Host"]);
        expect(shownSections({ view: "warden", "warden:section": "vault", "vault:section": "cards" }).map((s) => [s.path, s.label])).toEqual([
            ["vault", "Vault"],
            ["vault/cards", "Cards and banks"],
        ]);
        expect(shownSections({ view: "term" })).toEqual([]);
    });

    it("finds the entry that opens exactly that section, and only that one", () => {
        const [lan] = shownSections({ view: "warden", "warden:section": "lan" });
        const [kan] = shownSections({ view: "warden", "warden:section": "jobs" });
        expect(sectionEntryKey(kan, config.widgets)).toBe("defwidget@jobs");
        // Warden itself opens no particular section: not LAN's entry.
        expect(sectionEntryKey(lan, config.widgets)).toBeUndefined();
    });

    it("pins an existing entry without adding one", async () => {
        const [kan] = shownSections({ view: "warden", "warden:section": "jobs" });
        await pinSection(kan);
        expect(sectionEntry).not.toHaveBeenCalled();
        expect(setConfig).toHaveBeenCalledWith({}, { "widget:pinned": ["warden", "jobs"] });
    });

    it("adds an entry through srv when none opens the section, then pins it by its key", async () => {
        const [, cards] = shownSections({ view: "warden", "warden:section": "vault", "vault:section": "cards" });
        await pinSection(cards);
        expect(sectionEntry).toHaveBeenCalledWith({}, { view: "warden", section: "vault/cards", label: "Cards and banks", icon: "credit-card" });
        expect(setConfig).toHaveBeenCalledWith({}, { "widget:pinned": ["warden", "section@warden-vault-cards"] });
    });

    it("shows an entry the user had hidden when pinning it", async () => {
        config.settings["widget:hidden"] = ["defwidget@jobs"];
        const [kan] = shownSections({ view: "warden", "warden:section": "jobs" });
        expect(sectionPinMenuItems({ view: "warden", "warden:section": "jobs" }).map((i) => i.label)).toEqual(["Pin “Jobs” to bar"]);
        await pinSection(kan);
        expect(setConfig).toHaveBeenCalledWith({}, { "widget:hidden": null, "widget:pinned": ["warden", "jobs"] });
    });

    it("draws the pin icon on Pin and the unpin icon on Unpin", () => {
        const icons = (section: string) => sectionPinMenuItems({ view: "warden", "warden:section": section }).map((i) => i.icon);
        expect(icons("jobs")).toEqual(["thumbtack"]);
        config.settings["widget:pinned"] = ["warden", "jobs"];
        expect(icons("jobs")).toEqual(["thumbtack-slash"]);
    });

    it("offers to unpin a section whose entry is pinned", () => {
        config.settings["widget:pinned"] = ["warden", "jobs"];
        expect(sectionPinMenuItems({ view: "warden", "warden:section": "jobs" }).map((i) => i.label)).toEqual(["Unpin “Jobs” from bar"]);
        expect(sectionPinMenuItems({ view: "warden", "warden:section": "lan" }).map((i) => i.label)).toEqual(["Pin “LAN” to bar"]);
        expect(sectionPinMenuItems({ view: "term" })).toEqual([]);
    });
});
