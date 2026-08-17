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
        GetMemoryCommand: vi.fn().mockResolvedValue(undefined),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/wps", () => ({ waveEventSubscribe: vi.fn(() => () => {}) }));
vi.mock("@/app/store/global", () => ({
    atoms: { fullConfigAtom: () => ({}) },
    getSettingsKeyAtom: () => () => false,
}));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: vi.fn() } }));
vi.mock("@/app/view/term/termutil", () => ({
    computeTermThemeFromSettings: () => [{}],
}));
vi.mock("@/util/clipboard", () => ({ writeText: vi.fn() }));

// Stub xterm.js entirely — this test exercises provider resolution, not
// terminal rendering, and jsdom has no canvas/ResizeObserver support xterm
// needs for real construction.
vi.mock("@xterm/xterm", () => ({
    Terminal: class {
        options: Record<string, unknown> = {};
        onSelectionChange(): void {}
        attachCustomKeyEventHandler(): void {}
        loadAddon(): void {}
        open(): void {}
        writeln(): void {}
        write(): void {}
        clear(): void {}
        dispose(): void {}
        buffer = { active: { length: 0, getLine: () => null } };
        getSelection(): string {
            return "";
        }
    },
}));
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

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
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
        vi.mocked(RpcApi.GetMemoryCommand).mockResolvedValue({ provider: "codex" } as any);
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
        vi.mocked(RpcApi.GetMemoryCommand).mockResolvedValue({ provider: "codex" } as any);
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
        expect(RpcApi.GetMemoryCommand).not.toHaveBeenCalled();
    });
});
