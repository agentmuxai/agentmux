// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for AgentCreatePanel, the panel below a harness card that takes a new
 * agent through Tools, Install and Create, showing only the steps that apply
 * (SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md §2.3).
 *
 * Covered:
 *  - Tools: each missing tool's label and install link; Check again re-probes
 *    fresh; Continue anyway moves on; a system tool installed inline keeps
 *    saying so even though srv's stale PATH still reports it missing
 *  - Install, npm-managed CLI: the install starts against the provider the
 *    launch resolves (#2594), the step list follows npm's output, a failure
 *    is explained, Cancel install stops it, and closing the panel does not
 *  - Install, a CLI AgentMux doesn't install: the vendor's own command for
 *    this platform, and Check again
 *  - an installed CLI with nothing missing goes straight to the Create form,
 *    with no step trail
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

// Any mock, so a test can pass its own vi.fn() for a typed callback.
type AnyMock = Mock<(...args: any[]) => any>;

const { openExternal, pushNotification, getPlatform } = vi.hoisted(() => ({
    openExternal: vi.fn(),
    pushNotification: vi.fn(),
    getPlatform: vi.fn(() => "linux"),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        InstallStartCommand: vi.fn(),
        InstallCancelCommand: vi.fn(),
        InstallCheckCommand: vi.fn(),
        ToolchainResolveInstallCommandCommand: vi.fn(),
        ToolchainInstallSystemToolCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", async () => {
    const { isNoPayloadMuxEvent } = await vi.importActual<typeof import("@/app/store/mps-events.generated")>("@/app/store/mps-events.generated");
    const mock = { muxEventSubscribe: vi.fn(() => () => {}) };
    // As the real onMuxEvent: a typed subscription is a muxEventSubscribe whose handler gets the data.
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        (mock.muxEventSubscribe as (sub: unknown) => () => void)({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); else if (isNoPayloadMuxEvent(eventType)) handler(null, e); } } as any);
    return { ...mock, onMuxEvent };
});
vi.mock("@/app/store/global", () => ({
    getApi: () => ({ openExternal }),
    // The install console reads agent:followcontroller (unset here).
    getSettingsKeyAtom: () => () => undefined,
    isDev: false,
}));
vi.mock("@/app/store/flash-notifications", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/flash-notifications")>()),
    pushNotification,
}));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: vi.fn() } }));
vi.mock("@/util/clipboard", () => ({ writeText: vi.fn().mockResolvedValue(undefined) }));
vi.mock("@/util/platformutil", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/util/platformutil")>()),
    getPlatform,
}));

vi.mock("../defaults/cli-catalog", async (importOriginal) => ({
    ...(await importOriginal<typeof import("../defaults/cli-catalog")>()),
    getCliCatalogEntry: (id: string) =>
        ({
            codex: { displayName: "Codex", icon: "🤖", popoverMarkdown: "" },
            antigravity: { displayName: "Antigravity", icon: "✦", popoverMarkdown: "" },
        })[id],
}));

// Fixed fixtures, so the assertions don't follow the real catalog's pins.
vi.mock("../providers", async (importOriginal) => ({
    ...(await importOriginal<typeof import("../providers")>()),
    getProvider: (id: string) =>
        ({
            codex: {
                id: "codex",
                displayName: "Codex",
                cliCommand: "codex",
                npmPackage: "@openai/codex",
                pinnedVersion: "0.116.0",
                docsUrl: "https://example.test/codex",
                windowsInstallCommand: "npm install -g @openai/codex",
                unixInstallCommand: "npm install -g @openai/codex",
            },
            antigravity: {
                id: "antigravity",
                displayName: "Antigravity",
                cliCommand: "agy",
                npmPackage: "",
                pinnedVersion: "",
                docsUrl: "https://example.test/agy-docs",
                windowsInstallCommand: "irm https://example.test/install.ps1 | iex",
                unixInstallCommand: "curl -fsSL https://example.test/install.sh | bash",
            },
        })[id],
}));

