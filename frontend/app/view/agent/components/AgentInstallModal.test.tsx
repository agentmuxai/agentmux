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
vi.mock("@/app/store/global", () => ({
    atoms: { fullConfigAtom: () => ({}) },
    getSettingsKeyAtom: () => () => false,
}));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: vi.fn() } }));
vi.mock("@/app/view/term/termutil", () => ({
    computeTermThemeFromSettings: () => [{}],
}));
vi.mock("@/util/clipboard", () => ({ writeText: vi.fn() }));

// Stub xterm.js entirely — jsdom has no canvas/ResizeObserver support xterm
// needs for real construction. The fake records what was written (with its
// ANSI colour codes) and where it was scrolled, so tests can check both.
// Its buffer wraps each line at `fake.cols` columns like xterm does,
// flagging continuation rows `isWrapped`.
const { terminals, FakeTerminal, fake } = vi.hoisted(() => {
    const ANSI_SGR = new RegExp(`${String.fromCharCode(27)}\\[[0-9;]*m`, "g");
    const fake = { cols: 80 };
    const terminals: InstanceType<typeof FakeTerminal>[] = [];
    class FakeTerminal {
        options: Record<string, unknown> = {};
        cols = fake.cols;
        written: string[] = [];
        scrolledTo: number | null = null;
        constructor(public ctorOptions: Record<string, unknown> = {}) {
            terminals.push(this);
        }
        onSelectionChange(): void {}
        attachCustomKeyEventHandler(): void {}
        loadAddon(): void {}
        open(): void {}
        writeln(): void {}
        write(data: string, cb?: () => void): void {
            if (data) this.written.push(...data.split("\r\n").filter(Boolean));
            cb?.();
        }
        scrollToLine(n: number): void {
            this.scrolledTo = n;
        }
        clear(): void {}
        dispose(): void {}
        get buffer() {
            const rows: { text: string; isWrapped: boolean }[] = [];
            for (const line of this.written) {
                const plain = line.replace(ANSI_SGR, "");
                for (let at = 0; at === 0 || at < plain.length; at += this.cols) {
                    rows.push({ text: plain.slice(at, at + this.cols), isWrapped: at > 0 });
                }
            }
            return {
                active: {
                    length: rows.length,
                    getLine: (i: number) =>
                        i < rows.length
                            ? { isWrapped: rows[i].isWrapped, translateToString: () => rows[i].text }
                            : null,
                },
            };
        }
        getSelection(): string {
            return "";
        }
    }
    return { terminals, FakeTerminal, fake };
});
vi.mock("@xterm/xterm", () => ({ Terminal: FakeTerminal }));
vi.mock("@xterm/addon-fit", () => ({
    FitAddon: class {
        fit(): void {}
    },
}));

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

// jsdom has no ResizeObserver — AgentInstallModalPanel's onMount
// constructs one unconditionally.
class FakeResizeObserver {
    observe(): void {}
    disconnect(): void {}
}
(globalThis as any).ResizeObserver = FakeResizeObserver;

import { AgentInstallModalPanel } from "./AgentInstallModal";
import { RpcApi } from "@/app/store/rpc-api";
import { muxEventSubscribe } from "@/app/store/mps";
import type { AgentDefinition } from "@/app/store/rpc-api";

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    terminals.length = 0;
    fake.cols = 80;
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

