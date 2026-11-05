// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// JobEditor on its own, against a fake model: the loading and failure states of Edit and Clone, the
// template list and its approval, what is passed down to the form's sections and the dry run, the
// busy and failure state around Save and Dry run, and how a click on the buttons is gated.
// job-editor.test.tsx covers the save, edit, clone, dry run and update paths through the whole section.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";

type ParamsProps = Parameters<typeof import("./job-params-form").JobParamsForm>[0];
type WhatProps = Parameters<typeof import("./job-editor-fields").WhatSection>[0];
type PolicyProps = Parameters<typeof import("./job-editor-fields").PolicySection>[0];

const h = vi.hoisted(() => ({
    agents: [] as { id: string; name: string; is_seeded: number; last_active_at?: number }[],
    vault: { available: true, entries: undefined as { id: string; label: string }[] | undefined },
    refreshVault: vi.fn(),
    params: [] as ParamsProps[],
    what: [] as WhatProps[],
    policy: [] as PolicyProps[],
}));

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/view/agent/use-agent-list", () => ({
    useAgentList: () => [() => h.agents, () => null],
}));
vi.mock("@/app/view/vault/vault-store", () => ({
    vaultAvailable: () => h.vault.available,
    vaultEntries: () => h.vault.entries,
    refreshVault: (...a: unknown[]) => h.refreshVault(...a),
}));
vi.mock("./job-params-form", async (orig) => {
    const a = await orig<typeof import("./job-params-form")>();
    return {
        ...a,
        JobParamsForm: (p: ParamsProps) => {
            h.params.push(p);
            return a.JobParamsForm(p);
        },
    };
});
vi.mock("./job-editor-fields", async (orig) => {
    const a = await orig<typeof import("./job-editor-fields")>();
    return {
        ...a,
        WhatSection: (p: WhatProps) => {
            h.what.push(p);
            return a.WhatSection(p);
        },
        PolicySection: (p: PolicyProps) => {
            h.policy.push(p);
            return a.PolicySection(p);
        },
    };
});

import { JobEditor, type EditorStart } from "./job-editor";
import { DEFAULTS, mkDryRun, mkTemplate } from "./jobs-fixtures";
import type { JobsModel } from "./jobs-model";

type Deferred<T> = { promise: Promise<T>; resolve: (v: T) => void; reject: (e: unknown) => void };
function deferred<T>(): Deferred<T> {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
}

const templatesOf = (...templates: ReturnType<typeof mkTemplate>[]) => ({ templates, publishers: [], catalogs: [] });
const saved = { ok: true, job_id: "new-1", notes: [], needs_confirmation: null, provider: null, error: null };
const STORED = {
    jobVersion: 1,
    id: "j1",
    name: "Nightly build",
    owner: "uid-b",
    params: {},
    trigger: { kind: "cron", cron: "0 6 * * *", tz: "UTC" },
    action: { kind: "message", to: "uid-b", brief: "Build it." },
    policy: { overlap: "skip", catch_up: "skip", retries: 0 },
    outputs: [],
    secrets: [],
    enabled: true,
};
const FROM_TEMPLATE = {
    ...STORED,
    template: { id: "agentmux.weekly-report", version: "0.9.0", hash: "hash-1" },
    params: { repo: "acme/site" },
    action: { kind: "message", to: "uid-b" },
};

const model = {
    templates: vi.fn(),
    get: vi.fn(),
    save: vi.fn(),
    dryRun: vi.fn(),
    approveTemplate: vi.fn(),
};
const onClose = vi.fn();

function mount(start: EditorStart = { mode: "new" }, defaults = DEFAULTS) {
    render(() => (
        <JobEditor start={start} model={model as unknown as JobsModel} defaults={defaults} onClose={onClose} />
    ));
}
const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
    h.agents = [{ id: "uid-b", name: "Builder", is_seeded: 0 }];
    h.vault = { available: true, entries: [{ id: "v-1", label: "Repo token" }] };
    h.params = [];
    h.what = [];
    h.policy = [];
    model.templates.mockResolvedValue(templatesOf());
    model.get.mockResolvedValue({ definition: STORED });
    model.save.mockResolvedValue(saved);
    model.dryRun.mockResolvedValue(mkDryRun());
    model.approveTemplate.mockResolvedValue(templatesOf());
});
afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

const type = (label: string | RegExp, value: string) =>
    fireEvent.input(screen.getByLabelText(label), { target: { value } });