vi.mock("../agent-launch-env", () => ({
    resolveEffectiveLaunchProvider: vi.fn(async (t: { provider: string }) => t.provider),
}));

// The Create step's own behaviour is AgentCreateForm.test.tsx's.
vi.mock("./AgentCreateForm", () => ({
    AgentCreateForm: () => <div data-testid="agent-create-form" />,
    createCreateDraft: () => ({}),
}));

import { AgentCreatePanel, type MissingPrereq } from "./AgentCreatePanel";
import { createCreateDraft } from "./AgentCreateForm";
import { RpcApi } from "@/app/store/rpc-api";
import { muxEventSubscribe } from "@/app/store/mps";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
import { __resetAgentInstallSessions, agentInstallSession } from "../install-sessions";
import { resetInstallDetailsPrefs } from "@/element/install/InstallProgress";
import { LOG_ROW_PX } from "@/element/install/LogView";
import type { AgentDefinition } from "@/app/store/rpc-api";

const baseTemplate = (over: Partial<AgentDefinition> = {}): AgentDefinition =>
    ({
        id: "tpl-1",
        slug: "tpl",
        name: "Codex",
        icon: "",
        provider: "codex",
        description: "",
        working_directory: "",
        shell: "",
        provider_flags: "",
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 0,
        agent_type: "host",
        environment: "local",
        agent_bus_id: "",
        is_seeded: 1,
        memory_id: "",
        ...over,
    }) as AgentDefinition;

beforeEach(() => {
    vi.mocked(RpcApi.InstallStartCommand).mockResolvedValue({ sessionId: "sess-1" } as any);
    vi.mocked(RpcApi.InstallCancelCommand).mockResolvedValue({} as any);
    vi.mocked(resolveEffectiveLaunchProvider).mockImplementation(async (t) => t.provider);
    getPlatform.mockReturnValue("linux");
});

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    resetInstallDetailsPrefs();
    __resetAgentInstallSessions();
});

interface RenderOpts {
    template?: AgentDefinition;
    installed?: boolean | undefined;
    missing?: MissingPrereq[];
    probePrereqs?: AnyMock;
    /** Defaults to marking the CLI installed. */
    recheckInstalled?: AnyMock;
}

function renderPanel(opts: RenderOpts = {}) {
    const template = opts.template ?? baseTemplate();
    const [installed, setInstalled] = createSignal<boolean | undefined>("installed" in opts ? opts.installed : true);
    const probePrereqs: AnyMock = opts.probePrereqs ?? vi.fn().mockResolvedValue(opts.missing ?? []);
    const recheckInstalled: AnyMock =
        opts.recheckInstalled ??
        vi.fn(async () => {
            setInstalled(true);
        });
    const onCreated = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();
    const onBusyChange = vi.fn();
    const draft = createCreateDraft(template);
    const result = render(() => (
        <AgentCreatePanel
            template={template}
            draft={draft}
            installed={installed()}
            probePrereqs={probePrereqs}
            recheckInstalled={recheckInstalled}
            onCreated={onCreated}
            onClose={onClose}
            onBusyChange={onBusyChange}
        />
    ));
    return { ...result, probePrereqs, recheckInstalled, onClose, setInstalled };
}

/** The `install_chunk` sender for the install subscribed under `scope`. */
async function chunkSender(scope: string): Promise<(data: Record<string, unknown>) => void> {
    let handler!: (e: unknown) => void;
    await waitFor(() => {
        const sub = vi
            .mocked(muxEventSubscribe)
            .mock.calls.map(([s]) => s as unknown as { scope?: string; handler: (e: unknown) => void })
            .find((s) => s.scope === scope);
        expect(sub).toBeTruthy();
        handler = sub!.handler;
    });
    return (data) => handler({ data });
}

