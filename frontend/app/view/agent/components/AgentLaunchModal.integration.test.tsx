// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Integration tests for AgentLaunchModal.
 *
 * Pins the "memory change → forgot login" regression at the
 * component level (the §6.10 acceptance criterion of
 * docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md).
 *
 * Approach + library choice: see
 * docs/specs/SPEC_LAUNCH_MODAL_INTEGRATION_TESTS_2026_05_19.md.
 *
 * Mocks RPC + IPC at the module boundary; SUT is the real
 * AgentLaunchModal + the real launch-flow-state slice + the real
 * AuthFlowController.
 */

import { render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentLaunchModalPanel } from "./AgentLaunchModal";

// ── Module mocks ────────────────────────────────────────────────────

vi.mock("@/app/store/rpc-api", () => {
    const RpcApi = {
        ListIdentityBundlesCommand: vi.fn(),
        ListMemoriesCommand: vi.fn(),
        ListIdentityBindingsCommand: vi.fn(),
        ListNamedAgentsCommand: vi.fn(),
    };
    return { RpcApi };
});

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn(() => () => {}),
}));

vi.mock("@/app/store/global", () => ({
    getApi: () => ({ openExternal: vi.fn() }),
}));

vi.mock("@/util/clipboard", () => ({
    writeText: vi.fn(),
}));

vi.mock("@/app/errors/translate", () => ({
    translateError: (e: unknown) => String(e),
}));

// Stub the providers module so the agent's provider entry exists.
vi.mock("../providers", () => ({
    getProvider: (id: string) =>
        id === "claude"
            ? {
                  id: "claude",
                  authType: "oauth",
                  authCommand: "claude",
                  authCheckArgs: [],
                  npmPackage: null,
                  cliCommand: "claude",
              }
            : undefined,
}));

// Stub the cli-catalog so displayName resolves.
vi.mock("../defaults/cli-catalog", () => ({
    getCliCatalogEntry: () => ({ displayName: "Claude Code", popoverMarkdown: "" }),
}));

// instance-slug — used to derive launch-form invariants.
vi.mock("../defaults/instance-slug", () => ({
    buildInstanceSlug: (name: string) => name.toLowerCase().replace(/\s+/g, "-"),
    slugifyInstanceName: (name: string) => name.trim().toLowerCase().replace(/\s+/g, "-"),
}));

// ── Fixtures ────────────────────────────────────────────────────────

const ts = () => 1_700_000_000_000;

const claudeAgent = {
    id: "agent-claude",
    slug: "claude-code",
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

const workIdentity: IdentityBundle = {
    id: "ident-work",
    name: "Work",
    is_blank: false,
    created_at: ts(),
    updated_at: ts(),
};

const notesMemory: Memory = {
    id: "mem-notes",
    name: "Notes",
    is_blank: false,
    created_at: ts(),
    updated_at: ts(),
};

const personalMemory: Memory = {
    id: "mem-personal",
    name: "Personal",
    is_blank: false,
    created_at: ts(),
    updated_at: ts(),
};

const claudeBinding: IdentityBinding = {
    identity_id: "ident-work",
    provider: "claude",
    account_id: "acc-1",
};

// Pull the mocked RpcApi after vi.mock has run.
let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;

beforeEach(async () => {
    vi.clearAllMocks();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
    vi.mocked(RpcApi.ListIdentityBundlesCommand).mockResolvedValue([workIdentity]);
    vi.mocked(RpcApi.ListMemoriesCommand).mockResolvedValue([notesMemory, personalMemory]);
    vi.mocked(RpcApi.ListIdentityBindingsCommand).mockResolvedValue([claudeBinding]);
    vi.mocked(RpcApi.ListNamedAgentsCommand).mockResolvedValue([]);
});

afterEach(() => {
    vi.restoreAllMocks();
});

// ── Tests ───────────────────────────────────────────────────────────

describe("AgentLaunchModal — memory change must not reset auth state (§6.10)", () => {
    // The §6.10 requirement holds — changing the preset does NOT reset auth
    // (asserted below). This test had gone stale on the "Memory bundle" → "Preset"
    // rename (the select's aria-label is now "Preset"); the stale selector read as
    // an auth "regression" when CI first ran it. Selector fixed; logic unchanged.
    it("preserves auth-ready state across a Memory selection change", async () => {
        const user = userEvent.setup();
        const onSubmit = vi.fn();
        const onCancel = vi.fn();
        render(() => (
            <AgentLaunchModalPanel
                agent={claudeAgent}
                onSubmit={onSubmit}
                onCancel={onCancel}
            />
        ));

        // Wait for identities + memories + bindings to settle. The
        // auto-pick effect runs after IdentitiesLoaded fires, then
        // the FetchBindings event sink runs and dispatches
        // BindingsLoaded.  The Identity selector renders once
        // identities are loaded.
        const identitySelect = await screen.findByLabelText("Identity bundle");
        const memorySelect = await screen.findByLabelText("Preset");

        // Verify the auto-pick wired the bound identity. With Work
        // bundle bound to claude, the Connect panel must NOT mount.
        expect((identitySelect as HTMLSelectElement).value).toBe("ident-work");

        // Type a valid agent name.
        await user.type(screen.getByLabelText("Agent name"), "alpha");

        // Auth should already be "ready" because hasMatchingBinding
        // for claude on Work is true. The Connect button is the
        // tell — it appears inside PreLaunchAuthPanel when
        // authRequired() is true.  Asserting its absence is the
        // strongest stable signal that the panel didn't mount.
        expect(
            screen.queryByRole("button", { name: /connect/i }),
        ).not.toBeInTheDocument();

        // THE REGRESSION: change Memory selection.
        await user.selectOptions(memorySelect, "mem-personal");

        // Connect button still absent. Auth state survives.
        expect(
            screen.queryByRole("button", { name: /connect/i }),
        ).not.toBeInTheDocument();
        expect((identitySelect as HTMLSelectElement).value).toBe("ident-work");
        expect((memorySelect as HTMLSelectElement).value).toBe("mem-personal");
    });
});
