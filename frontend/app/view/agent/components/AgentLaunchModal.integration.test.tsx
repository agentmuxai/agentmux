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

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentLaunchModalPanel } from "./AgentLaunchModal";
import { resetCapabilities } from "@/app/store/toolchain-capabilities";

// ── Module mocks ────────────────────────────────────────────────────

vi.mock("@/app/store/rpc-api", () => {
    const RpcApi = {
        ListMemoriesCommand: vi.fn(),
        ListNamedAgentsCommand: vi.fn(),
        // Backs the shared toolchain-capabilities store's Docker liveness
        // probe — this modal polls it (watchCapability("docker")) for the
        // non-blocking sandbox hint. An unconfigured vi.fn() resolves to
        // undefined, which the store treats as "unavailable"; fine as a
        // default for tests that don't care about Docker state.
        ContainerRuntimeAvailableCommand: vi.fn(),
        // Backs `effectiveProviderId`'s bound-bundle resolution (PR
        // following #2592/#2594) — resolves to `undefined` by default so
        // existing tests (none of whose agent fixtures set `memory_id`)
        // never even trigger the fetch; tests that DO exercise the
        // resolution set their own `mockResolvedValue`.
        GetMemoryCommand: vi.fn().mockResolvedValue(undefined),
    };
    return { RpcApi };
});

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

// Issue #1624 PR-C Part B — the launch modal sources accounts from the
// shared account cache instead of `ListIdentityBundlesCommand`/
// `ListIdentityBindingsCommand`.
vi.mock("@/app/view/identity/identity-model", () => ({
    refreshAccountCache: vi.fn(),
    subscribeAccountChanges: vi.fn(() => () => {}),
}));

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
    getProvider: (id: string) => {
        if (id === "claude") {
            return {
                id: "claude",
                authType: "oauth",
                authCommand: "claude",
                authCheckArgs: [],
                npmPackage: null,
                cliCommand: "claude",
            };
        }
        // Recognized (unlike "codex" below, which stays undefined) —
        // needed for the auto-pick race regression test, where the
        // STALE provider must itself be valid (with its own account)
        // for the bug to manifest at all.
        if (id === "gemini") {
            return {
                id: "gemini",
                authType: "oauth",
                authCommand: "gemini",
                authCheckArgs: [],
                npmPackage: null,
                cliCommand: "gemini",
            };
        }
        return undefined;
    },
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

// The same drift scenario PR #2592 fixed on the backend: the agent's own
// `provider` column says "codex" (unrecognized by this file's
// `getProvider` mock, which only knows "claude"), but it's bound to a
// bundle whose provider correctly says "claude".
const driftedProviderAgent = {
    ...claudeAgent,
    id: "agent-drift",
    provider: "codex",
    memory_id: "mem-bundle-1",
} as AgentDefinition;

const driftedAgentsBundle: Memory = {
    id: "mem-bundle-1",
    name: "Drift Test Bundle",
    is_blank: false,
    provider: "claude",
    created_at: ts(),
    updated_at: ts(),
};

// The auto-pick race scenario from the round-2 review: the stale
// `agent.provider` value must itself be a RECOGNIZED provider with its
// own account for the bug to manifest — "codex" above is unrecognized,
// so it never reaches this code path at all.
const raceDriftAgent = {
    ...claudeAgent,
    id: "agent-race",
    provider: "gemini",
    memory_id: "mem-bundle-race",
} as AgentDefinition;

const raceDriftBundle: Memory = {
    id: "mem-bundle-race",
    name: "Race Test Bundle",
    is_blank: false,
    provider: "claude",
    created_at: ts(),
    updated_at: ts(),
};

const workAccount = {
    id: "acct-work",
    name: "Work",
    provider: "claude",
    kind: "oauth",
    secret_ref: { backend: "env" },
    context: {},
    assigned_agents: [],
    status: "valid",
    created_at: "",
    updated_at: "",
} as unknown as import("@/app/view/identity/identity-model").Account;

