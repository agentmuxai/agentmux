// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Jobs section's editor: New job (blank, from a template), the template approval, Edit, Clone,
// Dry run and the template update diff, through the section the way the operator uses them.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { JobsListResult } from "@/types/rpc/JobsListResult";
import type { JobView } from "@/types/rpc/JobView";

const rpc = vi.hoisted(() => ({
    list: vi.fn(),
    runs: vi.fn(),
    act: vi.fn(),
    get: vi.fn(),
    save: vi.fn(),
    dry: vi.fn(),
    templates: vi.fn(),
    template: vi.fn(),
    approve: vi.fn(),
    diff: vi.fn(),
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        JobsListCommand: () => rpc.list(),
        JobsRunsCommand: (_c: unknown, d: unknown) => rpc.runs(d),
        JobsActCommand: (_c: unknown, d: unknown) => rpc.act(d),
        JobsGetCommand: (_c: unknown, d: unknown) => rpc.get(d),
        JobsSaveCommand: (_c: unknown, d: unknown) => rpc.save(d),
        JobsDryrunCommand: (_c: unknown, d: unknown) => rpc.dry(d),
        JobsTemplatesListCommand: () => rpc.templates(),
        JobsTemplatesGetCommand: (_c: unknown, d: unknown) => rpc.template(d),
        JobsTemplatesApproveCommand: (_c: unknown, d: unknown) => rpc.approve(d),
        JobsTemplatesUpdateDiffCommand: (_c: unknown, d: unknown) => rpc.diff(d),
    },
}));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/store/global", () => ({ openLink: vi.fn() }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: vi.fn() }));
vi.mock("@/app/view/warden/open-warden", () => ({ openWarden: vi.fn() }));
vi.mock("@/app/view/agent/use-agent-list", () => ({
    useAgentList: () => [() => [{ id: "uid-b", name: "Builder", is_seeded: 0 }], () => null],
}));
vi.mock("@/app/view/vault/vault-store", () => ({
    vaultAvailable: () => true,
    vaultEntries: () => [{ id: "v-1", label: "Repo token" }],
    refreshVault: async () => {},
}));

import { DEFAULTS, mkDryRun, mkJob, mkTemplate, NOW } from "./jobs-fixtures";
import { WardenJobsManager } from "./warden-jobs-manager";

const listOf = (jobs: JobView[]): JobsListResult => ({ jobs, paused_all: null, now_ms: NOW, defaults: DEFAULTS, dropped: [] });
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

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

async function openEditor(jobs: JobView[] = []) {
    rpc.list.mockResolvedValue(listOf(jobs));
    render(() => <WardenJobsManager visible />);
    await waitFor(() => expect(screen.getByRole("button", { name: /New job/ })).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: /New job/ }));
    await waitFor(() => expect(screen.getByRole("region", { name: "New job" })).toBeInTheDocument());
}

const type = (label: string | RegExp, value: string) =>
    fireEvent.input(screen.getByLabelText(label), { target: { value } });

