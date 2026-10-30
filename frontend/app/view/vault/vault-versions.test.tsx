// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Earlier values: listed without a value, and Restore puts one back
 *  (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.4). */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultVersionInfo } from "@/types/rpc/VaultVersionInfo";

const loadVersions = vi.fn<(id: string) => Promise<VaultVersionInfo[]>>();
const restoreVersion = vi.fn<(id: string, seq: number) => Promise<unknown>>();
vi.mock("./vault-store", () => ({ loadVersions: (id: string) => loadVersions(id), restoreVersion: (id: string, seq: number) => restoreVersion(id, seq) }));

import { VaultVersionsDialog } from "./vault-versions";

const key: VaultEntryInfo = {
    id: "api-key-openai",
    kind: "api-key",
    section: "api-keys",
    label: "OpenAI",
    fields: {},
    tags: [],
    parts_set: ["value"],
    grants: [],
    tail: "9xQ2",
    versions: 1,
    created_at: 1,
    updated_at: 1,
    last_used_at: 0,
};

const earlier: VaultVersionInfo = { seq: 7, replaced_at: Date.now() - 60_000, replaced_by: "you", how: "replaced", tail: "a1B3", state: "previous" };

describe("Earlier values", () => {
    beforeEach(() => {
        loadVersions.mockReset();
        restoreVersion.mockReset();
    });
    afterEach(cleanup);

    it("lists each earlier value by its last characters, and who replaced it", async () => {
        loadVersions.mockResolvedValue([earlier]);
        render(() => <VaultVersionsDialog entry={key} onClose={() => {}} />);
        const row = await waitFor(() => {
            const el = document.querySelector<HTMLElement>('[data-seq="7"]');
            if (!el) throw new Error("not listed yet");
            return el;
        });
        expect(row.textContent).toContain("•••• a1B3");
        expect(row.textContent).toContain("replaced by you");
        expect(loadVersions).toHaveBeenCalledWith("api-key-openai");
    });

    it("restores one, then shows the list again", async () => {
        loadVersions.mockResolvedValueOnce([earlier]).mockResolvedValueOnce([{ ...earlier, seq: 8, how: "restored", tail: "9xQ2" }]);
        restoreVersion.mockResolvedValue({});
        render(() => <VaultVersionsDialog entry={key} onClose={() => {}} />);
        fireEvent.click(await screen.findByRole("button", { name: /Restore/ }));
        await waitFor(() => expect(document.querySelector('[data-seq="8"]')).not.toBeNull());
        expect(restoreVersion).toHaveBeenCalledWith("api-key-openai", 7);
    });

    it("says why a restore failed", async () => {
        loadVersions.mockResolvedValue([earlier]);
        restoreVersion.mockRejectedValue(new Error("OpenAI has no earlier value 7: it may be past its 7 days"));
        render(() => <VaultVersionsDialog entry={key} onClose={() => {}} />);
        fireEvent.click(await screen.findByRole("button", { name: /Restore/ }));
        expect((await screen.findByRole("alert")).textContent).toContain("past its 7 days");
    });

    it("marks a rotation's new value, and offers no restore while its rotation runs", async () => {
        loadVersions.mockResolvedValue([
            { ...earlier, seq: 9, how: "rotated", replaced_by: "Agent2-6", state: "pending" },
            { ...earlier, seq: 10, how: "rotated", replaced_by: "Agent2-6", state: "rotation failed at step 3" },
        ]);
        render(() => <VaultVersionsDialog entry={key} onClose={() => {}} />);
        const pending = await waitFor(() => {
            const el = document.querySelector('[data-seq="9"]');
            if (!el) throw new Error("not yet");
            return el;
        });
        expect(pending.textContent).toContain("still running");
        expect(pending.querySelector("button")).toBeNull();
        const failed = document.querySelector('[data-seq="10"]')!;
        expect(failed.textContent).toContain("may hold it");
        expect(failed.querySelector("button")).not.toBeNull();
    });

    it("says when none is kept", async () => {
        loadVersions.mockResolvedValue([]);
        render(() => <VaultVersionsDialog entry={key} onClose={() => {}} />);
        await screen.findByText("No earlier value is kept.");
    });
});
