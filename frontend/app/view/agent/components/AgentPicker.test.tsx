// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the Option E default-continue UX on `AgentPicker`
 * (PR #1008 frontend follow-up to PR #1007 backend).
 *
 * The agent picker should:
 *   1. Auto-continue (no modal) when the agent has a non-empty
 *      session zone (`agent:<defId>:current`).
 *   2. Open the launch modal when the agent has no session yet.
 *   3. Force the launch modal regardless of session content when the
 *      user clicks with Shift/Ctrl/Alt held (escape hatch).
 *
 * Spec: SPEC_CONTINUATION_SESSION_PERSISTENCE_2026_05_23.md.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => {
    const RpcApi = {
        ListAgentDefinitionsCommand: vi.fn(),
        ListRecentSessionsCommand: vi.fn(),
        ListNamedAgentsCommand: vi.fn(),
        InstallCheckCommand: vi.fn(),
        ResolvePrereqsCommand: vi.fn(),
        AgentSessionReadCommand: vi.fn(),
        AgentSessionArchiveCommand: vi.fn(),
    };
    return { RpcApi };
});
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn(() => () => {}),
}));

vi.mock("@/app/tab/tab-modal", () => ({
    useTabModal: () => ({
        open: vi.fn(),
        replace: vi.fn(),
        close: vi.fn(),
    }),
}));

vi.mock("@/util/platformutil", () => ({
    getPlatform: () => "linux",
}));

vi.mock("../providers", () => ({
    getProvider: (id: string) =>
        id === "claude"
            ? {
                  id: "claude",
                  npmPackage: null, // no install needed
                  cliCommand: "claude",
                  systemPrereqs: [],
              }
            : undefined,
}));

vi.mock("./AgentCard", () => ({
    AgentCard: (props: any) => (
        <button
            data-testid={`agent-card-${props.agent.id}`}
            data-has-session={String(!!props.hasCurrentSession)}
            data-launching={String(!!props.launching)}
            onClick={(e) => props.onLaunch(props.agent, e)}
            onContextMenu={(e) => {
                // tests use right-click to simulate Shift+click since
                // jsdom MouseEvent ctor wires shiftKey deterministically
                e.preventDefault();
                const fake = new MouseEvent("click", {
                    shiftKey: true,
                    bubbles: true,
                });
                props.onLaunch(props.agent, fake);
            }}
        >
            {props.agent.name}
        </button>
    ),
}));

vi.mock("./AgentActionBar", () => ({
    AgentActionBar: () => null,
}));

vi.mock("./RecentSessionsList", () => ({
    RecentSessionsList: () => null,
}));

import { AgentPicker } from "./AgentPicker";

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;

const ts = () => 1_700_000_000_000;

const claudeAgent = {
    id: "agent-claude",
    slug: "claude",
    name: "Claude Code",
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
} as AgentDefinition;

const makeMockModel = () => ({
    blockId: "blk-1",
    blockAtom: () => ({ meta: {} }),
    nodejsError: null as string | null,
    launchAgentDefinition: vi.fn().mockResolvedValue(undefined),
});

beforeEach(async () => {
    vi.clearAllMocks();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
    vi.mocked(RpcApi.ListAgentDefinitionsCommand).mockResolvedValue([claudeAgent]);
    vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue([]);
    vi.mocked(RpcApi.ListNamedAgentsCommand).mockResolvedValue([]);
});

afterEach(() => {
    cleanup();
});

const flush = () => new Promise<void>((r) => setTimeout(r, 0));

describe("AgentPicker — default-continue UX (Option E)", () => {
    it("auto-continues without opening the modal when the agent has a current session", async () => {
        // Agent has a session in its zone.
        vi.mocked(RpcApi.AgentSessionReadCommand).mockResolvedValue({
            content: '{"schemaVersion":1,"nodes":[]}',
            modts: ts(),
        });
        const model = makeMockModel();
        render(() => <AgentPicker model={model as any} />);

        // Wait for definitions to load + session probe.
        await flush();
        await flush();
        await flush();

        const card = await screen.findByTestId("agent-card-agent-claude");
        expect(card.getAttribute("data-has-session")).toBe("true");

        fireEvent.click(card);
        await flush();
        await flush();

        // launchAgentDefinition called → no modal.
        expect(model.launchAgentDefinition).toHaveBeenCalledTimes(1);
        const [, overrides] = model.launchAgentDefinition.mock.calls[0];
        // Default-continue does NOT set continueOfInstanceId — the
        // agent zone is structurally continuous.
        expect(overrides.continueOfInstanceId).toBeUndefined();
        expect(overrides.instanceName).toBe("Claude Code");
    });

    it("opens the launch modal when the agent has no current session", async () => {
        vi.mocked(RpcApi.AgentSessionReadCommand).mockResolvedValue({
            content: null,
            modts: null,
        });
        const model = makeMockModel();
        render(() => <AgentPicker model={model as any} />);

        await flush();
        await flush();
        await flush();

        const card = await screen.findByTestId("agent-card-agent-claude");
        expect(card.getAttribute("data-has-session")).toBe("false");

        fireEvent.click(card);
        await flush();
        await flush();

        // No auto-continue — would have set launching state via modal.
        expect(model.launchAgentDefinition).not.toHaveBeenCalled();
    });

    it("forces the launch modal even with a current session when Shift is held", async () => {
        vi.mocked(RpcApi.AgentSessionReadCommand).mockResolvedValue({
            content: '{"schemaVersion":1,"nodes":[]}',
            modts: ts(),
        });
        const model = makeMockModel();
        render(() => <AgentPicker model={model as any} />);

        await flush();
        await flush();
        await flush();

        const card = await screen.findByTestId("agent-card-agent-claude");
        expect(card.getAttribute("data-has-session")).toBe("true");

        // Simulate shift+click via our mock card's contextmenu handler
        // (the mock invokes onLaunch with a synthetic shiftKey=true
        // MouseEvent — see vi.mock("./AgentCard") above).
        fireEvent.contextMenu(card);
        await flush();
        await flush();

        // Shift forces the modal path; launchAgentDefinition NOT called
        // directly (the modal would call it after the user submits).
        expect(model.launchAgentDefinition).not.toHaveBeenCalled();
    });
});