/** Clicks "Install now" and returns a function that delivers install_chunk events. */
async function startAndSubscribe(): Promise<(data: Record<string, unknown>) => void> {
    fireEvent.click(await screen.findByTestId("agent-create-install-start"));
    const send = await chunkSender("install:sess-1");
    return (data) => send({ sessionId: "sess-1", ...data });
}

const stepStatus = (container: HTMLElement) =>
    Array.from(container.querySelectorAll<HTMLElement>(".install-step")).map((li) => [
        li.querySelector(".install-step-label")?.textContent,
        li.dataset.status,
    ]);

const logRows = (container: HTMLElement) =>
    Array.from(container.querySelectorAll<HTMLElement>(".install-log-line")).map((el) => [el.textContent, el.dataset.tone]);

const trail = () => Array.from(screen.getByTestId("agent-create-steps").querySelectorAll("li")).map((li) => li.textContent);
const currentStep = () => screen.getByTestId("agent-create-steps").querySelector('[aria-current="step"]')?.textContent;

const git: MissingPrereq = { tool: "git", label: "Git", installUrl: "https://git-scm.com", installLinkText: "Install Git" };
const jq: MissingPrereq = { tool: "jq", label: "jq", installUrl: "https://jqlang.org", installLinkText: "Get jq" };

const gitResolved = {
    available: true,
    program: "winget",
    args: ["install", "--id", "Git.Git"],
    needsElevation: true,
    commandPreview: "winget install --id Git.Git",
    resolvedVersion: "2.47.1",
};

describe("AgentCreatePanel — straight to Create", () => {
    it("shows the Create form directly, with no step trail, when the CLI is installed and nothing is missing", async () => {
        const { probePrereqs } = renderPanel({ installed: true });
        await screen.findByTestId("agent-create-form");
        expect(probePrereqs).toHaveBeenCalledWith(false);
        expect(screen.queryByTestId("agent-create-tools")).toBeNull();
        expect(screen.queryByTestId("agent-create-npm-install")).toBeNull();
        expect(screen.queryByTestId("agent-create-steps")).toBeNull();
        expect(screen.getByText("New Codex agent")).toBeInTheDocument();
    });

    it("says it is checking until the tools probe answers", async () => {
        let answer!: (v: MissingPrereq[]) => void;
        renderPanel({ probePrereqs: vi.fn(() => new Promise<MissingPrereq[]>((r) => (answer = r))) });
        expect(screen.getByText(/Checking what Codex needs/)).toBeInTheDocument();
        expect(screen.queryByTestId("agent-create-form")).toBeNull();
        answer([]);
        await screen.findByTestId("agent-create-form");
    });
});

