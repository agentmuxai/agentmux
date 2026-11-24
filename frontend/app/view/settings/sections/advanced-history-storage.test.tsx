// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings › Advanced › Conversation history: the one row that opens the Audit pane on its Storage section
 * (where history is kept and for how long). It is in Settings search and it is a button in the section.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ open: vi.fn() }));

vi.mock("@/app/view/audit/open-audit", () => ({ openAuditSection: h.open }));
vi.mock("@/app/store/global", async () => {
    const { createSignal } = await import("solid-js");
    const [settings] = createSignal<unknown>({});
    return { settingsAtom: settings };
});

import { SETTINGS_INDEX, searchSettings } from "../settings-index";
import { HistoryGroup } from "./advanced-section-parts";

afterEach(() => {
    cleanup();
    h.open.mockClear();
});

describe("the history storage row", () => {
    it("is found by search, and choosing it opens the Audit on Storage instead of showing a row", async () => {
        for (const query of ["where history is kept", "disk space", "retention"]) {
            const hit = searchSettings(SETTINGS_INDEX, query).find((e) => e.id === "advanced.history_storage");
            expect(hit, query).toBeDefined();
        }
        SETTINGS_INDEX.find((e) => e.id === "advanced.history_storage")!.opens!();
        await vi.waitFor(() => expect(h.open).toHaveBeenCalledWith("storage"));
    });

    it("is a button in the Advanced section that does the same", async () => {
        const { getByRole, container } = render(() => <HistoryGroup />);
        expect(container.querySelector('[id="setting-advanced.history_storage"]')).not.toBeNull();
        fireEvent.click(getByRole("button", { name: "Open Storage" }));
        await vi.waitFor(() => expect(h.open).toHaveBeenCalledWith("storage"));
    });
});