describe("New job", () => {
    it("saves a blank job with the form's trigger, action and policy", async () => {
        rpc.templates.mockResolvedValue(templatesOf());
        rpc.save.mockResolvedValue(saved);
        await openEditor();
        type("Name", "Morning check");
        fireEvent.change(screen.getByLabelText("Message"), { target: { value: "uid-b" } });
        type("Brief", "Look at the board.");
        type("Cron expression", "30 7 * * 1-5");
        type("Time zone", "Europe/Berlin");
        type("Cap per run ($)", "12");
        fireEvent.click(screen.getByRole("button", { name: /Create job/ }));
        await waitFor(() => expect(rpc.save).toHaveBeenCalledTimes(1));
        expect(rpc.save).toHaveBeenCalledWith({
            definition: {
                jobVersion: 1,
                name: "Morning check",
                owner: "",
                enabled: true,
                params: {},
                trigger: { kind: "cron", cron: "30 7 * * 1-5", tz: "Europe/Berlin" },
                action: { kind: "message", to: "uid-b", brief: "Look at the board." },
                policy: { overlap: "skip", catch_up: "skip", retries: 0, caps: { run_usd: 12 } },
            },
        });
        await waitFor(() => expect(screen.queryByRole("region", { name: "New job" })).toBeNull());
    });

    it("shows the defaults beside the caps and says when one is raised above its default", async () => {
        rpc.templates.mockResolvedValue(templatesOf());
        await openEditor();
        expect(screen.getByText("Default per run: $5.")).toBeInTheDocument();
        expect(screen.getByLabelText("Cap per day ($)").getAttribute("placeholder")).toBe("20");
        type("Cap per run ($)", "50");
        expect(screen.getByText("Above the default of $5: you raise it for this job.")).toBeInTheDocument();
    });

    it("says what is missing and sends nothing", async () => {
        rpc.templates.mockResolvedValue(templatesOf());
        await openEditor();
        fireEvent.click(screen.getByRole("button", { name: /Create job/ }));
        const problems = await screen.findByRole("alert");
        expect(problems.textContent).toContain("Give the job a name.");
        expect(problems.textContent).toContain("Choose the agent to message.");
        expect(rpc.save).not.toHaveBeenCalled();
    });

    it("shows what srv refuses", async () => {
        rpc.templates.mockResolvedValue(templatesOf());
        rpc.save.mockRejectedValue(new Error("trigger: an unattended agent job runs at most every 15 minutes"));
        await openEditor();
        type("Name", "Too often");
        fireEvent.change(screen.getByLabelText("Message"), { target: { value: "uid-b" } });
        type("Brief", "Go.");
        fireEvent.click(screen.getByRole("button", { name: /Create job/ }));
        await waitFor(() => expect(screen.getByText(/at most every 15 minutes/)).toBeInTheDocument());
        expect(screen.getByRole("region", { name: "New job" })).toBeInTheDocument();
    });

    it("asks the operator to confirm a job on a provider that reports no cost, then sends the confirmation", async () => {
        rpc.templates.mockResolvedValue(templatesOf());
        rpc.save.mockResolvedValueOnce({
            ...saved,
            ok: false,
            job_id: "",
            needs_confirmation: "no_cost",
            provider: "codex",
            error: "this job's agent runs on codex, which doesn't report what a run costs",
        });
        rpc.save.mockResolvedValueOnce(saved);
        await openEditor();
        type("Name", "On codex");
        fireEvent.change(screen.getByLabelText("Message"), { target: { value: "uid-b" } });
        type("Brief", "Go.");
        fireEvent.click(screen.getByRole("button", { name: /Create job/ }));
        await waitFor(() => expect(screen.getByText(/doesn't report what a run costs/)).toBeInTheDocument());
        expect(rpc.save.mock.calls[0][0].confirm_no_cost).toBeUndefined();
        fireEvent.click(screen.getByRole("button", { name: /Confirm and save/ }));
        await waitFor(() => expect(rpc.save).toHaveBeenCalledTimes(2));
        expect(rpc.save.mock.calls[1][0].confirm_no_cost).toBe(true);
    });
});

describe("New job from a template", () => {
    it("makes the form from the template's parameters and saves its pin and values", async () => {
        rpc.templates.mockResolvedValue(templatesOf(mkTemplate()));
        rpc.save.mockResolvedValue(saved);
        await openEditor();
        fireEvent.click(screen.getByRole("radio", { name: "A template" }));
        const row = (await screen.findByText("Weekly report")).closest("li")!;
        expect(within(row).getByText("Approved")).toBeInTheDocument();
        fireEvent.click(within(row).getByRole("button", { name: /Clone to a job/ }));

        // The parameter form: a text field, a choice with its default, a list, a number, a Vault entry.
        await waitFor(() => expect(screen.getByLabelText("Repository")).toBeInTheDocument());
        expect((screen.getByLabelText("Focus (optional)") as HTMLSelectElement).value).toBe("security");
        expect((screen.getByLabelText("Most items (optional)") as HTMLInputElement).value).toBe("10");
        expect(within(screen.getByLabelText("Token (optional)")).getByText("Repo token")).toBeInTheDocument();
        expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Weekly report");
        expect((screen.getByLabelText("Cron expression") as HTMLInputElement).value).toBe("0 6 * * 1");
        expect((screen.getByLabelText("Timeout (minutes)") as HTMLInputElement).value).toBe("60");
        expect(screen.getByText("Written by the template from its settings. A dry run shows it.")).toBeInTheDocument();

        // A required setting left empty stops it.
        fireEvent.change(screen.getByLabelText("Message"), { target: { value: "uid-b" } });
        fireEvent.click(screen.getByRole("button", { name: /Create job/ }));
        await waitFor(() => expect(screen.getByText("Required.")).toBeInTheDocument());
        expect(screen.getByRole("alert").textContent).toContain("Some of the template's settings need a value.");
        expect(rpc.save).not.toHaveBeenCalled();

        type("Repository", "acme/site");
        fireEvent.input(screen.getByLabelText("Paths (optional)"), { target: { value: "docs\nsrc" } });
        fireEvent.change(screen.getByLabelText("Token (optional)"), { target: { value: "v-1" } });
        fireEvent.click(screen.getByRole("button", { name: /Create job/ }));
        await waitFor(() => expect(rpc.save).toHaveBeenCalledTimes(1));
        const sent = rpc.save.mock.calls[0][0].definition;
        expect(sent.template).toEqual({ id: "agentmux.weekly-report", version: "1.0.0", hash: "hash-1" });
        expect(sent.params).toEqual({
            repo: "acme/site",
            focus: "security",
            paths: ["docs", "src"],
            max_items: 10,
            token: "v-1",
        });
        expect(sent.action).toEqual({ kind: "message", to: "uid-b" });
        expect(sent.policy).toMatchObject({ catch_up: "latest", timeout_minutes: 60, caps: { run_usd: 3 } });
    });

    it("shows an unapproved template's publisher, trigger, outputs, secrets and caps, and approves exactly that", async () => {
        const unapproved = mkTemplate({ state: "needs_approval" });
        rpc.templates.mockResolvedValueOnce(templatesOf(unapproved));
        rpc.approve.mockResolvedValue(templatesOf(mkTemplate()));
        await openEditor();
        fireEvent.click(screen.getByRole("radio", { name: "A template" }));
        const row = (await screen.findByText("Weekly report")).closest("li")!;
        expect(within(row).getByText("Needs your approval")).toBeInTheDocument();
        expect(within(row).queryByRole("button", { name: /Clone to a job/ })).toBeNull();
        fireEvent.click(within(row).getByRole("button", { name: /Review and approve/ }));

        const panel = await screen.findByRole("region", { name: "Approve agentmux.weekly-report" });
        expect(panel.textContent).toContain("Approve Weekly report 1.0.0?");
        expect(panel.textContent).toContain("Published by AgentMux");
        expect(panel.textContent).toContain("Mondays 06:00 UTC");
        expect(panel.querySelector('[title="Writes outside AgentMux"]')?.textContent).toBe("pull_request");
        expect(panel.textContent).toContain("report_file");
        expect(panel.textContent).toContain("REPO_TOKEN");
        expect(panel.textContent).toContain("$3 per run");
        expect(rpc.approve).not.toHaveBeenCalled();

        fireEvent.click(within(panel).getByRole("button", { name: /^Approve$/ }));
        await waitFor(() =>
            expect(rpc.approve).toHaveBeenCalledWith({ id: "agentmux.weekly-report", hash: "hash-1", signer: "" })
        );
        await waitFor(() => expect(screen.getByRole("button", { name: /Clone to a job/ })).toBeInTheDocument());
        expect(screen.queryByRole("region", { name: /^Approve/ })).toBeNull();
    });

    it("warns when a publisher's key is not the one pinned", async () => {
        const changed = mkTemplate({
            source: "installed",
            state: "needs_approval",
            signature: { state: "key_changed", publisher: "acme", fingerprint: "NEWK-EY00", pinned: "OLDK-EY00" },
        });
        rpc.templates.mockResolvedValue(templatesOf(changed));
        await openEditor();
        fireEvent.click(screen.getByRole("radio", { name: "A template" }));
        fireEvent.click(await screen.findByRole("button", { name: /Review and approve/ }));
        const panel = await screen.findByRole("region", { name: /^Approve/ });
        expect(panel.textContent).toContain(
            "differs from the one pinned for acme: pinned OLDK-EY00, this one NEWK-EY00"
        );
    });

    it("says why an approval was refused", async () => {
        rpc.templates.mockResolvedValue(templatesOf(mkTemplate({ state: "changed" })));
        rpc.approve.mockRejectedValue(new Error("the template changed since you were asked; review it again"));
        await openEditor();
        fireEvent.click(screen.getByRole("radio", { name: "A template" }));
        fireEvent.click(await screen.findByRole("button", { name: /Review and approve/ }));
        fireEvent.click(await screen.findByRole("button", { name: /^Approve$/ }));
        await waitFor(() => expect(screen.getByText(/changed since you were asked/)).toBeInTheDocument());
    });
});

describe("Edit and Clone", () => {
    async function openJob() {
        rpc.list.mockResolvedValue(listOf([mkJob({ owner_uid: "uid-b" })]));
        rpc.runs.mockResolvedValue({ runs: [] });
        rpc.templates.mockResolvedValue(templatesOf());
        rpc.get.mockResolvedValue({ definition: STORED });
        render(() => <WardenJobsManager visible />);
        await waitFor(() => expect(screen.getByText("Nightly build")).toBeInTheDocument());
        fireEvent.click(screen.getByText("Nightly build"));
        await waitFor(() => expect(screen.getByRole("button", { name: /^Edit$/ })).toBeInTheDocument());
    }

    it("edits a job in place: its fields load, and the save names the job", async () => {
        rpc.save.mockResolvedValue({ ...saved, job_id: "j1" });
        await openJob();
        fireEvent.click(screen.getByRole("button", { name: /^Edit$/ }));
        await waitFor(() => expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Nightly build"));
        expect(rpc.get).toHaveBeenCalledWith({ job_id: "j1" });
        expect((screen.getByLabelText("Brief") as HTMLTextAreaElement).value).toBe("Build it.");
        expect((screen.getByLabelText("Message") as HTMLSelectElement).value).toBe("uid-b");
        type("Name", "Nightly build, early");
        fireEvent.click(screen.getByRole("button", { name: /Save changes/ }));
        await waitFor(() => expect(rpc.save).toHaveBeenCalledTimes(1));
        const sent = rpc.save.mock.calls[0][0];
        expect(sent.job_id).toBe("j1");
        expect(sent.definition).toMatchObject({
            name: "Nightly build, early",
            trigger: { kind: "cron", cron: "0 6 * * *", tz: "UTC" },
            action: STORED.action,
        });
    });

    it("clones a job as a new one named as a copy", async () => {
        rpc.save.mockResolvedValue(saved);
        await openJob();
        fireEvent.click(screen.getByRole("button", { name: /^Clone$/ }));
        await waitFor(() =>
            expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Nightly build (copy)")
        );
        expect(screen.getByRole("region", { name: "Clone job" })).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: /Create job/ }));
        await waitFor(() => expect(rpc.save).toHaveBeenCalledTimes(1));
        const sent = rpc.save.mock.calls[0][0];
        expect(sent.job_id).toBeUndefined();
        expect(sent.definition.name).toBe("Nightly build (copy)");
        expect(sent.definition.action).toEqual(STORED.action);
    });
});

describe("Dry run", () => {
    it("shows a stored job's brief, fire times in its zone, caps, outputs and secrets, and says nothing started", async () => {
        rpc.list.mockResolvedValue(listOf([mkJob()]));
        rpc.runs.mockResolvedValue({ runs: [] });
        rpc.dry.mockResolvedValue(
            mkDryRun({
                secrets: [{ name: "REPO_TOKEN", vault: "v-gone" }],
                group: "deploy",
                caps: { run_usd: 3, day_usd: 20, account_day_usd: 50, messages_per_day: 24, clamped: true },
                notes: ["codex doesn't report cost"],
            })
        );
        render(() => <WardenJobsManager visible />);
        await waitFor(() => expect(screen.getByText("Nightly build")).toBeInTheDocument());
        fireEvent.click(screen.getByText("Nightly build"));
        fireEvent.click(await screen.findByRole("button", { name: /Dry run/ }));
        const panel = await screen.findByRole("region", { name: "Dry run" });
        expect(rpc.dry).toHaveBeenCalledWith({ job_id: "j1" });
        expect(panel.textContent).toContain("Nothing was started");
        expect(panel.textContent).toContain("Report on acme/site.");
        expect(panel.textContent).toContain("Mon 2026-10-26 06:00 CET");
        expect(panel.textContent).toContain("$3 per run, $20 per day, $50 per account per day");
        expect(panel.textContent).toContain("held to the defaults");
        expect(panel.textContent).toContain("report_file");
        expect(panel.textContent).toContain("REPO_TOKEN");
        expect(panel.textContent).toContain("Vault entry v-gone");
        expect(panel.textContent).toContain("not in the Vault");
        expect(panel.textContent).toContain("the deploy group");
        expect(panel.textContent).toContain("codex doesn't report cost");
    });

    it("runs the form's definition from the editor, and shows why one would not be saved", async () => {
        rpc.templates.mockResolvedValue(templatesOf());
        rpc.dry.mockResolvedValueOnce(mkDryRun({ brief: "Look at the board." })).mockResolvedValueOnce({
            ...mkDryRun(),
            ok: false,
            error: "trigger: invalid cron 'nope'",
        });
        await openEditor();
        type("Name", "Morning check");
        fireEvent.change(screen.getByLabelText("Message"), { target: { value: "uid-b" } });
        type("Brief", "Look at the board.");
        fireEvent.click(screen.getByRole("button", { name: /^Dry run$/ }));
        await waitFor(() => expect(screen.getByText("Look at the board.", { selector: "pre" })).toBeInTheDocument());
        expect(rpc.dry.mock.calls[0][0].definition).toMatchObject({
            name: "Morning check",
            action: { kind: "message", to: "uid-b", brief: "Look at the board." },
        });
        expect(rpc.dry.mock.calls[0][0].job_id).toBeUndefined();
        expect(rpc.save).not.toHaveBeenCalled();

        type("Cron expression", "nope");
        fireEvent.click(screen.getByRole("button", { name: /^Dry run$/ }));
        await waitFor(() => expect(screen.getByText(/this wouldn't be saved/)).toBeInTheDocument());
        expect(screen.getByText(/invalid cron 'nope'/)).toBeInTheDocument();
    });
});

describe("a template's update", () => {
    const behind = (approved: boolean) =>
        mkJob({ template: { id: "agentmux.weekly-report", version: "1.0.0", update: { version: "1.1.0", approved } } });
    const diff = {
        job_id: "j1",
        template_id: "agentmux.weekly-report",
        status: "available",
        reason: null,
        from_version: "1.0.0",
        to_version: "1.1.0",
        to_hash: "hash-2",
        brief: { before: "Report on {repo}.", after: "Report on {repo} and its links." },
        outputs: { added: ["kanban_task"], removed: ["comment"] },
        secrets: null,
        policy: [{ field: "timeout_minutes", before: "60", after: "90" }],
        params_error: null,
    };

    async function openUpdate(approved: boolean) {
        rpc.list.mockResolvedValue(listOf([behind(approved)]));
        rpc.runs.mockResolvedValue({ runs: [] });
        render(() => <WardenJobsManager visible />);
        await waitFor(() => expect(screen.getByText("Nightly build")).toBeInTheDocument());
    }

    it("marks the job, shows what accepting changes, and accepts exactly that version", async () => {
        rpc.diff.mockResolvedValue(diff);
        rpc.save.mockResolvedValue({ ...saved, job_id: "j1" });
        await openUpdate(true);
        expect(screen.getByText("Update available")).toBeInTheDocument();
        fireEvent.click(screen.getByText("Nightly build"));
        fireEvent.click(await screen.findByRole("button", { name: "Update to 1.1.0" }));
        const panel = await screen.findByRole("region", { name: "Update for Nightly build" });
        await waitFor(() => expect(panel.textContent).toContain("1.0.0 → 1.1.0"));
        expect(rpc.diff).toHaveBeenCalledWith({ job_id: "j1" });
        expect(panel.textContent).toContain("Report on {repo} and its links.");
        expect(panel.textContent).toContain("+ kanban_task");
        expect(panel.textContent).toContain("− comment");
        expect(panel.textContent).toContain("timeout_minutes");
        expect(panel.textContent).toContain("60 → 90");

        fireEvent.click(within(panel).getByRole("button", { name: /Accept the update/ }));
        await waitFor(() =>
            expect(rpc.save).toHaveBeenCalledWith({
                job_id: "j1",
                definition: { template: { id: "agentmux.weekly-report", version: "1.1.0", hash: "hash-2" } },
            })
        );
        await waitFor(() => expect(screen.queryByRole("region", { name: "Update for Nightly build" })).toBeNull());
    });

    it("sends a version the operator hasn't approved to be reviewed first, with nothing to accept", async () => {
        rpc.diff.mockResolvedValue({
            ...diff,
            status: "needs_approval",
            reason: "agentmux.weekly-report 1.1.0 hasn't been approved yet",
            brief: null,
            outputs: null,
            policy: [],
        });
        rpc.template.mockResolvedValue({
            template: mkTemplate({ state: "changed" }),
            brief: null,
            preview: null,
            preview_error: null,
        });
        rpc.approve.mockResolvedValue(templatesOf(mkTemplate()));
        await openUpdate(false);
        expect(screen.getByText("Update to approve")).toBeInTheDocument();
        fireEvent.click(screen.getByText("Nightly build"));
        fireEvent.click(await screen.findByRole("button", { name: "Review 1.1.0" }));
        const panel = await screen.findByRole("region", { name: "Update for Nightly build" });
        await waitFor(() => expect(panel.textContent).toContain("hasn't been approved yet"));
        expect(within(panel).queryByRole("button", { name: /Accept the update/ })).toBeNull();

        fireEvent.click(within(panel).getByRole("button", { name: /Review and approve 1.1.0/ }));
        const approval = await screen.findByRole("region", { name: "Approve agentmux.weekly-report" });
        fireEvent.click(within(approval).getByRole("button", { name: /^Approve$/ }));
        await waitFor(() =>
            expect(rpc.approve).toHaveBeenCalledWith({ id: "agentmux.weekly-report", hash: "hash-1", signer: "" })
        );
        await waitFor(() => expect(rpc.diff).toHaveBeenCalledTimes(2));
    });

    it("tells the operator when the new version can't take the job's settings", async () => {
        rpc.diff.mockResolvedValue({ ...diff, brief: null, params_error: 'parameter "branch" is required' });
        await openUpdate(true);
        fireEvent.click(screen.getByText("Nightly build"));
        fireEvent.click(await screen.findByRole("button", { name: "Update to 1.1.0" }));
        const panel = await screen.findByRole("region", { name: "Update for Nightly build" });
        await waitFor(() => expect(panel.textContent).toContain('parameter "branch" is required'));
        expect(within(panel).queryByRole("button", { name: /Accept the update/ })).toBeNull();
    });

    it("shows no update mark on a job at its template's version", async () => {
        rpc.list.mockResolvedValue(
            listOf([mkJob({ template: { id: "agentmux.weekly-report", version: "1.0.0", update: null } })])
        );
        render(() => <WardenJobsManager visible />);
        await waitFor(() => expect(screen.getByText("Nightly build")).toBeInTheDocument());
        expect(screen.queryByText("Update available")).toBeNull();
    });
});
