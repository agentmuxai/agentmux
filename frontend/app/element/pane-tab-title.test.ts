// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * A tab's title comes from persisted state (block meta and the view's registry entry), not from whether its view
 * has mounted: a restored Settings tab on Paired devices says so before anyone clicks it.
 */

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({ atoms: { fullConfigAtom: () => ({ widgets: {} }) } }));

import { registerPaneTab } from "@/app/block/pane-tab-registry";
import { connectorsPaneTab } from "@/app/view/connectors/connectors";
import { filesPaneTab } from "@/app/view/files/files";
import { identityPaneTab } from "@/app/view/identity/identity-pane";
import { memoryPaneTab } from "@/app/view/memory/memory";
import { settingsPaneTab } from "@/app/view/settings/settings";
import { swarmPaneTab } from "@/app/view/swarm/swarm";
import { towerPaneTab } from "@/app/view/tower/tower";
import { vaultPaneTab } from "@/app/view/vault/vault";
import { createPaneTabMemory, describePaneTab } from "./pane-tab-model";

for (const tab of [settingsPaneTab, vaultPaneTab, connectorsPaneTab, memoryPaneTab, swarmPaneTab, filesPaneTab, identityPaneTab, towerPaneTab]) {
    registerPaneTab(tab);
}

const restored = (view: string, meta: Record<string, unknown>, extra: Record<string, unknown> = {}) =>
    ({ blockId: `b-${view}`, view, meta: { view, ...meta }, ordinal: 1, liveViewModel: null, ...extra }) as Parameters<typeof describePaneTab>[0];

describe("a restored tab that has not mounted", () => {
    it("names a Settings pane after its section, from the meta, before any click", () => {
        expect(describePaneTab(restored("settings", { "settings:section": "devices" })).label).toBe("Paired devices");
        expect(describePaneTab(restored("settings", {})).label).toBe("Appearance");
    });

    it("names the other sectioned panes the way their mounted view does", () => {
        expect(describePaneTab(restored("vault", { "vault:section": "cards" })).label).toBe("Vault · Cards and banks");
        expect(describePaneTab(restored("swarm", { "swarm:section": "broadcast" })).label).toBe("Swarm · Broadcast");
        expect(describePaneTab(restored("swarm", {})).label).toBe("Swarm · Supervisor");
        expect(describePaneTab(restored("connectors", { "connectors:section": "mcp" })).label).toBe("Connectors · MCP servers");
        expect(describePaneTab(restored("memory", { "memory:section": "skills" })).label).toBe("Memory · Skills");
    });

    it("names Files after its folder, Tower after its view, and Identity", () => {
        expect(describePaneTab(restored("files", { "files:path": "C:/work/notes" })).label).toBe("notes");
        expect(describePaneTab(restored("tower", { "tower:view": "panes" })).label).toBe("Tower · Panes");
        expect(describePaneTab(restored("identity", {})).label).toBe("Identity");
    });

    it("leaves a remote Tower to the name it last showed, which the meta does not hold", () => {
        const meta = { "tower:connection": "peer:area54", "pane:label": "Tower · Area54" };
        expect(describePaneTab(restored("tower", meta)).label).toBe("Tower · Area54");
        expect(describePaneTab(restored("tower", { "tower:connection": "peer:area54" })).label).toBe("Tower");
    });

    it("follows the meta over a name kept from before the section changed", () => {
        const tab = describePaneTab(restored("settings", { "settings:section": "devices", "pane:label": "Appearance" }));
        expect(tab.label).toBe("Paired devices");
        // The kept name is not rewritten with an older one.
        expect(tab.persist?.name).toBeUndefined();
    });

    it("does not change when the tab is clicked and its view reports the same title", () => {
        const memory = createPaneTabMemory();
        const before = describePaneTab(restored("settings", { "settings:section": "devices" }), undefined, memory);
        const live = { viewName: () => "Paired devices" } as unknown as ViewModel;
        const after = describePaneTab(restored("settings", { "settings:section": "devices" }, { liveViewModel: live }), undefined, memory);
        expect(after.label).toBe(before.label);
    });

    it("lets the mounted view refine it (a remote Tower's host), and the user's own title win", () => {
        const live = { viewName: () => "Tower · Area54" } as unknown as ViewModel;
        expect(describePaneTab(restored("tower", { "tower:view": "panes" }, { liveViewModel: live })).label).toBe("Tower · Area54");
        expect(describePaneTab(restored("settings", { "settings:section": "devices", "frame:title": "Mine" })).label).toBe("Mine");
    });

    it("still shows the view's own label for a view with no title of its own", () => {
        expect(settingsPaneTab.title?.({})).toBe("Appearance");
        expect(describePaneTab(restored("nothing-registered", {})).label).not.toBe("");
    });
});