const btn = (name: string | RegExp) => screen.getByRole("button", { name }) as HTMLButtonElement;
const region = (name: string | RegExp) => screen.queryByRole("region", { name });

async function fillBlank() {
    await screen.findByLabelText("Name");
    type("Name", "Morning check");
    fireEvent.change(screen.getByLabelText("Message"), { target: { value: "uid-b" } });
    type("Brief", "Look at the board.");
}

// ── Heading and loading ───────────────────────────────────────────────────

describe("heading and loading", () => {
    it("names the section New job, Clone job or Edit <name>, with a heading to match", async () => {
        mount({ mode: "new" });
        expect(region("New job")).not.toBeNull();
        expect(screen.getByRole("heading", { name: "New job" })).toBeInTheDocument();
        cleanup();
        mount({ mode: "clone", jobId: "j1" });
        expect(screen.getByRole("heading", { name: "Clone job" })).toBeInTheDocument();
        expect(region("Clone job")).not.toBeNull();
        cleanup();
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect(screen.getByRole("heading", { name: "Edit Nightly build" })).toBeInTheDocument());
        expect(region("Edit Nightly build")).not.toBeNull();
    });

    it("calls an edit 'Edit job' until the job's name has loaded", async () => {
        const d = deferred<unknown>();
        model.get.mockReturnValue(d.promise);
        mount({ mode: "edit", jobId: "j1" });
        await flush();
        expect(screen.getByRole("heading", { name: "Edit job" })).toBeInTheDocument();
        d.resolve({ definition: STORED });
        await waitFor(() => expect(screen.getByRole("heading", { name: "Edit Nightly build" })).toBeInTheDocument());
    });

    it("shows Loading… and no form or enabled buttons while an edit loads, then the form", async () => {
        const d = deferred<unknown>();
        model.get.mockReturnValue(d.promise);
        mount({ mode: "edit", jobId: "j1" });
        await flush();
        expect(screen.getByText("Loading…")).toBeInTheDocument();
        expect(screen.queryByLabelText("Name")).toBeNull();
        expect(btn(/Save changes|Create job/).disabled).toBe(true);
        expect(btn("Dry run").disabled).toBe(true);
        expect(model.get).toHaveBeenCalledWith("j1");
        d.resolve({ definition: STORED });
        await waitFor(() => expect(screen.queryByText("Loading…")).toBeNull());
        expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Nightly build");
        expect(btn("Save changes").disabled).toBe(false);
        expect(btn("Dry run").disabled).toBe(false);
    });

    it("shows Loading… and no form while a clone loads", async () => {
        const d = deferred<unknown>();
        model.get.mockReturnValue(d.promise);
        mount({ mode: "clone", jobId: "j1" });
        await flush();
        expect(screen.getByText("Loading…")).toBeInTheDocument();
        expect(screen.queryByLabelText("Name")).toBeNull();
        expect(btn("Create job").disabled).toBe(true);
        d.resolve({ definition: STORED });
        await waitFor(() => expect(screen.queryByText("Loading…")).toBeNull());
        expect(screen.getByLabelText("Name")).toBeInTheDocument();
    });

    it("loads a clone the same way, as a new job named as a copy", async () => {
        mount({ mode: "clone", jobId: "j1" });
        await waitFor(() =>
            expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Nightly build (copy)")
        );
        expect(btn("Create job")).toBeInTheDocument();
        expect(model.get).toHaveBeenCalledWith("j1");
    });

    it("loads nothing and shows the form at once for a new job", async () => {
        mount({ mode: "new" });
        expect(screen.queryByText("Loading…")).toBeNull();
        expect(screen.getByLabelText("Name")).toBeInTheDocument();
        await flush();
        expect(model.get).not.toHaveBeenCalled();
        expect(model.templates).toHaveBeenCalledTimes(1);
    });

    it("shows why the job couldn't be read, and stops loading", async () => {
        model.get.mockRejectedValue(new Error("no such job"));
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("Error: no such job"));
        expect(screen.queryByText("Loading…")).toBeNull();
    });

    it("shows why the templates couldn't be listed, and still loads the job", async () => {
        model.templates.mockRejectedValue(new Error("srv down"));
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Nightly build"));
        expect(screen.getByRole("alert").textContent).toBe("Error: srv down");
    });

    it("shows templates' failure on a new job too, with no job fetched", async () => {
        model.templates.mockRejectedValue(new Error("srv down"));
        mount({ mode: "new" });
        await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("Error: srv down"));
    });
});

