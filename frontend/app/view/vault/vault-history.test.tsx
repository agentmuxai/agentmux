// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** An entry's History dialog: the rows, the Changes and Uses filters, what a missing reason says, and the record's
 *  state (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md §6). */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { VaultActivity } from "@/types/rpc/VaultActivity";
import type { VaultChain } from "@/types/rpc/VaultChain";

const row = (over: Partial<VaultActivity>): VaultActivity => ({
    at: Date.now() - 60_000,
    entry_id: "api-key-stripe",
    entry_label: "Stripe",
    actor: "you",
    action: "created",
    target: "",
    outcome: "done",
    seq: 1,
    version: 1,
    actor_kind: "person",
    host: "narko",
    reason: "",
    refs: [],
    is_use: false,
    ...over,
});

const rows: VaultActivity[] = [
    row({ seq: 4, actor: "builder", actor_kind: "agent", action: "ran a command", target: "deploy", version: 2, is_use: true, reason: "ship the fix for task 27968", refs: ["#27968", "https://example.com/pr/1"] }),
    row({ seq: 3, actor: "builder", actor_kind: "agent", action: "rotated", version: 2, reason: "after the leak" }),
    row({ seq: 2, actor: "builder", actor_kind: "agent", action: "revealed", version: 1, is_use: true }),
    row({ seq: 1, actor: "old", actor_kind: "", host: "", action: "copied", version: 0, is_use: true }),
];
const chain = vi.fn<() => Promise<VaultChain>>();
vi.mock("./vault-store", () => ({
    loadHistory: () => Promise.resolve(rows),
    loadChain: () => chain(),
}));

import { chainNote, historyFilter, VaultHistoryDialog, whyOf } from "./vault-history";

beforeEach(() => chain.mockReset().mockResolvedValue({ intact: true, checked: 3, before_chain: 1, broken_at: null, problem: "" }));
afterEach(cleanup);

describe("what a row's reason says", () => {
    it("shows the agent's words, or what their absence means for that row", () => {
        expect(whyOf(rows[0])).toBe("ship the fix for task 27968");
        expect(whyOf(rows[2])).toBe("(no reason given)");
        expect(whyOf(rows[3])).toBe("(before reasons were recorded)");
        expect(whyOf(row({}))).toBe("");
    });

    it("filters changes from uses", () => {
        expect(historyFilter(rows, "all")).toHaveLength(4);
        expect(historyFilter(rows, "uses").map((r) => r.seq)).toEqual([4, 2, 1]);
        expect(historyFilter(rows, "changes").map((r) => r.seq)).toEqual([3]);
    });

    it("says whether the record holds", () => {
        expect(chainNote(undefined)).toBe("");
        expect(chainNote({ intact: true, checked: 3, before_chain: 1, broken_at: null, problem: "" })).toContain("3 checked; 1 older line was made before checking began");
        expect(chainNote({ intact: false, checked: 1, before_chain: 0, broken_at: 7, problem: "this line was changed" })).toBe("The record has been changed: line 7 — this line was changed.");
    });
});

describe("the History dialog", () => {
    it("lists every row with who, what, version and why, and the record's state", async () => {
        render(() => <VaultHistoryDialog entryId="api-key-stripe" label="Stripe" onClose={() => {}} />);
        await waitFor(() => expect(document.querySelectorAll("tr[data-seq]")).toHaveLength(4));
        const first = document.querySelector('tr[data-seq="4"]')!;
        expect(first.textContent).toContain("builder");
        expect(first.textContent).toContain("ran a command");
        expect(first.textContent).toContain("v2");
        expect(first.textContent).toContain("ship the fix for task 27968");
        expect(first.textContent).toContain("#27968 · https://example.com/pr/1");
        expect(document.querySelector('tr[data-seq="1"]')!.textContent).toContain("(before reasons were recorded)");
        await waitFor(() => expect(document.body.textContent).toContain("Each line follows the one before it"));
    });

    it("filters to uses and to changes", async () => {
        render(() => <VaultHistoryDialog entryId="api-key-stripe" label="Stripe" onClose={() => {}} />);
        await waitFor(() => expect(document.querySelectorAll("tr[data-seq]")).toHaveLength(4));
        fireEvent.click(screen.getByRole("button", { name: "Uses" }));
        expect(Array.from(document.querySelectorAll("tr[data-seq]")).map((r) => r.getAttribute("data-seq"))).toEqual(["4", "2", "1"]);
        fireEvent.click(screen.getByRole("button", { name: "Changes" }));
        expect(Array.from(document.querySelectorAll("tr[data-seq]")).map((r) => r.getAttribute("data-seq"))).toEqual(["3"]);
    });

    it("warns when the record was changed", async () => {
        chain.mockResolvedValue({ intact: false, checked: 1, before_chain: 0, broken_at: 3, problem: "this line was changed" });
        render(() => <VaultHistoryDialog entryId="api-key-stripe" label="Stripe" onClose={() => {}} />);
        expect((await screen.findByRole("alert")).textContent).toContain("line 3");
    });
});