const geminiAccount = {
    id: "acct-gemini",
    name: "Gemini",
    provider: "gemini",
    kind: "oauth",
    secret_ref: { backend: "env" },
    context: {},
    assigned_agents: [],
    status: "valid",
    created_at: "",
    updated_at: "",
} as unknown as import("@/app/view/identity/identity-model").Account;

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

// Pull the mocked RpcApi + identity-model after vi.mock has run.
let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;
let refreshAccountCache: typeof import("@/app/view/identity/identity-model").refreshAccountCache;

beforeEach(async () => {
    vi.clearAllMocks();
    resetCapabilities();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
    ({ refreshAccountCache } = await import("@/app/view/identity/identity-model"));
    vi.mocked(refreshAccountCache).mockResolvedValue([workAccount]);
    vi.mocked(RpcApi.ListMemoriesCommand).mockResolvedValue([notesMemory, personalMemory]);
    vi.mocked(RpcApi.ListNamedAgentsCommand).mockResolvedValue([]);
});

afterEach(() => {
    // This file only ever had one test before; adding more exposed that
    // it never unmounted between runs (no global afterEach(cleanup) in
    // test/vitest-setup.ts, unlike files that call this explicitly).
    // Without it, a still-live component from a prior test's render
    // (including its still-reactive createResource calls) can pollute
    // the next test's DOM queries.
    cleanup();
    vi.restoreAllMocks();
});

// ── Tests ───────────────────────────────────────────────────────────

describe("AgentLaunchModal — memory change must not reset auth state (§6.10)", () => {
    // The §6.10 requirement holds — changing the Bundle does NOT reset
    // auth (asserted below).
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

        // Wait for accounts + memories to settle. The auto-pick effect
        // runs after AccountsLoaded fires. The Account selector renders
        // once accounts for the agent's provider are loaded.
        const identitySelect = await screen.findByLabelText("Account");
        const memorySelect = await screen.findByLabelText("Bundle");

        // Verify the auto-pick wired the Work account. It supplies
        // claude creds, so the Connect panel must NOT mount.
        expect((identitySelect as HTMLSelectElement).value).toBe("acct-work");

        // Type a valid agent name.
        await user.type(screen.getByLabelText("Agent name"), "alpha");

        // Auth should already be "ready" because the Work account
        // supplies claude creds. The Connect button is the tell — it
        // appears inside PreLaunchAuthPanel when authRequired() is
        // true. Asserting its absence is the strongest stable signal
        // that the panel didn't mount.
        expect(
            screen.queryByRole("button", { name: /connect/i }),
        ).not.toBeInTheDocument();

        // THE REGRESSION: change Memory selection.
        await user.selectOptions(memorySelect, "mem-personal");

        // Connect button still absent. Auth state survives.
        expect(
            screen.queryByRole("button", { name: /connect/i }),
        ).not.toBeInTheDocument();
        expect((identitySelect as HTMLSelectElement).value).toBe("acct-work");
        expect((memorySelect as HTMLSelectElement).value).toBe("mem-personal");
    });
});