// ── Vault ────────────────────────────────────────────────────────────────

describe("the Vault", () => {
    it("asks the Vault for its entries once when it can be read", async () => {
        mount({ mode: "new" });
        await flush();
        expect(h.refreshVault).toHaveBeenCalledTimes(1);
    });

    it("leaves the Vault alone and passes null when it can't be read here", async () => {
        h.vault = { available: false, entries: [{ id: "v-1", label: "Repo token" }] };
        model.templates.mockResolvedValue(templatesOf(mkTemplate()));
        model.get.mockResolvedValue({ definition: FROM_TEMPLATE });
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect(h.params.length).toBeGreaterThan(0));
        expect(h.refreshVault).not.toHaveBeenCalled();
        expect(h.params.at(-1)!.vault).toBeNull();
    });

    it("passes the entries to the settings form, or an empty list while none are known", async () => {
        model.templates.mockResolvedValue(templatesOf(mkTemplate()));
        model.get.mockResolvedValue({ definition: FROM_TEMPLATE });
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect(h.params.length).toBeGreaterThan(0));
        expect(h.params.at(-1)!.vault).toEqual([{ id: "v-1", label: "Repo token" }]);
        cleanup();
        h.params = [];
        h.vault = { available: true, entries: undefined };
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect(h.params.length).toBeGreaterThan(0));
        expect(h.params.at(-1)!.vault).toEqual([]);
    });

    it("tells the dry run which Vault entries exist, so a missing one is marked", async () => {
        model.dryRun.mockResolvedValue(mkDryRun({ secrets: [{ name: "REPO_TOKEN", vault: "v-gone" }] }));
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Dry run"));
        const panel = await screen.findByRole("region", { name: "Dry run" });
        expect(panel.textContent).toContain("not in the Vault");
        cleanup();
        h.vault = { available: false, entries: [] };
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Dry run"));
        const again = await screen.findByRole("region", { name: "Dry run" });
        expect(again.textContent).toContain("Vault entry v-gone");
        expect(again.textContent).not.toContain("not in the Vault");
    });
});

// ── Agents ───────────────────────────────────────────────────────────────

describe("agents", () => {
    it("offers the unseeded agents, most recently used first, by id and name, to the action section", async () => {
        h.agents = [
            { id: "a-old", name: "Zed", is_seeded: 0, last_active_at: 100 },
            { id: "a-seed", name: "Seeded", is_seeded: 1, last_active_at: 900 },
            { id: "a-new", name: "Amy", is_seeded: 0, last_active_at: 500 },
        ];
        mount({ mode: "new" });
        await flush();
        expect(h.what.at(-1)!.agents).toEqual([
            { value: "a-new", label: "Amy" },
            { value: "a-old", label: "Zed" },
        ]);
    });

    it("tells the action section whether the job is made from a template", async () => {
        mount({ mode: "new" });
        await flush();
        expect(h.what.at(-1)!.templated).toBe(false);
        cleanup();
        model.templates.mockResolvedValue(templatesOf(mkTemplate()));
        model.get.mockResolvedValue({ definition: FROM_TEMPLATE });
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect(h.what.at(-1)!.templated).toBe(true));
    });

    it("hands the settings' caps to the policy section", async () => {
        mount({ mode: "new" }, { ...DEFAULTS, run_usd: 7 });
        await flush();
        expect(h.policy.at(-1)!.defaults!.run_usd).toBe(7);
    });

    it("names the dry run's target by agent name, or by its id when it isn't listed", async () => {
        model.dryRun.mockResolvedValueOnce(mkDryRun({ target: "uid-b" }));
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Dry run"));
        expect((await screen.findByRole("region", { name: "Dry run" })).textContent).toContain("→ Builder");
        model.dryRun.mockResolvedValueOnce(mkDryRun({ target: "uid-gone" }));
        fireEvent.click(btn("Dry run"));
        await waitFor(() =>
            expect(screen.getByRole("region", { name: "Dry run" }).textContent).toContain("→ uid-gone")
        );
    });
});

// ── Name and owner ───────────────────────────────────────────────────────

