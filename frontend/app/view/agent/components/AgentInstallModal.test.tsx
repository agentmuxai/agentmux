// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentInstallModal must install the agent's EFFECTIVE (bundle-resolved)
 * provider, not a possibly-drifted `agent.provider` column — #2594, same
 * "gate vs. actual launch can disagree" risk class #2592/#2596/#2607/#2609
 * fixed. This modal is the one place that actually determines which CLI
 * package gets installed (`InstallStartCommand`'s `providerId`/
 * `npmPackage`), so getting this wrong installs the wrong provider's CLI
 * while the agent's real (bundle) provider stays uninstalled.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        InstallStartCommand: vi.fn().mockResolvedValue({ sessionId: "sess-1" }),
        InstallCancelCommand: vi.fn().mockResolvedValue({}),
        // Backs `resolveEffectiveLaunchProvider`'s bound-bundle resolution.
        GetBundleCommand: vi.fn().mockResolvedValue(undefined),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: vi.fn() } }));
vi.mock("@/util/clipboard", () => ({ writeText: vi.fn().mockResolvedValue(undefined) }));

vi.mock("../defaults/cli-catalog", () => ({
    getCliCatalogEntry: (id: string) =>
        id === "codex"
            ? { displayName: "Codex", icon: "🤖", popoverMarkdown: "" }
            : { displayName: "Claude Code", icon: "✦", popoverMarkdown: "" },
}));

vi.mock("../providers", () => ({
    getProvider: (id: string) => {
        if (id === "claude") {
            return { id: "claude", cliCommand: "claude", npmPackage: null, pinnedVersion: "1.0.0" };
        }
        if (id === "codex") {
            return { id: "codex", cliCommand: "codex", npmPackage: "@openai/codex", pinnedVersion: "0.116.0" };
        }
        return undefined;
    },
}));

import { AgentInstallModalPanel } from "./AgentInstallModal";
import { RpcApi } from "@/app/store/rpc-api";
import { muxEventSubscribe } from "@/app/store/mps";
import { resetInstallDetailsPrefs } from "@/element/install/InstallProgress";
import { LOG_ROW_PX } from "@/element/install/LogView";
import type { AgentDefinition } from "@/app/store/rpc-api";

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    resetInstallDetailsPrefs();
});

const ts = () => 1_700_000_000_000;

const baseAgent = (over: Partial<AgentDefinition>): AgentDefinition =>
    ({
        id: "agent-1",
        slug: "agent-1",
        name: "Agent One",
        icon: "",
        provider: "claude",
        description: "",
        working_directory: "",
        shell: "",
        provider_flags: "",
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: ts(),
        agent_type: "host",
        environment: "local",
        agent_bus_id: "",
        is_seeded: 0,
        memory_id: "",
        ...over,
    }) as AgentDefinition;

describe("AgentInstallModal — installs the bound bundle's provider, not a drifted agent.provider (#2594)", () => {
    it("starts the install against the resolved (bundle) provider, not the drifted column", async () => {
        // Drifted `.provider` column says "claude", but the bound
        // bundle's REAL provider is "codex" — a correct install must
        // fetch/run codex's CLI, not claude's.
        vi.mocked(RpcApi.GetBundleCommand).mockResolvedValue({ provider: "codex" } as any);
        const agent = baseAgent({ provider: "claude", memory_id: "mem-1" });

        render(() => (
            <AgentInstallModalPanel agent={agent} onCancel={vi.fn()} onInstalled={vi.fn()} />
        ));

        const installBtn = await screen.findByText("Install now");
        fireEvent.click(installBtn);

        await waitFor(() => expect(RpcApi.InstallStartCommand).toHaveBeenCalled());
        const call = vi.mocked(RpcApi.InstallStartCommand).mock.calls[0][1];
        expect(call).toMatchObject({
            providerId: "codex",
            cliCommand: "codex",
            npmPackage: "@openai/codex",
            pinnedVersion: "0.116.0",
        });
    });

    it("shows the resolved (bundle) provider's display name in the header, not the drifted column's", async () => {
        vi.mocked(RpcApi.GetBundleCommand).mockResolvedValue({ provider: "codex" } as any);
        const agent = baseAgent({ provider: "claude", memory_id: "mem-1", name: "Agent One" });

        render(() => (
            <AgentInstallModalPanel agent={agent} onCancel={vi.fn()} onInstalled={vi.fn()} />
        ));

        await waitFor(() => {
            expect(screen.getByText(/Install Codex/)).toBeInTheDocument();
        });
    });

    it("an unbound agent (no memory_id) installs its own agent.provider directly", async () => {
        const agent = baseAgent({ provider: "codex", memory_id: "" });

        render(() => (
            <AgentInstallModalPanel agent={agent} onCancel={vi.fn()} onInstalled={vi.fn()} />
        ));

        const installBtn = await screen.findByText("Install now");
        fireEvent.click(installBtn);

        await waitFor(() => expect(RpcApi.InstallStartCommand).toHaveBeenCalled());
        const call = vi.mocked(RpcApi.InstallStartCommand).mock.calls[0][1];
        expect(call).toMatchObject({ providerId: "codex", cliCommand: "codex" });
        // Unbound agents never even trigger a bundle fetch.
        expect(RpcApi.GetBundleCommand).not.toHaveBeenCalled();
    });
});

