// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A batch import's one approval: a row for every secret with its tick, an answer that names the rows left ticked,
 *  and the batch's lines together in the Activity tab
 *  (docs/specs/SPEC_VAULT_LINKED_ENTRIES_FLEET_VAULT_AND_BATCH_IMPORT_2026_11_15.md §4). Secret ids are invented. */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { UseBatch } from "@/types/rpc/UseBatch";
import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultActivity } from "@/types/rpc/VaultActivity";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

const batch = (over: Partial<UseBatch> = {}): UseBatch => ({
    name: "host passwords",
    service: "AWS Secrets Manager",
    region: "eu-west-1",
    profile: "work-read",
    grant: false,
    vault: "Fleet",
    items: [
        { label: "Alpha", kind: "login", secret: "hosts/alpha", key: "password", replaces: false, link: false },
        { label: "Beta", kind: "login", secret: "hosts/beta", key: "", replaces: false, link: false },
        { label: "Stripe", kind: "api-key", secret: "pay/stripe", key: "", replaces: true, link: false },
    ],
    ...over,
});

const use = (over: Partial<UseRequest> = {}): UseRequest => ({
    id: "use-1",
    agent: "Agent1",
    entry_id: "",
    entry_label: "host passwords",
    what: "secrets",
    origin: "",
    page: "",
    field: "",
    offer_always: false,
    reason: "an agent asked to copy secrets into your Vault",
    agent_reason: "copy the build hosts' passwords for task 31007",
    needs_value: false,
    kind: "",
    command: "",
    cwd: "",
    replaces: false,
    signin: false,
    account: false,
    change: false,
    requested_ms: 1,
    block_id: "pane-1",
    deadline_ms: 1,
    expires_ms: 1,
    goes_ahead: false,
    batch: batch(),
    ...over,
});

let uses: UseRequest[] = [];
const vault = vi.fn(async (op: string, _args?: Record<string, unknown>): Promise<unknown> => {
    if (op === "list") return { entries: [], problem: null, requests: [], uses, backend: "keychain" };
    return {};
});
vi.mock("@/util/app-api", () => ({ getApi: () => ({ approvals: { vault } }) }));

import { batchOf, groupBatches } from "./vault-activity-batches";
import { outcomeOf } from "./vault-format";
import { VaultUseItem as UsePrompt } from "./vault-requests-list";
import { refreshVault } from "./vault-store";

const calls = (op: string) => vault.mock.calls.filter((c) => c[0] === op).map((c) => c[1] ?? {});