describe("name and owner", () => {
    it("limits the name to 200 characters", async () => {
        mount({ mode: "new" });
        expect((screen.getByLabelText("Name") as HTMLInputElement).maxLength).toBe(200);
    });

    it("explains who gets the alerts, and lists no agent first, then the agents", async () => {
        mount({ mode: "new" });
        await flush();
        expect(screen.getByText("Its leader gets the failure alerts.")).toBeInTheDocument();
        const owner = screen.getByLabelText("Owner") as HTMLSelectElement;
        expect(Array.from(owner.options).map((o) => [o.value, o.textContent])).toEqual([
            ["", "You (no agent)"],
            ["uid-b", "Builder"],
        ]);
    });

    it("saves the owner picked, and keeps an owner that is no longer listed as an option", async () => {
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.change(screen.getByLabelText("Owner"), { target: { value: "uid-b" } });
        fireEvent.click(btn("Create job"));
        await waitFor(() => expect(model.save).toHaveBeenCalledTimes(1));
        expect(model.save.mock.calls[0][0].definition.owner).toBe("uid-b");
        cleanup();
        model.get.mockResolvedValue({ definition: { ...STORED, owner: "uid-gone" } });
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect((screen.getByLabelText("Owner") as HTMLSelectElement).value).toBe("uid-gone"));
        const options = Array.from((screen.getByLabelText("Owner") as HTMLSelectElement).options);
        expect(options.map((o) => o.textContent)).toEqual(["You (no agent)", "Builder", "uid-gone"]);
    });

    it("shows the owner a job has", async () => {
        model.get.mockResolvedValue({ definition: STORED });
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect((screen.getByLabelText("Owner") as HTMLSelectElement).value).toBe("uid-b"));
    });
});

// ── Start from ───────────────────────────────────────────────────────────