/** Clicks "Install now" and returns a function that delivers install_chunk events. */
async function startAndSubscribe(): Promise<(data: Record<string, unknown>) => void> {
    fireEvent.click(await screen.findByText("Install now"));
    await waitFor(() => expect(muxEventSubscribe).toHaveBeenCalled());
    const { handler } = vi.mocked(muxEventSubscribe).mock.calls[0][0] as { handler: (e: unknown) => void };
    return (data) => handler({ data: { sessionId: "sess-1", ...data } });
}

const stepStatus = (container: HTMLElement) =>
    Array.from(container.querySelectorAll<HTMLElement>(".install-step")).map((li) => [
        li.querySelector(".install-step-label")?.textContent,
        li.dataset.status,
    ]);

const logRows = (container: HTMLElement) =>
    Array.from(container.querySelectorAll<HTMLElement>(".install-log-line")).map((el) => [el.textContent, el.dataset.tone]);

const description = (container: HTMLElement) => container.querySelector(".modal-panel-description")?.textContent;

const renderCodex = () =>
    render(() => (
        <AgentInstallModalPanel
            agent={baseAgent({ provider: "codex", memory_id: "" })}
            onCancel={vi.fn()}
            onInstalled={vi.fn()}
        />
    ));

describe("AgentInstallModal — shared install dialog (SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md)", () => {
    it("shows the plan with every step pending before the install starts", async () => {
        const { container } = renderCodex();
        await screen.findByText("Install now");
        expect(description(container)).toBe("Needs an internet connection.");
        expect(stepStatus(container)).toEqual([
            ["Check requirements", "pending"],
            ["Download packages", "pending"],
            ["Set up files", "pending"],
            ["Run setup scripts", "pending"],
            ["Check Codex is installed", "pending"],
        ]);
    });

    it("marks the primary button as the modal's initial focus, so the console never gets it", async () => {
        renderCodex();
        const btn = await screen.findByText("Install now");
        expect(btn.closest("button")?.hasAttribute("data-modal-initial-focus")).toBe(true);
    });

    it("keeps Details collapsed by default while the log still fills", async () => {
        const { container } = renderCodex();
        const send = await startAndSubscribe();
        send({ line: "npm http fetch GET 200 https://registry.npmjs.org/chalk/-/chalk-4.1.2.tgz 12ms", stream: "stderr" });
        const details = container.querySelector(".install-details") as HTMLDetailsElement;
        expect(details.open).toBe(false);
        await waitFor(() => expect(logRows(container).some(([t]) => t?.includes("chalk-4.1.2.tgz"))).toBe(true));
    });

    it("advances the steps from npm output and does not paint healthy stderr red", async () => {
        const { container } = renderCodex();
        const send = await startAndSubscribe();
        await waitFor(() => expect(stepStatus(container)[0]).toEqual(["Check requirements", "active"]));

        send({ line: "npm http fetch GET 200 https://registry.npmjs.org/chalk/-/chalk-4.1.2.tgz 12ms", stream: "stderr" });
        await waitFor(() => expect(stepStatus(container)[1]).toEqual(["Download packages", "active"]));
        expect(container.querySelector(".install-step-hint")?.textContent).toBe("1 fetched");
        expect(description(container)).toBe("Needs an internet connection.");

        send({ line: "added 1 package in 1s", stream: "stdout" });
        send({ op: "done", ok: true });
        await waitFor(() => expect(screen.getByText("Codex is installed")).toBeInTheDocument());
        // The pre-install requirement no longer applies once it's done.
        expect(description(container)).toBe("Ready to launch.");
        expect(stepStatus(container).map(([, st]) => st)).toEqual(["done", "done", "done", "skipped", "done"]);
        expect(logRows(container).find(([t]) => t?.includes("chalk-4.1.2.tgz"))?.[1]).toBe("normal");
    });

    it("on failure explains it in the step list and opens Details at the first error", async () => {
        const { container } = renderCodex();
        const send = await startAndSubscribe();
        send({ line: "npm http fetch GET https://registry.npmjs.org/npm attempt 1 failed with ENOTFOUND", stream: "stderr" });
        for (let i = 0; i < 30; i++) send({ line: `npm verbose stack frame ${i}`, stream: "stderr" });
        send({ line: "npm error code ENOTFOUND", stream: "stderr" });
        send({ line: "npm error network This is a problem related to network connectivity.", stream: "stderr" });
        send({ op: "done", ok: false, error: "npm exited Some(1)" });

        await waitFor(() => expect(screen.getByText("Couldn't reach the package server.")).toBeInTheDocument());
        expect(stepStatus(container)[1]).toEqual(["Download packages", "failed"]);
        expect(description(container)).toBe("The install didn't finish.");
        expect(screen.getByText("Retry")).toBeInTheDocument();

        const details = container.querySelector(".install-details") as HTMLDetailsElement;
        await waitFor(() => expect(details.open).toBe(true));
        // The first error is line 31; scrolled so it sits two rows from the top.
        const log = container.querySelector(".install-log") as HTMLElement;
        await waitFor(() => expect(log.scrollTop).toBe((31 - 2) * LOG_ROW_PX));
        expect(logRows(container).find(([t]) => t === "npm error code ENOTFOUND")?.[1]).toBe("error");
        // The backend's own reason ends the log.
        expect(logRows(container).at(-1)).toEqual(["Install failed: npm exited Some(1)", "error"]);
    });

    it("scrolls to the first error even when Details was already open during the install", async () => {
        const { container } = renderCodex();
        const send = await startAndSubscribe();
        const details = container.querySelector(".install-details") as HTMLDetailsElement;
        details.open = true;
        fireEvent(details, new Event("toggle"));

        for (let i = 0; i < 30; i++) send({ line: `npm verbose line ${i}`, stream: "stderr" });
        send({ line: "npm error code ENOTFOUND", stream: "stderr" });
        send({ op: "done", ok: false, error: "npm exited Some(1)" });
        const log = container.querySelector(".install-log") as HTMLElement;
        await waitFor(() => expect(log.scrollTop).toBe((30 - 2) * LOG_ROW_PX));
    });

    it("shows why the install couldn't start when the start RPC itself fails", async () => {
        vi.mocked(RpcApi.InstallStartCommand).mockRejectedValueOnce(new Error("provider codex install already in progress"));
        const { container } = renderCodex();
        fireEvent.click(await screen.findByText("Install now"));

        await waitFor(() => expect(screen.getByText("Retry")).toBeInTheDocument());
        const details = container.querySelector(".install-details") as HTMLDetailsElement;
        await waitFor(() => expect(details.open).toBe(true));
        expect(logRows(container)).toEqual([["Install failed: provider codex install already in progress", "error"]]);
    });

    it("cancels the running install when the dialog unmounts", async () => {
        const { unmount } = renderCodex();
        await startAndSubscribe();
        unmount();
        expect(RpcApi.InstallCancelCommand).toHaveBeenCalledWith(expect.anything(), { sessionId: "sess-1" });
    });

    it("still tells the picker the CLI is installed when the success screen is dismissed without a button", async () => {
        const onInstalled = vi.fn();
        const { unmount } = render(() => (
            <AgentInstallModalPanel
                agent={baseAgent({ provider: "codex", memory_id: "" })}
                onCancel={vi.fn()}
                onInstalled={onInstalled}
            />
        ));
        const send = await startAndSubscribe();
        send({ op: "done", ok: true });
        await screen.findByText("Continue to Launch");
        unmount();
        expect(onInstalled).toHaveBeenCalledWith(false);
    });
});