describe("AgentInstallModal — two layers (SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md phase 1)", () => {
    it("shows the plan with every step pending before the install starts", async () => {
        const agent = baseAgent({ provider: "codex", memory_id: "" });
        const { container } = render(() => (
            <AgentInstallModalPanel agent={agent} onCancel={vi.fn()} onInstalled={vi.fn()} />
        ));
        await screen.findByText("Install now");
        expect(container.querySelector(".modal-panel-description")?.textContent).toBe("Needs an internet connection.");
        expect(stepStatus(container)).toEqual([
            ["Check requirements", "pending"],
            ["Download packages", "pending"],
            ["Set up files", "pending"],
            ["Run setup scripts", "pending"],
            ["Check Codex is installed", "pending"],
        ]);
    });

    it("marks the primary button as the modal's initial focus, so the console never gets it", async () => {
        const agent = baseAgent({ provider: "codex", memory_id: "" });
        render(() => <AgentInstallModalPanel agent={agent} onCancel={vi.fn()} onInstalled={vi.fn()} />);
        const btn = await screen.findByText("Install now");
        expect(btn.closest("button")?.hasAttribute("data-modal-initial-focus")).toBe(true);
    });

    it("keeps Details collapsed by default and doesn't create the terminal until it opens", async () => {
        const agent = baseAgent({ provider: "codex", memory_id: "" });
        const { container } = render(() => (
            <AgentInstallModalPanel agent={agent} onCancel={vi.fn()} onInstalled={vi.fn()} />
        ));
        const send = await startAndSubscribe();
        send({ line: "npm http fetch GET 200 https://registry.npmjs.org/chalk/-/chalk-4.1.2.tgz 12ms", stream: "stderr" });

        const details = container.querySelector(".agent-install-modal-details") as HTMLDetailsElement;
        expect(details.open).toBe(false);
        expect(terminals).toHaveLength(0);

        // Opening it creates the terminal and replays everything so far.
        details.open = true;
        fireEvent(details, new Event("toggle"));
        await waitFor(() => expect(terminals).toHaveLength(1));
        await waitFor(() => expect(terminals[0].written.some((l) => l.includes("chalk-4.1.2.tgz"))).toBe(true));

        // Close again so the remembered preference doesn't leak into later tests.
        details.open = false;
        fireEvent(details, new Event("toggle"));
    });

    it("advances the steps from npm output and does not paint healthy stderr red", async () => {
        const agent = baseAgent({ provider: "codex", memory_id: "" });
        const { container } = render(() => (
            <AgentInstallModalPanel agent={agent} onCancel={vi.fn()} onInstalled={vi.fn()} />
        ));
        const send = await startAndSubscribe();
        await waitFor(() => expect(stepStatus(container)[0]).toEqual(["Check requirements", "active"]));

        send({ line: "npm http fetch GET 200 https://registry.npmjs.org/chalk/-/chalk-4.1.2.tgz 12ms", stream: "stderr" });
        await waitFor(() => expect(stepStatus(container)[1]).toEqual(["Download packages", "active"]));
        expect(container.querySelector(".install-step-hint")?.textContent).toBe("1 fetched");

        expect(container.querySelector(".modal-panel-description")?.textContent).toBe("Needs an internet connection.");

        send({ line: "added 1 package in 1s", stream: "stdout" });
        send({ op: "done", ok: true });
        await waitFor(() => expect(screen.getByText("Codex is installed")).toBeInTheDocument());
        // The pre-install requirement no longer applies once it's done.
        expect(container.querySelector(".modal-panel-description")?.textContent).toBe("Ready to launch.");
        expect(stepStatus(container).map(([, st]) => st)).toEqual(["done", "done", "done", "skipped", "done"]);

        const details = container.querySelector(".agent-install-modal-details") as HTMLDetailsElement;
        details.open = true;
        fireEvent(details, new Event("toggle"));
        await waitFor(() => expect(terminals[0]?.written.length).toBeGreaterThan(0));
        const fetchLine = terminals[0].written.find((l) => l.includes("chalk-4.1.2.tgz"))!;
        expect(fetchLine.startsWith("\x1b[31m")).toBe(false);
        details.open = false;
        fireEvent(details, new Event("toggle"));
    });

    it("on failure explains it in the step list and opens Details at the first error", async () => {
        const agent = baseAgent({ provider: "codex", memory_id: "" });
        const { container } = render(() => (
            <AgentInstallModalPanel agent={agent} onCancel={vi.fn()} onInstalled={vi.fn()} />
        ));
        const send = await startAndSubscribe();
        send({ line: "npm http fetch GET https://registry.npmjs.org/npm attempt 1 failed with ENOTFOUND", stream: "stderr" });
        send({ line: "npm error code ENOTFOUND", stream: "stderr" });
        send({ line: "npm error network This is a problem related to network connectivity.", stream: "stderr" });
        send({ op: "done", ok: false, error: "npm exited Some(1)" });

        await waitFor(() => expect(screen.getByText("Couldn't reach the package server.")).toBeInTheDocument());
        expect(stepStatus(container)[1]).toEqual(["Download packages", "failed"]);
        expect(container.querySelector(".modal-panel-description")?.textContent).toBe("The install didn't finish.");
        expect(screen.getByText("Retry")).toBeInTheDocument();

        const details = container.querySelector(".agent-install-modal-details") as HTMLDetailsElement;
        await waitFor(() => expect(details.open).toBe(true));
        await waitFor(() => expect(terminals[0]?.scrolledTo).not.toBeNull());
        const errorLine = terminals[0].written.find((l) => l.includes("npm error code ENOTFOUND"))!;
        expect(errorLine.startsWith("\x1b[31m")).toBe(true);
    });

    it("scrolls to the first error even when Details was already open during the install", async () => {
        const agent = baseAgent({ provider: "codex", memory_id: "" });
        const { container } = render(() => (
            <AgentInstallModalPanel agent={agent} onCancel={vi.fn()} onInstalled={vi.fn()} />
        ));
        const send = await startAndSubscribe();
        const details = container.querySelector(".agent-install-modal-details") as HTMLDetailsElement;
        details.open = true;
        fireEvent(details, new Event("toggle"));
        await waitFor(() => expect(terminals).toHaveLength(1));

        send({ line: "npm error code ENOTFOUND", stream: "stderr" });
        send({ op: "done", ok: false, error: "npm exited Some(1)" });
        await waitFor(() => expect(terminals[0].scrolledTo).not.toBeNull());

        details.open = false;
        fireEvent(details, new Event("toggle"));
    });

    it("finds a first error that xterm wrapped across rows in a narrow pane", async () => {
        fake.cols = 24;
        const agent = baseAgent({ provider: "codex", memory_id: "" });
        render(() => <AgentInstallModalPanel agent={agent} onCancel={vi.fn()} onInstalled={vi.fn()} />);
        const send = await startAndSubscribe();
        for (let i = 0; i < 5; i++) send({ line: `npm http fetch GET 200 https://registry.npmjs.org/pkg${i}`, stream: "stderr" });
        send({ line: "npm error code ENOTFOUND while fetching https://registry.npmjs.org/some-long-package-name", stream: "stderr" });
        for (let i = 0; i < 5; i++) send({ line: `npm error network trailing line ${i}`, stream: "stderr" });
        send({ op: "done", ok: false, error: "npm exited Some(1)" });

        await waitFor(() => expect(terminals[0]?.scrolledTo).not.toBeNull());
        // Every earlier line fills whole rows; the error starts right after them.
        const rows = terminals[0].buffer.active;
        let errorRow = -1;
        for (let i = 0; i < rows.length; i++) {
            if (rows.getLine(i)?.translateToString().startsWith("npm error code")) {
                errorRow = i;
                break;
            }
        }
        expect(errorRow).toBeGreaterThan(0);
        expect(terminals[0].scrolledTo).toBe(Math.max(0, errorRow - 2));

        const details = document.querySelector(".agent-install-modal-details") as HTMLDetailsElement;
        details.open = false;
        fireEvent(details, new Event("toggle"));
    });

    it("keeps as much scrollback in the console as the dialog retains in its log", async () => {
        const agent = baseAgent({ provider: "codex", memory_id: "" });
        const { container } = render(() => (
            <AgentInstallModalPanel agent={agent} onCancel={vi.fn()} onInstalled={vi.fn()} />
        ));
        await screen.findByText("Install now");
        const details = container.querySelector(".agent-install-modal-details") as HTMLDetailsElement;
        details.open = true;
        fireEvent(details, new Event("toggle"));
        await waitFor(() => expect(terminals).toHaveLength(1));
        expect(terminals[0].ctorOptions.scrollback).toBeGreaterThanOrEqual(20_000);
        details.open = false;
        fireEvent(details, new Event("toggle"));
    });

    it("shows why the install couldn't start when the start RPC itself fails", async () => {
        vi.mocked(RpcApi.InstallStartCommand).mockRejectedValueOnce(new Error("provider codex install already in progress"));
        const agent = baseAgent({ provider: "codex", memory_id: "" });
        const { container } = render(() => (
            <AgentInstallModalPanel agent={agent} onCancel={vi.fn()} onInstalled={vi.fn()} />
        ));
        fireEvent.click(await screen.findByText("Install now"));

        await waitFor(() => expect(screen.getByText("Retry")).toBeInTheDocument());
        const details = container.querySelector(".agent-install-modal-details") as HTMLDetailsElement;
        await waitFor(() => expect(details.open).toBe(true));
        // No npm output exists, so the reason has to reach Details itself.
        await waitFor(() =>
            expect(terminals[0]?.written.some((l) => l.includes("provider codex install already in progress"))).toBe(true),
        );
        details.open = false;
        fireEvent(details, new Event("toggle"));
    });
});
