// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** An entry's machine copy in the pane: the line under an entry's details and its Check button, the
 *  form's inputs (and that a save never drops the link), and the question an agent's change of place
 *  asks. Every place named here is invented. */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

let uses: UseRequest[] = [];
let entries: VaultEntryInfo[] = [];
/** Set to have srv refuse the next checks with this. */
let checkRefusal = "";
const vault = vi.fn(async (op: string, _args?: Record<string, unknown>): Promise<unknown> => {
    if (op === "list") return { entries, problem: null, requests: [], uses, backend: "keychain" };
    if (op === "check_source") {
        if (checkRefusal) throw new Error(checkRefusal);
        return { checked: { id: "api-key-staging", label: "Staging database", outcome: "same", source: "aws-secret:staging/app#db.password", checked_at: 1 } };
    }
    return {};
});
vi.mock("@/util/app-api", () => ({ getApi: () => ({ approvals: { vault } }) }));

import { VaultEntries } from "./vault-entries";
import { VaultEntryForm } from "./vault-entry-form";
import { VaultUseItem as UsePrompt } from "./vault-requests-list";
import { machineCopy, offersSource, sourceCheck, sourceFields } from "./vault-source";
import { refreshVault } from "./vault-store";

const AWS = { source_kind: "aws-secret", source_location: "staging/app", source_key: "db.password", source_region: "eu-west-1" };

const entry = (over: Partial<VaultEntryInfo> = {}): VaultEntryInfo => ({
    id: "api-key-staging",
    kind: "api-key",
    section: "api-keys",
    label: "Staging database",
    vault_id: "personal",
    fields: { service: "Example", ...AWS },
    tags: [],
    parts_set: ["value"],
    grants: [],
    tail: "k4vz",
    versions: 0,
    created_at: 1,
    updated_at: 1,
    last_used_at: 0,
    ...over,
});

const calls = (op: string) => vault.mock.calls.filter((c) => c[0] === op).map((c) => c[1] ?? {});
const row = (id: string) => document.querySelector(`tr[data-entry="${id}"]`) as HTMLElement;

beforeEach(() => {
    vault.mockClear();
    uses = [];
    entries = [];
    checkRefusal = "";
});
afterEach(cleanup);

describe("the link's fields and what a check found", () => {
    it("saves no link without a kind, and a region and a profile only for an AWS secret", () => {
        expect(sourceFields({ service: "Example", ...AWS, source_profile: " work-read " })).toEqual({ ...AWS, source_profile: "work-read" });
        expect(sourceFields({ source_kind: "", source_location: "staging/app" })).toEqual({});
        expect(sourceFields({ source_kind: "github-secret", source_location: "example-org/example-app", source_key: "DEPLOY_TOKEN", source_region: "eu-west-1" })).toEqual({
            source_kind: "github-secret",
            source_location: "example-org/example-app",
            source_key: "DEPLOY_TOKEN",
        });
    });

    it("names the place the way a person reads it", () => {
        // An AWS place names its region and the profile it is read with: the same name elsewhere is another secret.
        expect(machineCopy(AWS)).toBe("AWS secret staging/app, key db.password, region eu-west-1");
        expect(machineCopy({ ...AWS, source_profile: "readonly" })).toBe("AWS secret staging/app, key db.password, region eu-west-1, profile readonly");
        expect(machineCopy({ source_kind: "aws-secret", source_location: "staging/app" })).toBe("AWS secret staging/app");
        expect(machineCopy({ source_kind: "github-secret", source_location: "example-org/example-app", source_key: "DEPLOY_TOKEN" })).toBe("GitHub secret DEPLOY_TOKEN of example-org/example-app");
        expect(machineCopy({ source_kind: "other", source_location: "the office safe" })).toBe("the office safe");
        expect(machineCopy({ service: "Example" })).toBe("");
    });

    it("says what the newest check found and when, and why a place could not be read", () => {
        const now = 10 * 60_000;
        expect(sourceCheck({}, now)).toBeNull();
        expect(sourceCheck({ source_checked: { at: now - 3 * 60_000, outcome: "same" } }, now)).toEqual({ state: "same", text: "same as the Vault's value, checked 3 min ago" });
        expect(sourceCheck({ source_checked: { at: now, outcome: "different" } }, now)?.state).toBe("different");
        expect(sourceCheck({ source_checked: { at: now, outcome: "could not read: AWS refused: this identity can't read that secret" } }, now)).toEqual({
            state: "unread",
            text: "could not be read, checked just now: AWS refused: this identity can't read that secret",
        });
    });

    it("offers the inputs where a machine copy makes sense, and wherever an entry already has one", () => {
        expect(offersSource("api-keys", {})).toBe(true);
        expect(offersSource("logins", {})).toBe(true);
        expect(offersSource("cards", {})).toBe(false);
        expect(offersSource("personal", {})).toBe(false);
        expect(offersSource("cards", { source_kind: "other" })).toBe(true);
    });
});