describe("start from", () => {
    it("offers a blank job or a template, stacked, on a new job only", async () => {
        mount({ mode: "new" });
        const field = screen.getByText("Start from").closest(".ui-field")!;
        expect(field.classList.contains("ui-field--stacked")).toBe(true);
        expect(
            (screen.getByRole("radio", { name: "A blank job" }) as HTMLInputElement).getAttribute("aria-checked")
        ).toBe("true");
        expect(screen.getByRole("radio", { name: "A template" })).toBeInTheDocument();
        cleanup();
        mount({ mode: "edit", jobId: "j1" });
        await flush();
        expect(screen.queryByText("Start from")).toBeNull();
        cleanup();
        mount({ mode: "clone", jobId: "j1" });
        await flush();
        expect(screen.queryByText("Start from")).toBeNull();
    });

    it("lists no template while the blank job is chosen, even when some are installed", async () => {
        model.templates.mockResolvedValue(templatesOf(mkTemplate()));
        mount({ mode: "new" });
        await flush();
        expect(screen.queryByText("Weekly report")).toBeNull();
    });

    it("says when no template is installed", async () => {
        mount({ mode: "new" });
        fireEvent.click(screen.getByRole("radio", { name: "A template" }));
        expect(await screen.findByText("No templates are installed.")).toBeInTheDocument();
    });

    it("hides the form, and disables Create and Dry run, until a template is cloned", async () => {
        model.templates.mockResolvedValue(templatesOf(mkTemplate()));
        mount({ mode: "new" });
        fireEvent.click(screen.getByRole("radio", { name: "A template" }));
        await screen.findByText("Weekly report");
        expect(screen.queryByLabelText("Name")).toBeNull();
        expect(btn("Create job").disabled).toBe(true);
        expect(btn("Dry run").disabled).toBe(true);
        fireEvent.click(btn(/Clone to a job/));
        await waitFor(() => expect(screen.getByLabelText("Name")).toBeInTheDocument());
        expect(screen.queryByText("Weekly report", { selector: "strong" })).not.toBeNull();
        expect(screen.queryByRole("list")).toBeNull();
        expect(btn("Create job").disabled).toBe(false);
        expect(btn("Dry run").disabled).toBe(false);
    });

    it("goes back to the list, with a blank form behind it, when blank is chosen and then a template again", async () => {
        model.templates.mockResolvedValue(templatesOf(mkTemplate()));
        mount({ mode: "new" });
        fireEvent.click(screen.getByRole("radio", { name: "A template" }));
        fireEvent.click(await screen.findByRole("button", { name: /Clone to a job/ }));
        await waitFor(() => expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Weekly report"));
        fireEvent.click(screen.getByRole("radio", { name: "A blank job" }));
        expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("");
        expect(screen.queryByText(/Made from/)).toBeNull();
        fireEvent.click(screen.getByRole("radio", { name: "A template" }));
        expect(screen.getByText("Weekly report", { selector: "strong" })).toBeInTheDocument();
        expect(screen.queryByLabelText("Name")).toBeNull();
    });

    it("clears a dry run when the choice changes", async () => {
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Dry run"));
        await screen.findByRole("region", { name: "Dry run" });
        fireEvent.click(screen.getByRole("radio", { name: "A template" }));
        expect(region("Dry run")).toBeNull();
        fireEvent.click(screen.getByRole("radio", { name: "A blank job" }));
        expect(region("Dry run")).toBeNull();
    });

    it("clears the fields typed when the choice changes", async () => {
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(screen.getByRole("radio", { name: "A template" }));
        fireEvent.click(screen.getByRole("radio", { name: "A blank job" }));
        expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("");
    });
});

// ── The template list ────────────────────────────────────────────────────

describe("the template list", () => {
    async function list(...templates: ReturnType<typeof mkTemplate>[]) {
        model.templates.mockResolvedValue(templatesOf(...templates));
        mount({ mode: "new" });
        fireEvent.click(screen.getByRole("radio", { name: "A template" }));
        await screen.findAllByRole("listitem");
    }
    const item = (text: string) => screen.getByText(text, { selector: "strong" }).closest("li")!;

    it.each([
        ["approved", "Approved", "ok"],
        ["needs_approval", "Needs your approval", "warn"],
        ["changed", "Changed since approved", "warn"],
        ["invalid", "Can't be read", "error"],
        ["from-a-newer-srv", "Can't be read", "error"],
    ])("shows a template in state %s as %s", async (state, label, tone) => {
        await list(mkTemplate({ state: state as JobTemplateInfo["state"] }));
        const pill = within(item("Weekly report")).getByText(label);
        expect(pill.classList.contains(`ui-pill--${tone}`)).toBe(true);
    });

    it("shows a template's name, version and description", async () => {
        await list(mkTemplate());
        const li = item("Weekly report");
        expect(within(li).getByText("1.0.0")).toBeInTheDocument();
        expect(within(li).getByText("Reads repositories since the last run and reports.")).toBeInTheDocument();
    });

    it("shows an unreadable template by its id with the error where the description goes", async () => {
        await list(
            mkTemplate({ id: "acme.broken", state: "invalid", source: "installed", manifest: null, error: "bad json" })
        );
        const li = item("acme.broken");
        expect(within(li).getByText("bad json")).toBeInTheDocument();
        expect(li.querySelector("span.warden-manager-dim")!.textContent).toBe("");
    });

    it("marks the templates AgentMux ships, and no others", async () => {
        await list(mkTemplate(), mkTemplate({ id: "acme.x", source: "installed" }));
        const items = screen.getAllByRole("listitem");
        expect(within(items[0]).getByText("AgentMux")).toBeInTheDocument();
        expect(within(items[1]).queryByText("AgentMux")).toBeNull();
    });

    it("offers cloning an approved template and reviewing a changed or unapproved one, and nothing else", async () => {
        await list(
            mkTemplate({ id: "t.ok", state: "approved", manifest: { ...mkTemplate().manifest!, name: "Ok" } }),
            mkTemplate({ id: "t.new", state: "needs_approval", manifest: { ...mkTemplate().manifest!, name: "New" } }),
            mkTemplate({ id: "t.chg", state: "changed", manifest: { ...mkTemplate().manifest!, name: "Chg" } }),
            mkTemplate({ id: "t.bad", state: "invalid", manifest: null, error: "x" })
        );
        const names = (li: HTMLElement) =>
            within(li)
                .queryAllByRole("button")
                .map((b) => b.textContent?.trim());
        expect(names(item("Ok"))).toEqual(["Clone to a job"]);
        expect(names(item("New"))).toEqual(["Review and approve"]);
        expect(names(item("Chg"))).toEqual(["Review and approve"]);
        expect(names(item("t.bad"))).toEqual([]);
    });
});

// ── Approving a template ─────────────────────────────────────────────────

describe("approving a template", () => {
    async function review(over: Partial<ReturnType<typeof mkTemplate>> = {}) {
        model.templates.mockResolvedValue(templatesOf(mkTemplate({ state: "needs_approval", ...over })));
        mount({ mode: "new" });
        fireEvent.click(screen.getByRole("radio", { name: "A template" }));
        fireEvent.click(await screen.findByRole("button", { name: /Review and approve/ }));
        return screen.findByRole("region", { name: /^Approve/ });
    }

    it("replaces the rest of the editor with the approval, and Cancel brings the list back", async () => {
        const panel = await review();
        expect(screen.queryByText("Start from")).toBeNull();
        expect(screen.queryByRole("button", { name: /Create job/ })).toBeNull();
        fireEvent.click(within(panel).getByRole("button", { name: "Cancel" }));
        expect(region(/^Approve/)).toBeNull();
        expect(screen.getByText("Start from")).toBeInTheDocument();
        expect(screen.getByText("Weekly report", { selector: "strong" })).toBeInTheDocument();
    });

    it("approves the template's id and hash with the publisher's key", async () => {
        const panel = await review({
            source: "installed",
            signature: { state: "signed", publisher: "acme", fingerprint: "ABCD-1234", pinned: "ABCD-1234" },
        });
        fireEvent.click(within(panel).getByRole("button", { name: /^Approve$/ }));
        await waitFor(() =>
            expect(model.approveTemplate).toHaveBeenCalledWith({
                id: "agentmux.weekly-report",
                hash: "hash-1",
                signer: "ABCD-1234",
            })
        );
    });

    it("shows the new template list once approved, and leaves the editor usable", async () => {
        model.approveTemplate.mockResolvedValue(templatesOf(mkTemplate()));
        const panel = await review();
        fireEvent.click(within(panel).getByRole("button", { name: /^Approve$/ }));
        fireEvent.click(await screen.findByRole("button", { name: /Clone to a job/ }));
        await waitFor(() => expect(screen.getByLabelText("Name")).toBeInTheDocument());
        expect(btn("Create job").disabled).toBe(false);
        expect(btn("Dry run").disabled).toBe(false);
    });

    it("shows the approval as busy while it is sent, and can't be cancelled meanwhile", async () => {
        const d = deferred<unknown>();
        model.approveTemplate.mockReturnValue(d.promise);
        const panel = await review();
        fireEvent.click(within(panel).getByRole("button", { name: /^Approve$/ }));
        await waitFor(() =>
            expect(
                within(panel)
                    .getByRole("button", { name: /^Approve$/ })
                    .getAttribute("aria-busy")
            ).toBe("true")
        );
        expect((within(panel).getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled).toBe(true);
        d.resolve(templatesOf(mkTemplate()));
        await waitFor(() => expect(region(/^Approve/)).toBeNull());
    });

    it("shows a refusal, stops being busy, and clears the refusal when it is tried again", async () => {
        const d = deferred<unknown>();
        model.approveTemplate
            .mockRejectedValueOnce(new Error("changed since you were asked"))
            .mockReturnValueOnce(d.promise);
        const panel = await review();
        fireEvent.click(within(panel).getByRole("button", { name: /^Approve$/ }));
        await waitFor(() =>
            expect(within(panel).getByRole("alert").textContent).toBe("Error: changed since you were asked")
        );
        expect(
            within(panel)
                .getByRole("button", { name: /^Approve$/ })
                .getAttribute("aria-busy")
        ).toBeNull();
        expect((within(panel).getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled).toBe(false);
        fireEvent.click(within(panel).getByRole("button", { name: /^Approve$/ }));
        await waitFor(() => expect(within(panel).queryByRole("alert")).toBeNull());
        d.resolve(templatesOf(mkTemplate()));
    });
});

// ── Settings of a template's job ─────────────────────────────────────────

describe("a job made from a template", () => {
    it("says which template and version it was made from, and shows its settings", async () => {
        const other = mkTemplate({
            id: "other.one",
            manifest: {
                ...mkTemplate().manifest!,
                id: "other.one",
                name: "Other",
                params: { type: "object", properties: { other: { type: "string", title: "Other" } }, required: [] },
            },
        });
        model.templates.mockResolvedValue(templatesOf(other, mkTemplate()));
        model.get.mockResolvedValue({ definition: FROM_TEMPLATE });
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect(screen.getByText(/Made from/)).toBeInTheDocument());
        const note = screen.getByText(/Made from/);
        expect(note.textContent).toBe(
            "Made from Weekly report 0.9.0. The template writes the brief; its settings are below."
        );
        expect(h.params.at(-1)!.manifest.id).toBe("agentmux.weekly-report");
        expect(h.params.at(-1)!.values).toMatchObject({ repo: "acme/site" });
        expect(screen.queryByText(/isn't installed any more/)).toBeNull();
    });

    it("falls back to the template's own version when the job doesn't record one", async () => {
        model.templates.mockResolvedValue(templatesOf(mkTemplate()));
        model.get.mockResolvedValue({
            definition: { ...FROM_TEMPLATE, template: { id: "agentmux.weekly-report", hash: "hash-1" } },
        });
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect(screen.getByText(/Made from/)).toBeInTheDocument());
        expect(screen.getByText(/Made from/).textContent).toContain("Weekly report 1.0.0.");
    });

    it("says its template isn't installed any more, and names it by id", async () => {
        model.get.mockResolvedValue({ definition: FROM_TEMPLATE });
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect(screen.getByText(/isn't installed any more/)).toBeInTheDocument());
        expect(
            screen.getByText("This job's template isn't installed any more, so its settings stay as they are.")
                .className
        ).toContain("warden-manager-error");
        expect(screen.getByText(/Made from/).textContent).toContain("agentmux.weekly-report 0.9.0.");
        expect(h.params).toHaveLength(0);
    });

    it("shows neither note for a job with no template", async () => {
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Nightly build"));
        expect(screen.queryByText(/Made from/)).toBeNull();
        expect(screen.queryByText(/isn't installed any more/)).toBeNull();
        expect(h.params).toHaveLength(0);
    });

    it("shows no 'not installed' note on a blank new job", async () => {
        mount({ mode: "new" });
        await flush();
        expect(screen.queryByText(/isn't installed any more/)).toBeNull();
    });
});

// ── Save and Dry run ─────────────────────────────────────────────────────

describe("save", () => {
    it("shows Create job on a new job and Save changes on an edit", async () => {
        mount({ mode: "new" });
        expect(btn("Create job")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Save changes" })).toBeNull();
        cleanup();
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect(btn("Save changes")).toBeInTheDocument());
        expect(screen.queryByRole("button", { name: "Create job" })).toBeNull();
    });

    it("closes the editor on success, and on Cancel", async () => {
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Create job"));
        await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
        onClose.mockClear();
        fireEvent.click(btn("Cancel"));
        expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("keeps the editor open when a confirmation is asked for, and Not now drops the question", async () => {
        model.save.mockResolvedValue({
            ...saved,
            ok: false,
            needs_confirmation: "no_cost",
            error: "codex doesn't report cost",
        });
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Create job"));
        const note = await screen.findByText(/codex doesn't report cost/);
        expect(onClose).not.toHaveBeenCalled();
        expect(btn("Create job").disabled).toBe(false);
        expect(note.closest('[role="alert"]')).not.toBeNull();
        fireEvent.click(btn("Not now"));
        expect(screen.queryByText(/codex doesn't report cost/)).toBeNull();
    });

    it("shows the Confirm and save button as busy while the confirmed save is sent", async () => {
        const d = deferred<unknown>();
        model.save.mockResolvedValueOnce({ ...saved, ok: false, needs_confirmation: "no_cost", error: "no cost" });
        model.save.mockReturnValueOnce(d.promise);
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Create job"));
        await screen.findByText("no cost");
        fireEvent.click(btn("Confirm and save"));
        await waitFor(() => expect(btn("Confirm and save").getAttribute("aria-busy")).toBe("true"));
        d.resolve(saved);
        await waitFor(() => expect(onClose).toHaveBeenCalled());
    });

    it("is busy while sending: Create job busy, both buttons off; then back on after a refusal that shows", async () => {
        const d = deferred<unknown>();
        model.save.mockReturnValue(d.promise);
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Create job"));
        await waitFor(() => expect(btn("Create job").getAttribute("aria-busy")).toBe("true"));
        expect(btn("Create job").disabled).toBe(true);
        expect(btn("Dry run").disabled).toBe(true);
        expect(btn("Dry run").getAttribute("aria-busy")).toBeNull();
        d.reject(new Error("refused"));
        await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("Error: refused"));
        expect(btn("Create job").disabled).toBe(false);
        expect(btn("Create job").getAttribute("aria-busy")).toBeNull();
        expect(btn("Dry run").disabled).toBe(false);
    });

    it("clears an earlier failure as soon as the next save starts", async () => {
        const d = deferred<unknown>();
        model.save.mockRejectedValueOnce(new Error("first")).mockReturnValueOnce(d.promise);
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Create job"));
        await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("Error: first"));
        fireEvent.click(btn("Create job"));
        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
        d.resolve(saved);
    });

    it("clears an earlier failure when a save is attempted with a job that has problems", async () => {
        model.save.mockRejectedValueOnce(new Error("first"));
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Create job"));
        await waitFor(() => expect(screen.getByText("Error: first")).toBeInTheDocument());
        type("Name", "");
        fireEvent.click(btn("Create job"));
        await waitFor(() => expect(screen.queryByText("Error: first")).toBeNull());
        expect(screen.getByText("Give the job a name.")).toBeInTheDocument();
    });

    it("doesn't go busy, or send, when the job has problems, and lists them as alerts", async () => {
        mount({ mode: "new" });
        await screen.findByLabelText("Name");
        fireEvent.click(btn("Create job"));
        const problems = await screen.findByRole("alert");
        expect(problems.tagName).toBe("UL");
        expect(problems.querySelectorAll("li").length).toBeGreaterThan(0);
        expect(btn("Create job").getAttribute("aria-busy")).toBeNull();
        expect(btn("Create job").disabled).toBe(false);
        expect(model.save).not.toHaveBeenCalled();
    });

    it("shows no list of problems while there are none", async () => {
        mount({ mode: "new" });
        await fillBlank();
        expect(document.querySelector(".warden-jobs-problems")).toBeNull();
        fireEvent.click(btn("Create job"));
        await waitFor(() => expect(onClose).toHaveBeenCalled());
        expect(document.querySelector(".warden-jobs-problems")).toBeNull();
    });

    it("names the job when it is saved from an edit, and from a clone leaves it out", async () => {
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect(btn("Save changes")).toBeInTheDocument());
        fireEvent.click(btn("Save changes"));
        await waitFor(() => expect(model.save).toHaveBeenCalledTimes(1));
        expect(model.save.mock.calls[0][0].job_id).toBe("j1");
        expect("confirm_no_cost" in model.save.mock.calls[0][0]).toBe(false);
    });
});

describe("dry run", () => {
    it("is busy while it runs: Dry run busy, both buttons off; then back on", async () => {
        const d = deferred<unknown>();
        model.dryRun.mockReturnValue(d.promise);
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Dry run"));
        await waitFor(() => expect(btn("Dry run").getAttribute("aria-busy")).toBe("true"));
        expect(btn("Dry run").disabled).toBe(true);
        expect(btn("Create job").disabled).toBe(true);
        expect(btn("Create job").getAttribute("aria-busy")).toBeNull();
        d.resolve(mkDryRun());
        await screen.findByRole("region", { name: "Dry run" });
        expect(btn("Dry run").disabled).toBe(false);
        expect(btn("Create job").disabled).toBe(false);
    });

    it("sends the job's id from an edit, and none from a new job", async () => {
        mount({ mode: "edit", jobId: "j1" });
        await waitFor(() => expect(btn("Dry run").disabled).toBe(false));
        fireEvent.click(btn("Dry run"));
        await waitFor(() => expect(model.dryRun).toHaveBeenCalledTimes(1));
        expect(model.dryRun.mock.calls[0][0].job_id).toBe("j1");
        expect(model.dryRun.mock.calls[0][0].definition.name).toBe("Nightly build");
        cleanup();
        model.dryRun.mockClear();
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Dry run"));
        await waitFor(() => expect(model.dryRun).toHaveBeenCalledTimes(1));
        expect("job_id" in model.dryRun.mock.calls[0][0]).toBe(false);
    });

    it("shows what was refused, and goes back to idle", async () => {
        model.dryRun.mockRejectedValue(new Error("dry down"));
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Dry run"));
        await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("Error: dry down"));
        expect(btn("Dry run").disabled).toBe(false);
        expect(btn("Dry run").getAttribute("aria-busy")).toBeNull();
        expect(btn("Create job").disabled).toBe(false);
    });

    it("clears an earlier failure as soon as the next dry run starts", async () => {
        const d = deferred<unknown>();
        model.dryRun.mockRejectedValueOnce(new Error("first")).mockReturnValueOnce(d.promise);
        mount({ mode: "new" });
        await fillBlank();
        fireEvent.click(btn("Dry run"));
        await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("Error: first"));
        fireEvent.click(btn("Dry run"));
        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
        d.resolve(mkDryRun());
    });

    it("doesn't go busy, or run, when the job has problems", async () => {
        mount({ mode: "new" });
        await screen.findByLabelText("Name");
        fireEvent.click(btn("Dry run"));
        await screen.findByRole("alert");
        expect(model.dryRun).not.toHaveBeenCalled();
        expect(btn("Dry run").getAttribute("aria-busy")).toBeNull();
        expect(btn("Dry run").disabled).toBe(false);
    });

    it("shows nothing for a dry run until one is run", async () => {
        mount({ mode: "new" });
        await flush();
        expect(region("Dry run")).toBeNull();
    });
});
