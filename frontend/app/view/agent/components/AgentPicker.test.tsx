// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the two-tier AgentPicker layout
 * (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md — Phase 1).
 *
 * Covered:
 *  - the card grid (the "+ New from template" tier) only renders
 *    definitions with `is_seeded === 1` — user-owned agents go to the
 *    `MyAgentsList` sibling above the grid (mocked in this file).
 *  - clicking a template card opens the `create-from-template` modal
 *    request via the tab-modal API, not the `launch-agent` request.
 *  - the modal's `onCreatedAndLaunch` hook eventually fires
 *    `launchAgentDefinition` with the picked bindings.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => {
    const RpcApi = {
        // Default resolved values keep the picker's mount-time RPC
        // calls from hanging when an individual test forgets to set
        // a fresh `.mockResolvedValue`. Tests override the
        // ListAgentDefinitions stub in `beforeEach`.
        ListAgentDefinitionsCommand: vi.fn().mockResolvedValue([]),
        ListRecentSessionsCommand: vi.fn().mockResolvedValue([]),
        ListNamedAgentsCommand: vi.fn().mockResolvedValue([]),
        InstallCheckCommand: vi.fn().mockResolvedValue({ installed: true }),
        ResolvePrereqsCommand: vi.fn().mockResolvedValue({ results: [] }),
        AgentSessionReadCommand: vi
            .fn()
            .mockResolvedValue({ content: null, modts: null }),
        AgentSessionArchiveCommand: vi.fn().mockResolvedValue({}),
        AgentDefCreateFromTemplateCommand: vi.fn().mockResolvedValue({
            definition_id: "new-def",
            identity_id: "",
            memory_id: "",
        }),
        ListIdentityBundlesCommand: vi.fn().mockResolvedValue([]),
        ListMemoriesCommand: vi.fn().mockResolvedValue([]),
    };
    return { RpcApi };
});
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn(() => () => {}),
}));

const tabModalOpen = vi.fn();
const tabModalReplace = vi.fn();
const tabModalClose = vi.fn();
vi.mock("@/app/tab/tab-modal", () => ({
    useTabModal: () => ({
        open: tabModalOpen,
        replace: tabModalReplace,
        close: tabModalClose,
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
            data-is-template={String(props.agent.is_seeded === 1)}
            data-has-session={String(!!props.hasCurrentSession)}
            data-launching={String(!!props.launching)}
            onClick={(e) => props.onLaunch(props.agent, e)}
        >
            {props.agent.name}
        </button>
    ),
}));

vi.mock("./AgentActionBar", () => ({
    AgentActionBar: () => null,
}));

vi.mock("./MyAgentsList", () => ({
    MyAgentsList: () => null,
}));

import { AgentPicker } from "./AgentPicker";

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;

const ts = () => 1_700_000_000_000;

const baseDef = (over: Partial<AgentDefinition>): AgentDefinition =>
    ({
        id: "agent-x",
        slug: "x",
        name: "X",
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
        ...over,
    }) as AgentDefinition;

const claudeTemplate = baseDef({
    id: "tpl-claude",
    slug: "claude",
    name: "Claude Code",
    is_seeded: 1,
});

const userAgent = baseDef({
    id: "user-maks",
    slug: "maks",
    name: "Maks",
    is_seeded: 0,
});

const makeMockModel = () => ({
    blockId: "blk-1",
    blockAtom: () => ({ meta: {} }),
    nodejsError: null as string | null,
    launchAgentDefinition: vi.fn().mockResolvedValue(undefined),
});

beforeEach(async () => {
    vi.clearAllMocks();
    tabModalOpen.mockClear();
    tabModalReplace.mockClear();
    tabModalClose.mockClear();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
    vi.mocked(RpcApi.ListAgentDefinitionsCommand).mockResolvedValue([
        claudeTemplate,
        userAgent,
    ]);
});

afterEach(() => {
    cleanup();
});

describe("AgentPicker — two-tier layout (Phase 1)", () => {
    it("renders templates section only with is_seeded === 1 cards", async () => {
        const model = makeMockModel();
        render(() => <AgentPicker model={model as any} />);
        await waitFor(() => {
            expect(screen.queryByTestId("agent-card-tpl-claude")).not.toBeNull();
        });
        // Only the template renders as a card. User agents go to
        // MyAgentsList (mocked).
        expect(screen.queryByTestId("agent-card-user-maks")).toBeNull();
    });

    it("renders the '+ New from template' section header", async () => {
        const model = makeMockModel();
        render(() => <AgentPicker model={model as any} />);
        const header = await screen.findByTestId("agent-templates-header");
        expect(header).toHaveTextContent("New from template");
    });

    it("clicks on a template open the create-from-template modal", async () => {
        const model = makeMockModel();
        render(() => <AgentPicker model={model as any} />);
        const card = await screen.findByTestId("agent-card-tpl-claude");
        fireEvent.click(card);
        await waitFor(() => expect(tabModalOpen).toHaveBeenCalled());

        const req = tabModalOpen.mock.calls[0][0];
        expect(req.kind).toBe("create-from-template");
        expect(req.template.id).toBe("tpl-claude");
        // Template clicks never go through launchAgentDefinition
        // directly — the modal's onCreatedAndLaunch handles that after
        // the create RPC.
        expect(model.launchAgentDefinition).not.toHaveBeenCalled();
    });

    it("template card suppresses the '+ New session' pill", async () => {
        const model = makeMockModel();
        render(() => <AgentPicker model={model as any} />);
        const card = await screen.findByTestId("agent-card-tpl-claude");
        expect(card.getAttribute("data-has-session")).toBe("false");
    });

    it("modal onCreatedAndLaunch fires launchAgentDefinition with the picked bindings", async () => {
        const model = makeMockModel();
        render(() => <AgentPicker model={model as any} />);
        const card = await screen.findByTestId("agent-card-tpl-claude");
        fireEvent.click(card);
        await waitFor(() => expect(tabModalOpen).toHaveBeenCalled());

        const req = tabModalOpen.mock.calls[0][0];
        await req.onCreatedAndLaunch(
            "new-def-id",
            "id-work",
            "mem-notes",
            "Mary",
        );
        expect(model.launchAgentDefinition).toHaveBeenCalledTimes(1);
        const [stubAgent, overrides] = model.launchAgentDefinition.mock.calls[0];
        expect(stubAgent.id).toBe("new-def-id");
        expect(stubAgent.name).toBe("Mary");
        expect(stubAgent.is_seeded).toBe(0);
        expect(stubAgent.parent_id).toBe("tpl-claude");
        expect(overrides.identityId).toBe("id-work");
        expect(overrides.memoryId).toBe("mem-notes");
        expect(overrides.instanceName).toBe("Mary");
        expect(overrides.continueOfInstanceId).toBeUndefined();
    });
});