describe("AgentLaunchModal — provider resolution through the bound bundle", () => {
    // The core regression case: if this modal resolved the drifted
    // "codex" column instead of the bundle's "claude", accountsForProvider
    // would find no matching account for "codex" (workAccount is
    // claude-provider) and the Account selector would never auto-pick
    // acct-work — offering the wrong provider's auth flow entirely (or
    // none at all) for a perfectly valid, correctly-configured agent.
    it("resolves the effective provider through the bound bundle, not a drifted agent.provider", async () => {
        vi.mocked(RpcApi.GetMemoryCommand).mockResolvedValue(driftedAgentsBundle);

        render(() => (
            <AgentLaunchModalPanel
                agent={driftedProviderAgent}
                onSubmit={vi.fn()}
                onCancel={vi.fn()}
            />
        ));

        const identitySelect = await screen.findByLabelText("Account");
        expect((identitySelect as HTMLSelectElement).value).toBe("acct-work");
        expect(RpcApi.GetMemoryCommand).toHaveBeenCalledWith({}, { id: "mem-bundle-1" });
    });

    it("falls back to agent.provider when the agent has no bound bundle", async () => {
        render(() => (
            <AgentLaunchModalPanel
                agent={claudeAgent}
                onSubmit={vi.fn()}
                onCancel={vi.fn()}
            />
        ));

        const identitySelect = await screen.findByLabelText("Account");
        expect((identitySelect as HTMLSelectElement).value).toBe("acct-work");
        expect(RpcApi.GetMemoryCommand).not.toHaveBeenCalled();
    });

    // Round-2 review finding on PR #2596: the account auto-pick effect
    // only fires while `accountId()` is empty, and once it commits it
    // never re-evaluates (Solid drops `provider()` from the effect's
    // tracked deps the moment it takes the early-return path). If the
    // modal exposed the STALE `agent.provider` while the bundle fetch was
    // still in flight, and that stale value happened to be a recognized
    // provider with its own account, the effect could commit to the WRONG
    // account before the bundle resolves — and then never self-correct.
    it("does not auto-pick the wrong account for a recognized-but-stale provider before the bundle resolves", async () => {
        // Deliberately let accounts resolve BEFORE the bundle so the
        // auto-pick effect gets a chance to fire against the stale
        // `agent.provider` ("gemini") first — this is what makes the race
        // actually reproducible instead of both fetches settling in the
        // same microtask batch.
        vi.mocked(refreshAccountCache).mockResolvedValue([workAccount, geminiAccount]);
        let resolveBundle!: (m: Memory) => void;
        vi.mocked(RpcApi.GetMemoryCommand).mockReturnValue(
            new Promise<Memory>((resolve) => {
                resolveBundle = resolve;
            }),
        );
        const onSubmit = vi.fn();

        render(() => (
            <AgentLaunchModalPanel
                agent={raceDriftAgent}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));

        // Let the already-resolved accounts fetch fully settle —
        // including the auto-pick effect's own commit — before the
        // bundle resolves. This is the race window the round-2 review
        // flagged: under the bug, the auto-pick effect commits to the
        // still-stale `agent.provider` ("gemini") here and never gets a
        // chance to reconsider once the bundle resolves. A macrotask
        // flush (not just a microtask) is needed since the account
        // fetch's promise chain + Solid's own effect scheduling both run
        // across several microtask ticks.
        await new Promise((r) => setTimeout(r, 0));

        resolveBundle(raceDriftBundle);
        await screen.findByLabelText("Account");

        // Ground-truth check: read the actual committed `accountId`
        // signal, not the rendered `<select>.value` — once the resolved
        // provider narrows the option list to a single entry, a stale
        // selected id that's no longer a listed option gets implicitly
        // reselected by the DOM itself (jsdom/browser `<select>`
        // behavior), which makes the widget's displayed value look
        // "self-corrected" even though the underlying signal driving
        // submission never changed. `useLaunchAuthGate`'s
        // `accountSupplies` reads that real signal against the current
        // provider — under the bug it stays false (stale "acct-gemini"
        // doesn't supply "claude"), so the Connect CTA stays mounted and
        // Launch stays blocked despite a valid, correct account existing.
        const user = userEvent.setup();
        await user.type(screen.getByLabelText("Agent name"), "race-test");

        await waitFor(() => {
            expect(
                screen.queryByRole("button", { name: /connect/i }),
            ).not.toBeInTheDocument();
        });
        const launchButton = screen.getByRole("button", { name: /^launch$/i });
        expect(launchButton).not.toBeDisabled();

        await user.click(launchButton);
        expect(onSubmit).toHaveBeenCalledWith(
            expect.objectContaining({ accountId: "acct-work" }),
        );
    });
});