describe("the approval of a batch import", () => {
    beforeEach(async () => {
        vault.mockClear();
        uses = [use()];
        await refreshVault();
    });
    afterEach(cleanup);

    it("names the list, the account, the Vault and what the agent says, with a ticked row for every secret", async () => {
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("group");
        const text = dialog.textContent!;
        expect(text).toContain("Agent1 wants to copy 3 secrets from AWS Secrets Manager into your Vault: host passwords");
        expect(text).toContain("Read from AWS Secrets Manager in eu-west-1.");
        expect(text).toContain("They are read with the AWS profile work-read on this computer.");
        expect(text).toContain("saved to the Vault Fleet");
        expect(text).toContain("copy the build hosts' passwords for task 31007");
        expect(text).toContain("nothing is copied without a yes");
        expect(text).toContain("The agent never sees a value: AgentMux reads them.");

        const rows = within(within(dialog).getByRole("list", { name: "The secrets to copy" })).getAllByRole("listitem");
        expect(rows.map((r) => r.textContent)).toEqual(["Alphanew login entryhosts/alpha (key password)", "Betanew login entryhosts/beta", "Stripereplaces its value (the old one is kept 7 days)pay/stripe"]);
        expect(within(dialog).getAllByRole("checkbox").map((c) => (c as HTMLInputElement).checked)).toEqual([true, true, true]);
        // One yes for the list, never a standing one, and no countdown: silence is a no.
        expect(within(dialog).getByRole("button", { name: "Allow these 3" })).toBeTruthy();
        expect(within(dialog).queryByRole("button", { name: /Always|Allow once/ })).toBeNull();
        expect(within(dialog).queryByRole("timer")).toBeNull();
    });

    it("answers with the rows left ticked, and can't allow none", async () => {
        const close = vi.fn();
        render(() => <UsePrompt use={uses[0]} close={close} />);
        const dialog = await screen.findByRole("group");
        const ticks = within(dialog).getAllByRole("checkbox");
        fireEvent.click(ticks[1]);
        expect(within(dialog).getByRole("button", { name: "Allow these 2" })).toBeTruthy();
        fireEvent.click(ticks[0]);
        fireEvent.click(ticks[2]);
        expect((within(dialog).getByRole("button", { name: "Allow these 0" }) as HTMLButtonElement).disabled).toBe(true);
        fireEvent.click(ticks[2]);
        fireEvent.click(ticks[0]);
        fireEvent.click(within(dialog).getByRole("button", { name: "Allow these 2" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "once", rows: [0, 2] }]));
        await waitFor(() => expect(close).toHaveBeenCalled());
    });

    it("denies the whole list with no rows, and says one row as one", async () => {
        const first = render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        fireEvent.click(within(await screen.findByRole("group")).getByRole("button", { name: "Deny" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "deny" }]));
        first.unmount();

        uses = [use({ batch: batch({ region: "", profile: "", items: batch().items.slice(0, 1) }) })];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("group");
        expect(dialog.textContent).toContain("Agent1 wants to copy 1 secret from AWS Secrets Manager into your Vault: host passwords");
        expect(dialog.textContent).toContain("Read from AWS Secrets Manager, in this computer's default AWS region.");
        expect(dialog.textContent).toContain("They are read with this computer's default AWS login.");
        expect(within(dialog).getByRole("button", { name: "Allow this one" })).toBeTruthy();
    });

    it("says a container agent's profile is a grant", async () => {
        uses = [use({ batch: batch({ grant: true }) })];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        expect((await screen.findByRole("group")).textContent).toContain("They are read under the AWS grant work-read.");
    });
});

describe("a batch's lines in the Activity tab", () => {
    const line = (seq: number, action: string, label: string, refs: string[]): VaultActivity => ({
        at: seq,
        entry_id: "",
        entry_label: label,
        actor: "Agent1",
        action,
        target: "",
        outcome: "done",
        seq,
        version: 0,
        actor_kind: "agent",
        host: "",
        reason: "",
        refs,
        is_use: false,
    });

    it("puts the lines of a batch's secrets under the batch's own line, and leaves every other line where it is", () => {
        const rows = [
            line(9, "revealed", "Passport", []),
            line(8, "imported a batch", "host passwords", ["batch:aaa", "task 31007"]),
            line(7, "imported", "Beta", ["batch:aaa", "task 31007"]),
            line(6, "used", "Stripe", []),
            line(5, "imported", "Alpha", ["batch:aaa", "task 31007"]),
            line(4, "imported", "Orphan", ["batch:bbb"]),
            line(3, "imported", "Single", ["task 28071"]),
        ];
        expect(batchOf(rows[2])).toBe("aaa");
        expect(batchOf(rows[6])).toBe("");
        const groups = groupBatches(rows);
        expect(groups.map((g) => [g.row.entry_label, g.batch, g.items.map((i) => i.entry_label)])).toEqual([
            ["Passport", "", []],
            // Under the batch's own line its secrets read in the approval's order, oldest first.
            ["host passwords", "aaa", ["Alpha", "Beta"]],
            ["Stripe", "", []],
            // Its batch's own line is older than the list goes back: the line stays a line.
            ["Orphan", "", []],
            ["Single", "", []],
        ]);
    });

    it("says what became of a line whose action alone does not, so a failed import is not read as one that worked", () => {
        expect(outcomeOf("done")).toBeNull();
        expect(outcomeOf("")).toBeNull();
        expect(outcomeOf("failed")).toEqual({ text: "failed", warn: true });
        expect(outcomeOf("denied")?.warn).toBe(true);
        expect(outcomeOf("partly done")?.warn).toBe(true);
        expect(outcomeOf("same")).toEqual({ text: "same", warn: false });
        // A check of a linked entry that found its place there is good news; one that found it changed or gone is not.
        expect(outcomeOf("present")).toEqual({ text: "present", warn: false });
        expect(outcomeOf("missing: AWS has no secret by that id, in that region")?.warn).toBe(true);
    });
});
