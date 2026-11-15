// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { KIND_PLACES } from "./summary-kinds";

const goTo = vi.hoisted(() => vi.fn());
vi.mock("@/app/notification/attention-go", () => ({ goToAttention: goTo }));
const openPane = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: openPane }));
const openSettingsAt = vi.hoisted(() => vi.fn());
vi.mock("@/app/view/settings/open-settings", () => ({ openSettingsAt }));
const openVaultAt = vi.hoisted(() => vi.fn());
vi.mock("@/app/view/vault/open-vault", () => ({ openVaultAt }));

import { goToSummaryRow, OPEN_VERSION_PANEL_EVENT } from "./summary-go";

afterEach(() => vi.clearAllMocks());

describe("goToSummaryRow", () => {
    it("reveals a pane, opens the Vault at a request and the Kanban from its row", async () => {
        const item = { blockId: "b1", name: "Korp", what: "", since: 0, local: true };
        await goToSummaryRow({ to: "pane", item });
        expect(goTo).toHaveBeenCalledWith(item);
        await goToSummaryRow({ to: "vault", id: "u1" });
        expect(openVaultAt).toHaveBeenCalledWith("u1");
        await goToSummaryRow({ to: "kanban" });
        expect(openPane).toHaveBeenCalledWith("kanban");
    });

    it("takes a request with no pane to where its kind is answered", async () => {
        await goToSummaryRow({ to: "app", kind: "widget", key: "k" });
        expect(openSettingsAt).toHaveBeenCalledWith("widgets");
        await goToSummaryRow({ to: "app", kind: "consent", key: "k" });
        expect(openSettingsAt).toHaveBeenCalledWith("devices");
        await goToSummaryRow({ to: "app", kind: "desktop", key: "k" });
        expect(openSettingsAt).toHaveBeenCalledWith("agents");
        const heard = vi.fn();
        window.addEventListener(OPEN_VERSION_PANEL_EVENT, heard);
        await goToSummaryRow({ to: "app", kind: "upgrade", key: "k" });
        expect(heard).toHaveBeenCalledTimes(1);
        window.removeEventListener(OPEN_VERSION_PANEL_EVENT, heard);
        await goToSummaryRow({ to: "app", kind: "vault", key: "u9" });
        expect(openVaultAt).toHaveBeenCalledWith("u9");
    });

    it("sends a kind answered in a pane, announced with no pane, to the Swarm", async () => {
        await goToSummaryRow({ to: "app", kind: "confirm", key: "k" });
        expect(openPane).toHaveBeenCalledWith("swarm");
    });

    it("has a place for every kind: none falls through unnamed", () => {
        for (const [kind, place] of Object.entries(KIND_PLACES)) expect(place.answer, kind).toBeTruthy();
    });
});