describe("an entry's machine copy in the list", () => {
    it("shows the place and the newest check under the details, and checks it on a click", async () => {
        entries = [entry({ source_checked: { at: Date.now() - 120_000, outcome: "same" } }), entry({ id: "api-key-plain", label: "Plain key", fields: { service: "Example" } })];
        await refreshVault();
        render(() => <VaultEntries section="api-keys" />);
        const linked = row("api-key-staging");
        expect(linked.textContent).toContain("Machine copy: AWS secret staging/app, key db.password");
        expect(linked.querySelector("[data-check]")?.textContent).toContain("same as the Vault's value, checked 2 min ago");
        expect(row("api-key-plain").textContent).not.toContain("Machine copy");
        // Only an entry with a link has the button.
        const buttons = screen.getAllByRole("button", { name: "Check the machine copy" });
        expect(buttons).toHaveLength(1);
        expect(linked.contains(buttons[0])).toBe(true);
        fireEvent.click(buttons[0]);
        await waitFor(() => expect(calls("check_source")).toEqual([{ id: "api-key-staging" }]));
    });

    it("marks a copy that differs, says when one could not be read, and when none was checked yet", async () => {
        entries = [
            entry({ id: "a", label: "A", source_checked: { at: Date.now(), outcome: "different" } }),
            entry({ id: "b", label: "B", source_checked: { at: Date.now(), outcome: "could not read: AWS has no secret by that id, in that region" } }),
            entry({ id: "c", label: "C" }),
        ];
        await refreshVault();
        render(() => <VaultEntries section="api-keys" />);
        const differs = row("a").querySelector("[data-check]") as HTMLElement;
        expect(differs.dataset.check).toBe("different");
        expect(differs.classList.contains("vault-expiry-warn")).toBe(true);
        expect(differs.textContent).toContain("different from the Vault's value");
        const unread = row("b").querySelector("[data-check]") as HTMLElement;
        expect(unread.dataset.check).toBe("unread");
        // Marked like a copy that differs, and the line itself says why: nothing is behind a hover.
        expect(unread.classList.contains("vault-expiry-warn")).toBe(true);
        expect(unread.textContent).toContain("AWS has no secret by that id, in that region");
        expect(unread.hasAttribute("title")).toBe(false);
        expect(unread.textContent).toContain("could not be read");
        expect(row("c").textContent).toContain("not checked yet");
    });

    it("finds an entry by the place its copy is in, and says when a check could not start", async () => {
        entries = [entry(), entry({ id: "api-key-plain", label: "Plain key", fields: { service: "Example" } })];
        await refreshVault();
        checkRefusal = "Staging database has no machine copy recorded";
        render(() => <VaultEntries section="api-keys" />);
        fireEvent.click(screen.getByRole("button", { name: "Check the machine copy" }));
        expect((await screen.findByRole("alert")).textContent).toContain("has no machine copy recorded");
        fireEvent.input(screen.getByPlaceholderText("Filter api keys"), { target: { value: "staging/app" } });
        await waitFor(() => expect(row("api-key-plain")).toBeNull());
        expect(row("api-key-staging")).not.toBeNull();
    });
});

