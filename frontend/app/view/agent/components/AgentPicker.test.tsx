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
 *    request via the modal-layer API, not the `launch-agent` request.
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
        // Phase 2 (Q2 Decision Y) — hide templates.
        AgentDefHideCommand: vi.fn().mockResolvedValue({ ok: true }),
        AgentDefUnhideCommand: vi.fn().mockResolvedValue({ ok: true }),
        AgentDefListHiddenTemplatesCommand: vi.fn().mockResolvedValue([]),
        ListIdentityBundlesCommand: vi.fn().mockResolvedValue([]),
        ListMemoriesCommand: vi.fn().mockResolvedValue([]),
    };
    return { RpcApi };
});
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

// Phase 2: the ContextMenu helper shows a native menu via CEF IPC.
// In jsdom there's no host bridge; we capture the menu spec the
// picker requests so tests can fire the "Hide template" item's click
// handler directly. Defined inline inside the factory so vi.mock's
// top-of-file hoisting doesn't read `contextMenuShow` before the
// const declaration runs.
vi.mock("@/app/store/contextmenu", () => ({
    ContextMenuModel: {
        showContextMenu: vi.fn(),
    },
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn(() => () => {}),
}));

const modalLayerOpen = vi.fn();
const modalLayerReplace = vi.fn();
const modalLayerClose = vi.fn();
vi.mock("@/element/modal-layer", () => ({
    useModalLayer: () => ({
        open: modalLayerOpen,
        replace: modalLayerReplace,
        close: modalLayerClose,
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
            data-has-ctx-menu={String(typeof props.onContextMenu === "function")}
            onClick={(e) => props.onLaunch(props.agent, e)}
            onContextMenu={(e) => {
                props.onContextMenu?.(props.agent, e);
            }}
        >
            {props.agent.name}
        </button>
    ),
}));

vi.mock("./HiddenTemplatesSection", () => ({
    HiddenTemplatesSection: () => null,
}));

vi.mock("./AgentActionBar", () => ({
    AgentActionBar: () => null,
}));

vi.mock("./MyAgentsList", () => ({
    MyAgentsList: () => null,
}));

import { AgentPicker } from "./AgentPicker";
import { ContextMenuModel } from "@/app/store/contextmenu";

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;
const contextMenuShow = vi.mocked(ContextMenuModel.showContextMenu);

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
    modalLayerOpen.mockClear();
    modalLayerReplace.mockClear();
    modalLayerClose.mockClear();
    contextMenuShow.mockClear();
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
        await waitFor(() => expect(modalLayerOpen).toHaveBeenCalled());

        const req = modalLayerOpen.mock.calls[0][0];
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

    // Phase 2 (Q2 Decision Y — hide templates).
    it("right-click on a template card opens the Hide context menu", async () => {
        const model = makeMockModel();
        render(() => <AgentPicker model={model as any} />);
        const card = await screen.findByTestId("agent-card-tpl-claude");
        // The mock card forwards the onContextMenu down so the
        // picker sees the right-click and asks ContextMenuModel to
        // render the menu.
        expect(card.getAttribute("data-has-ctx-menu")).toBe("true");
        fireEvent.contextMenu(card);
        await waitFor(() => expect(contextMenuShow).toHaveBeenCalled());

        const [menu, evt] = contextMenuShow.mock.calls[0];
        expect(Array.isArray(menu)).toBe(true);
        expect(menu.length).toBe(1);
        expect(menu[0].label).toContain("Hide template");
        expect(menu[0].label).toContain("Claude Code");
        // Sanity: the event we forwarded carries the synthetic MouseEvent
        // shape ContextMenuModel.showContextMenu expects (clientX/Y reads
        // on the production CEF path).
        expect(evt).toBeTruthy();
    });

    it("Hide menu click fires AgentDefHideCommand with the template id", async () => {
        const model = makeMockModel();
        render(() => <AgentPicker model={model as any} />);
        const card = await screen.findByTestId("agent-card-tpl-claude");
        fireEvent.contextMenu(card);
        await waitFor(() => expect(contextMenuShow).toHaveBeenCalled());

        const [menu] = contextMenuShow.mock.calls[0];
        // Invoke the menu item's click handler directly — this is
        // what the CEF bridge does after the user picks the row.
        menu[0].click();
        await waitFor(() =>
            expect(RpcApi.AgentDefHideCommand).toHaveBeenCalledTimes(1),
        );
        const call = vi.mocked(RpcApi.AgentDefHideCommand).mock.calls[0];
        expect(call[1]).toEqual({ definition_id: "tpl-claude" });
    });

    it("templates tier never renders hidden templates (filter happens server-side)", async () => {
        const model = makeMockModel();
        // Backend filters out hidden rows by default — simulate by
        // returning only the visible template.
        vi.mocked(RpcApi.ListAgentDefinitionsCommand).mockResolvedValue([
            // Hidden template absent — server already excluded it.
            userAgent,
        ]);
        render(() => <AgentPicker model={model as any} />);
        await waitFor(() => {
            // No template card renders because no template was
            // returned by ListAgents.
            expect(screen.queryByTestId("agent-card-tpl-claude")).toBeNull();
        });
    });

    it("user-owned agents never get a Hide context menu (they go to MyAgentsList)", async () => {
        const model = makeMockModel();
        // Only user-owned in the templates tier shouldn't happen by
        // design — the picker partitions on is_seeded === 1. But
        // assert the negative case: only template cards expose the
        // ctx-menu handler. (User-owned rows are mocked out via
        // MyAgentsList.)
        render(() => <AgentPicker model={model as any} />);
        await waitFor(() => {
            expect(screen.queryByTestId("agent-card-tpl-claude")).not.toBeNull();
        });
        // user-maks isn't in the templates tier — its card never
        // renders here, so the contextMenu would have no surface to
        // attach to.
        expect(screen.queryByTestId("agent-card-user-maks")).toBeNull();
    });

    it("modal onCreatedAndLaunch fires launchAgentDefinition with the picked bindings", async () => {
        const model = makeMockModel();
        render(() => <AgentPicker model={model as any} />);
        const card = await screen.findByTestId("agent-card-tpl-claude");
        fireEvent.click(card);
        await waitFor(() => expect(modalLayerOpen).toHaveBeenCalled());

        const req = modalLayerOpen.mock.calls[0][0];
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