describe("AgentCreatePanel — Tools step", () => {
    it("lists each missing tool with its install link", async () => {
        renderPanel({ missing: [jq] });
        await screen.findByTestId("agent-create-tools");
        expect(screen.getByText("jq")).toBeInTheDocument();
        expect(screen.getByText(/— not found/)).toBeInTheDocument();
        const link = screen.getByText("Get jq").closest("a")!;
        expect(link.getAttribute("href")).toBe("https://jqlang.org");
        fireEvent.click(link);
        expect(openExternal).toHaveBeenCalledWith("https://jqlang.org");
        // Tools, then Create (the CLI is installed).
        expect(trail()).toEqual(["Tools", "Create"]);
        expect(currentStep()).toBe("Tools");
        expect(screen.queryByTestId("agent-create-form")).toBeNull();
    });

    it("Check again probes fresh, and moves on once nothing is missing", async () => {
        const probePrereqs = vi.fn().mockResolvedValueOnce([jq]).mockResolvedValueOnce([]);
        renderPanel({ probePrereqs });
        await screen.findByTestId("agent-create-tools");
        fireEvent.click(screen.getByRole("button", { name: "Check again" }));
        expect(probePrereqs).toHaveBeenLastCalledWith(true);
        await screen.findByTestId("agent-create-form");
        expect(screen.queryByTestId("agent-create-steps")).toBeNull();
    });

    it("Continue anyway moves on to the next step", async () => {
        renderPanel({ missing: [jq], installed: false });
        await screen.findByTestId("agent-create-tools");
        fireEvent.click(screen.getByTestId("agent-create-tools-continue"));
        await screen.findByTestId("agent-create-npm-install");
        expect(trail()).toEqual(["Tools", "Install", "Create"]);
        expect(currentStep()).toBe("Install");
    });

    it("Cancel closes the panel", async () => {
        const { onClose } = renderPanel({ missing: [jq] });
        await screen.findByTestId("agent-create-tools");
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("offers no inline install for a tool the system installer doesn't cover", async () => {
        renderPanel({ missing: [jq] });
        await screen.findByTestId("agent-create-tools");
        expect(RpcApi.ToolchainResolveInstallCommandCommand).not.toHaveBeenCalled();
        expect(screen.queryByText(/install it now/)).toBeNull();
    });

    it("labels the inline install with the resolved version once resolution completes", async () => {
        vi.mocked(RpcApi.ToolchainResolveInstallCommandCommand).mockResolvedValue(gitResolved as any);
        renderPanel({ missing: [git] });
        await screen.findByText("Install v2.47.1 now ↓");
        expect(RpcApi.ToolchainResolveInstallCommandCommand).toHaveBeenCalledWith(expect.anything(), { toolId: "git" });
    });

    it("hides the inline install when the system installer can't install the tool", async () => {
        vi.mocked(RpcApi.ToolchainResolveInstallCommandCommand).mockResolvedValue({ available: false } as any);
        renderPanel({ missing: [git] });
        await screen.findByTestId("agent-create-tools");
        await waitFor(() => expect(RpcApi.ToolchainResolveInstallCommandCommand).toHaveBeenCalled());
        await waitFor(() => expect(screen.queryByText(/install it now|Install v/)).toBeNull());
        // The link stays.
        expect(screen.getByText("Install Git")).toBeInTheDocument();
    });

    // srv's own PATH is captured at process startup, so a freshly installed
    // binary routinely still probes as missing right after the install. The
    // row must keep saying "installed" instead of reverting to "not found".
    it("keeps saying a tool installed inline is installed, though the stale-PATH re-probe still lists it", async () => {
        vi.mocked(RpcApi.ToolchainResolveInstallCommandCommand).mockResolvedValue(gitResolved as any);
        vi.mocked(RpcApi.ToolchainInstallSystemToolCommand).mockResolvedValue({ sessionId: "sysinstall-1" } as any);
        const probePrereqs = vi.fn().mockResolvedValue([git, jq]);
        renderPanel({ probePrereqs });

        fireEvent.click(await screen.findByText("Install v2.47.1 now ↓"));
        await screen.findByText("winget install --id Git.Git");
        fireEvent.click(screen.getByText("Install v2.47.1 now"));
        const send = await chunkSender("install:sysinstall-1");
        send({ op: "done", ok: true });

        // The install re-probes fresh; git is still reported missing.
        await waitFor(() => expect(probePrereqs).toHaveBeenLastCalledWith(true));
        await screen.findByText(/installed, restart AgentMux to use it/);
        // jq is still really missing, so the step stays.
        expect(screen.getAllByText(/— not found/)).toHaveLength(1);
        expect(screen.getByTestId("agent-create-tools")).toBeInTheDocument();
    });

    it("moves on once every missing tool is installed inline, keeping Tools in the trail", async () => {
        vi.mocked(RpcApi.ToolchainResolveInstallCommandCommand).mockResolvedValue(gitResolved as any);
        vi.mocked(RpcApi.ToolchainInstallSystemToolCommand).mockResolvedValue({ sessionId: "sysinstall-1" } as any);
        renderPanel({ probePrereqs: vi.fn().mockResolvedValue([git]) });

        fireEvent.click(await screen.findByText("Install v2.47.1 now ↓"));
        fireEvent.click(await screen.findByText("Install v2.47.1 now"));
        const send = await chunkSender("install:sysinstall-1");
        send({ op: "done", ok: true });

        await screen.findByTestId("agent-create-form");
        expect(trail()).toEqual(["Tools", "Create"]);
        expect(currentStep()).toBe("Create");
    });
});

describe("AgentCreatePanel — Install step, a CLI AgentMux installs (npm)", () => {
    it("shows the install step with every step pending before it starts", async () => {
        const { container } = renderPanel({ installed: false });
        await screen.findByTestId("agent-create-npm-install");
        expect(screen.getByText(/Codex isn't installed yet/)).toBeInTheDocument();
        expect(trail()).toEqual(["Install", "Create"]);
        expect(currentStep()).toBe("Install");
        expect(stepStatus(container)).toEqual([
            ["Check requirements", "pending"],
            ["Download packages", "pending"],
            ["Set up files", "pending"],
            ["Run setup scripts", "pending"],
            ["Check Codex is installed", "pending"],
        ]);
    });

    it("Install now starts the install with the provider's package and pin", async () => {
        renderPanel({ installed: false });
        fireEvent.click(await screen.findByTestId("agent-create-install-start"));
        await waitFor(() => expect(RpcApi.InstallStartCommand).toHaveBeenCalledTimes(1));
        expect(vi.mocked(RpcApi.InstallStartCommand).mock.calls[0][1]).toEqual({
            providerId: "codex",
            cliCommand: "codex",
            npmPackage: "@openai/codex",
            pinnedVersion: "0.116.0",
        });
    });

    // The provider decides which CLI package gets installed, so it must be
    // the one the launch resolves (an agent with no provider of its own
    // takes its bundle's), not the possibly-drifted column (#2594).
    it("installs the provider the launch resolves", async () => {
        vi.mocked(resolveEffectiveLaunchProvider).mockResolvedValue("codex");
        renderPanel({ template: baseTemplate({ provider: "", memory_id: "mem-1" }), installed: false });
        fireEvent.click(await screen.findByTestId("agent-create-install-start"));
        await waitFor(() => expect(RpcApi.InstallStartCommand).toHaveBeenCalled());
        expect(vi.mocked(RpcApi.InstallStartCommand).mock.calls[0][1]).toMatchObject({ providerId: "codex", npmPackage: "@openai/codex" });
        expect(screen.getByText(/Codex isn't installed yet/)).toBeInTheDocument();
    });

    it("keeps Details collapsed by default while the log still fills", async () => {
        const { container } = renderPanel({ installed: false });
        const send = await startAndSubscribe();
        send({ line: "npm http fetch GET 200 https://registry.npmjs.org/chalk/-/chalk-4.1.2.tgz 12ms", stream: "stderr" });
        const details = container.querySelector(".install-details") as HTMLDetailsElement;
        expect(details.open).toBe(false);
        await waitFor(() => expect(logRows(container).some(([t]) => t?.includes("chalk-4.1.2.tgz"))).toBe(true));
    });

    it("advances the steps from npm output, doesn't paint healthy stderr red, and re-checks the CLI when done", async () => {
        const { container, recheckInstalled } = renderPanel({ installed: false });
        const send = await startAndSubscribe();
        await waitFor(() => expect(stepStatus(container)[0]).toEqual(["Check requirements", "active"]));

        send({ line: "npm http fetch GET 200 https://registry.npmjs.org/chalk/-/chalk-4.1.2.tgz 12ms", stream: "stderr" });
        await waitFor(() => expect(stepStatus(container)[1]).toEqual(["Download packages", "active"]));
        expect(container.querySelector(".install-step-hint")?.textContent).toBe("1 fetched");
        expect(logRows(container).find(([t]) => t?.includes("chalk-4.1.2.tgz"))?.[1]).toBe("normal");

        send({ line: "added 1 package in 1s", stream: "stdout" });
        send({ op: "done", ok: true });
        // The panel watches this install: its end re-checks the CLI, which
        // moves on to Create; no toast.
        await waitFor(() => expect(recheckInstalled).toHaveBeenCalledTimes(1));
        await screen.findByTestId("agent-create-form");
        expect(trail()).toEqual(["Install", "Create"]);
        expect(currentStep()).toBe("Create");
        expect(pushNotification).not.toHaveBeenCalled();
    });

    it("offers Continue after the install when the re-check hasn't caught up", async () => {
        const recheckInstalled = vi.fn().mockResolvedValue(undefined);
        const { container } = renderPanel({ installed: false, recheckInstalled });
        const send = await startAndSubscribe();
        send({ line: "added 1 package in 1s", stream: "stdout" });
        send({ op: "done", ok: true });
        await waitFor(() => expect(stepStatus(container).map(([, st]) => st)).toEqual(["done", "done", "done", "skipped", "done"]));
        fireEvent.click(screen.getByRole("button", { name: "Continue" }));
        expect(recheckInstalled).toHaveBeenCalledTimes(2);
    });

    it("on failure explains it in the step list, offers Retry, and opens Details at the first error", async () => {
        const { container, recheckInstalled } = renderPanel({ installed: false });
        const send = await startAndSubscribe();
        send({ line: "npm http fetch GET https://registry.npmjs.org/npm attempt 1 failed with ENOTFOUND", stream: "stderr" });
        for (let i = 0; i < 30; i++) send({ line: `npm verbose stack frame ${i}`, stream: "stderr" });
        send({ line: "npm error code ENOTFOUND", stream: "stderr" });
        send({ line: "npm error network This is a problem related to network connectivity.", stream: "stderr" });
        send({ op: "done", ok: false, error: "npm exited Some(1)" });

        await waitFor(() => expect(screen.getByText("Couldn't reach the package server.")).toBeInTheDocument());
        expect(stepStatus(container)[1]).toEqual(["Download packages", "failed"]);
        expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
        expect(recheckInstalled).not.toHaveBeenCalled();

        const details = container.querySelector(".install-details") as HTMLDetailsElement;
        await waitFor(() => expect(details.open).toBe(true));
        // The first error is line 31; scrolled so it sits two rows from the top.
        const log = container.querySelector(".install-log") as HTMLElement;
        await waitFor(() => expect(log.scrollTop).toBe((31 - 2) * LOG_ROW_PX));
        expect(logRows(container).find(([t]) => t === "npm error code ENOTFOUND")?.[1]).toBe("error");
        // The backend's own reason ends the log.
        expect(logRows(container).at(-1)).toEqual(["Install failed: npm exited Some(1)", "error"]);

        // Retry starts a new run.
        fireEvent.click(screen.getByRole("button", { name: "Retry" }));
        await waitFor(() => expect(RpcApi.InstallStartCommand).toHaveBeenCalledTimes(2));
    });

    it("shows why the install couldn't start when the start RPC itself fails", async () => {
        vi.mocked(RpcApi.InstallStartCommand).mockRejectedValueOnce(new Error("provider codex install already in progress"));
        const { container } = renderPanel({ installed: false });
        fireEvent.click(await screen.findByTestId("agent-create-install-start"));

        await waitFor(() => expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument());
        const details = container.querySelector(".install-details") as HTMLDetailsElement;
        await waitFor(() => expect(details.open).toBe(true));
        expect(logRows(container)).toEqual([["Install failed: provider codex install already in progress", "error"]]);
    });

    it("a running install offers Cancel install, which stops it", async () => {
        renderPanel({ installed: false });
        await startAndSubscribe();
        const cancel = await screen.findByTestId("agent-create-install-cancel");
        expect(cancel.textContent).toContain("Cancel install");
        expect(screen.queryByTestId("agent-create-install-start")).toBeNull();
        fireEvent.click(cancel);
        await waitFor(() => expect(RpcApi.InstallCancelCommand).toHaveBeenCalledWith(expect.anything(), { sessionId: "sess-1" }));
    });

    it("keeps installing when the panel closes, shows the same run when it reopens, and toasts the end once closed", async () => {
        const first = renderPanel({ installed: false });
        const send = await startAndSubscribe();
        first.unmount();
        // Closing is not Cancel.
        expect(RpcApi.InstallCancelCommand).not.toHaveBeenCalled();
        expect(agentInstallSession("codex").state()).toBe("running");

        // Reopened: the same run, still running.
        const second = renderPanel({ installed: false });
        await screen.findByTestId("agent-create-install-cancel");
        expect(RpcApi.InstallStartCommand).toHaveBeenCalledTimes(1);
        second.unmount();

        send({ op: "done", ok: true });
        await waitFor(() => expect(pushNotification).toHaveBeenCalledTimes(1));
        expect(pushNotification.mock.calls[0][0]).toMatchObject({ title: "Codex is installed", type: "info" });
    });

    it("the install step stays in the trail while the CLI installs even if the picker already says installed", async () => {
        const { setInstalled } = renderPanel({ installed: false });
        await startAndSubscribe();
        setInstalled(true);
        // A run in progress keeps the Install step current.
        await screen.findByTestId("agent-create-install-cancel");
        expect(currentStep()).toBe("Install");
    });
});

describe("AgentCreatePanel — Install step, a CLI AgentMux doesn't install", () => {
    const vendorTemplate = baseTemplate({ name: "Antigravity", provider: "antigravity" });

    it("shows the vendor's install command for this platform and its instructions link", async () => {
        renderPanel({ template: vendorTemplate, installed: false });
        await screen.findByTestId("agent-create-vendor-install");
        expect(screen.queryByTestId("agent-create-npm-install")).toBeNull();
        expect(screen.getByTestId("agent-create-vendor-command").textContent).toBe("curl -fsSL https://example.test/install.sh | bash");
        const link = screen.getByText(/Antigravity's install instructions/).closest("a")!;
        expect(link.getAttribute("href")).toBe("https://example.test/agy-docs");
        fireEvent.click(link);
        expect(openExternal).toHaveBeenCalledWith("https://example.test/agy-docs");
        expect(trail()).toEqual(["Install", "Create"]);
    });

    it("shows the Windows command on Windows", async () => {
        getPlatform.mockReturnValue("win32");
        renderPanel({ template: vendorTemplate, installed: false });
        await screen.findByTestId("agent-create-vendor-install");
        expect(screen.getByTestId("agent-create-vendor-command").textContent).toBe("irm https://example.test/install.ps1 | iex");
    });

    it("Check again re-checks the CLI and moves on once it is found", async () => {
        const { recheckInstalled } = renderPanel({ template: vendorTemplate, installed: false });
        fireEvent.click(await screen.findByTestId("agent-create-check-again"));
        expect(recheckInstalled).toHaveBeenCalledTimes(1);
        await screen.findByTestId("agent-create-form");
        // Never touches AgentMux's own installer.
        expect(RpcApi.InstallStartCommand).not.toHaveBeenCalled();
    });

    it("stays on the install step while the CLI is still not found", async () => {
        const recheckInstalled = vi.fn().mockResolvedValue(undefined);
        renderPanel({ template: vendorTemplate, installed: false, recheckInstalled });
        fireEvent.click(await screen.findByTestId("agent-create-check-again"));
        await waitFor(() => expect(recheckInstalled).toHaveBeenCalledTimes(1));
        await waitFor(() => expect((screen.getByTestId("agent-create-check-again") as HTMLButtonElement).disabled).toBe(false));
        expect(screen.getByTestId("agent-create-vendor-install")).toBeInTheDocument();
    });
});