describe("the machine copy in the entry form", () => {
    const save = () => fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    const fieldsSent = async () => {
        await waitFor(() => expect(calls("update")).toHaveLength(1));
        return calls("update")[0].fields as Record<string, string>;
    };

    it("keeps the link when another detail is changed", async () => {
        render(() => <VaultEntryForm target={{ mode: "edit", entry: entry() }} onClose={() => {}} />);
        expect((screen.getByLabelText("Secret") as HTMLInputElement).value).toBe("staging/app");
        expect((screen.getByLabelText("Key in the secret") as HTMLInputElement).value).toBe("db.password");
        expect((screen.getByLabelText("Region") as HTMLInputElement).value).toBe("eu-west-1");
        fireEvent.input(screen.getByLabelText("Service"), { target: { value: "Example DB" } });
        save();
        expect(await fieldsSent()).toEqual({ service: "Example DB", ...AWS });
    });

    it("takes the link off when the kind of place is set to None", async () => {
        render(() => <VaultEntryForm target={{ mode: "edit", entry: entry() }} onClose={() => {}} />);
        fireEvent.change(screen.getByLabelText("Machine copy"), { target: { value: "" } });
        expect(screen.queryByLabelText("Secret")).toBeNull();
        save();
        expect(await fieldsSent()).toEqual({ service: "Example" });
    });

    it("asks a GitHub secret for a repository and a name, and saves no region with it", async () => {
        render(() => <VaultEntryForm target={{ mode: "edit", entry: entry() }} onClose={() => {}} />);
        fireEvent.change(screen.getByLabelText("Machine copy"), { target: { value: "github-secret" } });
        expect(screen.queryByLabelText("Region")).toBeNull();
        fireEvent.input(screen.getByLabelText("Repository"), { target: { value: "example-org/example-app" } });
        fireEvent.input(screen.getByLabelText("Secret name"), { target: { value: "DEPLOY_TOKEN" } });
        save();
        expect(await fieldsSent()).toEqual({ service: "Example", source_kind: "github-secret", source_location: "example-org/example-app", source_key: "DEPLOY_TOKEN" });
    });

    it("leaves a card's form as it was, and still keeps a link an agent recorded on one", async () => {
        const card = { id: "card-company", kind: "card", section: "cards" as const, label: "Company card", parts_set: ["number"] };
        const first = render(() => <VaultEntryForm target={{ mode: "edit", entry: entry({ ...card, fields: {} }) }} onClose={() => {}} />);
        expect(screen.queryByLabelText("Machine copy")).toBeNull();
        first.unmount();
        render(() => <VaultEntryForm target={{ mode: "edit", entry: entry({ ...card, fields: { source_kind: "other", source_location: "the office safe" } }) }} onClose={() => {}} />);
        expect((screen.getByLabelText("Where") as HTMLInputElement).value).toBe("the office safe");
        save();
        expect(await fieldsSent()).toEqual({ source_kind: "other", source_location: "the office safe" });
    });
});

describe("the question before an agent changes where a machine copy is", () => {
    const use = (link: { from: string; to: string }): UseRequest => ({
        id: "use-1",
        agent: "Agent1",
        entry_id: "api-key-staging",
        entry_label: "Staging database",
        what: "machine copy",
        origin: "",
        page: "",
        field: "",
        offer_always: false,
        reason: "an agent asked to change where this entry's machine copy is",
        agent_reason: "record where the staging copy is for task 28071",
        needs_value: false,
        kind: "api-key",
        command: "",
        cwd: "",
        replaces: false,
        signin: false,
        account: false,
        change: false,
        requested_ms: 1,
        block_id: "pane-1",
        deadline_ms: Date.now() + 120_000,
        expires_ms: Date.now() + 120_000,
        goes_ahead: false,
        link: { about_from: "", about_to: "", only: false, ...link },
    });
    const shown = async (link: { from: string; to: string }) => {
        uses = [use(link)];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        return await screen.findByRole("group");
    };

    it("shows the place now and the place asked for, waits for a yes and offers no Always", async () => {
        const card = await shown({ from: "the AWS secret staging/app, key db.password", to: "the AWS secret production/app, key db.password" });
        expect(card.textContent).toContain("Agent1 wants to change where the machine copy of Staging database is");
        expect(card.querySelector('[data-link="from"]')?.textContent).toContain("the AWS secret staging/app, key db.password");
        expect(card.querySelector('[data-link="to"]')?.textContent).toContain("the AWS secret production/app, key db.password");
        expect(card.textContent).toContain("record where the staging copy is for task 28071");
        expect(card.textContent).toContain("the place is not changed without a yes");
        expect(card.textContent).not.toContain("Goes ahead in");
        expect(screen.getByRole("button", { name: "Allow once" })).toBeTruthy();
        expect(screen.getByRole("button", { name: "Deny" })).toBeTruthy();
        expect(screen.queryByRole("button", { name: /Always/ })).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: "Deny" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([expect.objectContaining({ id: "use-1", decision: "deny" })]));
    });

    it("says so when there was no place before, and when the record is being removed", async () => {
        const first = await shown({ from: "", to: "the GitHub secret DEPLOY_TOKEN of example-org/example-app" });
        expect(first.textContent).toContain("Agent1 wants to record where the machine copy of Staging database is");
        expect(first.querySelector('[data-link="from"]')?.textContent).toContain("no place is recorded");
        cleanup();
        const second = await shown({ from: "the office safe", to: "" });
        expect(second.textContent).toContain("Agent1 wants to remove where the machine copy of Staging database is");
        expect(second.querySelector('[data-link="to"]')?.textContent).toContain("no place (the record is removed)");
    });
});
