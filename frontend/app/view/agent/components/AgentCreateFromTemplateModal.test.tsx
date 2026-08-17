// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for AgentCreateFromTemplateModalPanel
 * (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md — Phase 1).
 *
 * Covered:
 *  - default name field pre-fills with the template's name
 *  - Identity + Memory selects render the accounts / bundles list
 *    (empty option represents ambient creds / vanilla CLI sentinels)
 *  - clicking Create fires onSubmit with the form snapshot
 *  - the Create button is disabled while submitting
 *  - error from onSubmit surfaces in the panel body
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { sleep } from "@/util/util";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListMemoriesCommand: vi.fn(),
        // Mount-time daemon probe (drives the host/container dropdown).
        // Default → no reachable runtime → host-only.
        ContainerRuntimeAvailableCommand: vi.fn(),
        // Backs `resolveEffectiveLaunchProvider`'s bound-bundle resolution
        // (#2594) — resolves to `undefined` by default so the base
        // `template` fixture (no memory_id) never even triggers a fetch;
        // the drift regression tests below set their own
        // `.mockResolvedValue`.
        GetMemoryCommand: vi.fn().mockResolvedValue(undefined),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/view/identity/identity-model", () => ({
    refreshAccountCache: vi.fn(),
}));

import { AgentCreateFromTemplateModalPanel } from "./AgentCreateFromTemplateModal";
import { resetCapabilities } from "@/app/store/toolchain-capabilities";
import { refreshAccountCache } from "@/app/view/identity/identity-model";

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;

const template: AgentDefinition = {
    id: "tpl-claude",
    slug: "claude",
    name: "Claude Code",
    icon: "",
    provider: "claude",
    description: "Anthropic's coding agent",
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
} as AgentDefinition;

beforeEach(async () => {
    vi.clearAllMocks();
    // toolchain-capabilities is a module-level singleton shared across every
    // test in this file (and every real consumer in the app) — reset it so
    // one test's mocked ContainerRuntimeAvailableCommand result can't leak
    // into the next via the shared cache.
    resetCapabilities();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
    vi.mocked(refreshAccountCache).mockResolvedValue([
        { id: "id-work", name: "Work", provider: "claude" } as any,
    ]);
    vi.mocked(RpcApi.ListMemoriesCommand).mockResolvedValue([
        { id: "mem-notes", name: "Notes", is_blank: false } as any,
    ]);
    // Default: Docker daemon not reachable → host-only. Consumed via the
    // shared toolchain-capabilities store, not called directly by the
    // component anymore — see docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
    vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: false });
});

afterEach(() => cleanup());

const flush = () => sleep(0);

describe("AgentCreateFromTemplateModalPanel", () => {
    it("defaults the name field to the template name", async () => {
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={vi.fn().mockResolvedValue(undefined)}
                onCancel={vi.fn()}
            />
        ));
        const input = screen.getByTestId(
            "create-from-template-name-input",
        ) as HTMLInputElement;
        expect(input.value).toBe("Claude Code");
    });

    it("clicking Create fires onSubmit with form snapshot", async () => {
        const onSubmit = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        // Bundles auto-pick once loaded.
        await flush();
        await flush();
        await flush();

        const input = screen.getByTestId(
            "create-from-template-name-input",
        ) as HTMLInputElement;
        fireEvent.input(input, { target: { value: "Mary" } });

        const submit = screen.getByTestId("create-from-template-submit");
        fireEvent.click(submit);

        await waitFor(() => {
            expect(onSubmit).toHaveBeenCalledTimes(1);
        });
        const args = onSubmit.mock.calls[0][0];
        expect(args.name).toBe("Mary");
        // Identity auto-picked from the first available account for
        // the template's provider; Memory from the first non-blank
        // bundle.
        expect(args.accountId).toBe("id-work");
        expect(args.memoryId).toBe("mem-notes");
        // No Docker → runtime defaults to host (never a mode that
        // can't actually start).
        expect(args.agentType).toBe("host");
    });

    it("disables the container option when no Docker runtime is found", async () => {
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={vi.fn().mockResolvedValue(undefined)}
                onCancel={vi.fn()}
            />
        ));
        // Let the mount-time Docker probe settle (rejects in beforeEach).
        await flush();
        await flush();
        const select = screen.getByTestId(
            "create-from-template-runtime-select",
        ) as HTMLSelectElement;
        const containerOpt = Array.from(select.options).find(
            (o) => o.value === "container",
        )!;
        expect(containerOpt.disabled).toBe(true);
        expect(select.value).toBe("host");
    });

    it("defaults to container when Docker is available and the template suggests it", async () => {
        vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: true });
        const containerTemplate = { ...template, agent_type: "container" } as AgentDefinition;
        const onSubmit = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={containerTemplate}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        // Wait for the probe to resolve and the default-pick effect to run.
        const select = screen.getByTestId(
            "create-from-template-runtime-select",
        ) as HTMLSelectElement;
        await waitFor(() => expect(select.value).toBe("container"));
        const containerOpt = Array.from(select.options).find(
            (o) => o.value === "container",
        )!;
        expect(containerOpt.disabled).toBe(false);
    });

    it("surfaces an error from onSubmit", async () => {
        const onSubmit = vi.fn().mockRejectedValue(new Error("name exists"));
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        const submit = screen.getByTestId("create-from-template-submit");
        fireEvent.click(submit);
        const err = await screen.findByTestId("create-from-template-error");
        expect(err.textContent).toContain("name exists");
    });

    it("trims whitespace and rejects empty names", async () => {
        const onSubmit = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        const input = screen.getByTestId(
            "create-from-template-name-input",
        ) as HTMLInputElement;
        fireEvent.input(input, { target: { value: "   " } });
        const submit = screen.getByTestId(
            "create-from-template-submit",
        ) as HTMLButtonElement;
        expect(submit.disabled).toBe(true);
        fireEvent.click(submit);
        await flush();
        expect(onSubmit).not.toHaveBeenCalled();
    });

    it("shows the Model Vendor / Custom Endpoint field for a provider that declares baseUrlEnvVar (claude) and includes it in the submit payload", async () => {
        const onSubmit = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        await flush();
        await flush();
        await flush();
        const input = screen.getByTestId(
            "create-from-template-vendor-base-url-input",
        ) as HTMLInputElement;
        fireEvent.input(input, { target: { value: "https://my-proxy.example.com" } });

        const submit = screen.getByTestId("create-from-template-submit");
        fireEvent.click(submit);

        await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
        expect(onSubmit.mock.calls[0][0].modelVendorBaseUrl).toBe(
            "https://my-proxy.example.com",
        );
    });

    it("hides the Model Vendor / Custom Endpoint field for a provider that doesn't declare baseUrlEnvVar, and submits an empty override", async () => {
        const codexTemplate = { ...template, provider: "codex" } as AgentDefinition;
        const onSubmit = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={codexTemplate}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        await flush();
        await flush();
        await flush();
        expect(screen.queryByTestId("create-from-template-vendor-base-url-input")).toBeNull();

        const submit = screen.getByTestId("create-from-template-submit");
        fireEvent.click(submit);

        await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
        expect(onSubmit.mock.calls[0][0].modelVendorBaseUrl).toBe("");
    });

    // #2594 follow-up: harness-then-model creation flow. The template
    // card already picked the harness (claude); these cover the model
    // picker that lets the user choose WHICH model that harness runs.
    describe("model picker (harness-then-model creation flow)", () => {
        it("renders a model select defaulting to the harness's own default model", async () => {
            render(() => (
                <AgentCreateFromTemplateModalPanel
                    template={template}
                    onSubmit={vi.fn().mockResolvedValue(undefined)}
                    onCancel={vi.fn()}
                />
            ));
            const select = screen.getByTestId(
                "create-from-template-model-select",
            ) as HTMLSelectElement;
            // claude's catalog entry marks "sonnet" as `default: true`.
            expect(select.value).toBe("sonnet");
            expect(Array.from(select.options).map((o) => o.value)).toContain("opus");
        });

        it("shows the harness-vs-model explanatory hint for a harness with a models list", async () => {
            render(() => (
                <AgentCreateFromTemplateModalPanel
                    template={template}
                    onSubmit={vi.fn().mockResolvedValue(undefined)}
                    onCancel={vi.fn()}
                />
            ));
            expect(screen.getByText(/is the/)).toHaveTextContent("harness");
        });

        it("sends the user's picked model in onSubmit's form data", async () => {
            const onSubmit = vi.fn().mockResolvedValue(undefined);
            render(() => (
                <AgentCreateFromTemplateModalPanel
                    template={template}
                    onSubmit={onSubmit}
                    onCancel={vi.fn()}
                />
            ));
            await flush();
            await flush();
            await flush();

            const select = screen.getByTestId(
                "create-from-template-model-select",
            ) as HTMLSelectElement;
            fireEvent.change(select, { target: { value: "opus" } });

            const submit = screen.getByTestId("create-from-template-submit");
            fireEvent.click(submit);

            await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
            expect(onSubmit.mock.calls[0][0].model).toBe("opus");
        });

        it("hides the model select, the hint, and sends an empty model for a harness with no models list", async () => {
            const noModelsTemplate = {
                ...template,
                provider: "not-a-real-provider",
            } as AgentDefinition;
            const onSubmit = vi.fn().mockResolvedValue(undefined);
            render(() => (
                <AgentCreateFromTemplateModalPanel
                    template={noModelsTemplate}
                    onSubmit={onSubmit}
                    onCancel={vi.fn()}
                />
            ));
            await flush();
            await flush();
            await flush();

            expect(screen.queryByTestId("create-from-template-model-select")).toBeNull();
            expect(screen.queryByText(/is the/)).toBeNull();

            const submit = screen.getByTestId("create-from-template-submit");
            fireEvent.click(submit);

            await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
            expect(onSubmit.mock.calls[0][0].model).toBe("");
        });
    });

    // ReAgent P1 on PR #2618 (round 2): every provider-dependent decision
    // in this modal (model list, account filter, container support,
    // vendor-endpoint field) must resolve through the template's BOUND
    // BUNDLE, not its possibly-drifted `.provider` column directly —
    // `agentdefcreatefromtemplate` itself already resolves the clone's
    // provider this way server-side (template.rs, #2607).
    describe("resolves through the template's bound bundle, not a drifted provider column (ReAgent P1 on #2618)", () => {
        it("shows the resolved (bundle) provider's models, not the drifted column's", async () => {
            const drifted = { ...template, provider: "codex", memory_id: "mem-1" } as AgentDefinition;
            vi.mocked(RpcApi.GetMemoryCommand).mockResolvedValue({ provider: "claude" } as any);

            render(() => (
                <AgentCreateFromTemplateModalPanel
                    template={drifted}
                    onSubmit={vi.fn().mockResolvedValue(undefined)}
                    onCancel={vi.fn()}
                />
            ));
            await flush();
            await flush();
            await flush();

            const select = (await screen.findByTestId(
                "create-from-template-model-select",
            )) as HTMLSelectElement;
            const options = Array.from(select.options).map((o) => o.value);
            // claude's models, not codex's (gpt-5.x family).
            expect(options).toContain("opus");
            expect(options).not.toContain("gpt-5.5");
        });

        it("filters accounts by the resolved (bundle) provider, not the drifted column's", async () => {
            const drifted = { ...template, provider: "codex", memory_id: "mem-1" } as AgentDefinition;
            vi.mocked(RpcApi.GetMemoryCommand).mockResolvedValue({ provider: "claude" } as any);
            vi.mocked(refreshAccountCache).mockResolvedValue([
                { id: "acct-claude", name: "Claude Work", provider: "claude" } as any,
                { id: "acct-codex", name: "Codex Work", provider: "codex" } as any,
            ]);

            render(() => (
                <AgentCreateFromTemplateModalPanel
                    template={drifted}
                    onSubmit={vi.fn().mockResolvedValue(undefined)}
                    onCancel={vi.fn()}
                />
            ));
            await flush();
            await flush();
            await flush();

            const select = screen.getByTestId(
                "create-from-template-identity-select",
            ) as HTMLSelectElement;
            const optionIds = Array.from(select.options).map((o) => o.value);
            expect(optionIds).toContain("acct-claude");
            expect(optionIds).not.toContain("acct-codex");
        });

        it("does not auto-pick an account filtered against the stale fallback provider before the bundle resolves", async () => {
            // Deliberately let accounts resolve BEFORE the bundle so the
            // auto-pick effect gets a chance to fire against the stale
            // fallback provider ("claude") first — this is what makes
            // the race actually reproducible.
            const drifted = { ...template, provider: "claude", memory_id: "mem-1" } as AgentDefinition;
            vi.mocked(refreshAccountCache).mockResolvedValue([
                { id: "acct-claude", name: "Claude Work", provider: "claude" } as any,
                { id: "acct-codex", name: "Codex Work", provider: "codex" } as any,
            ]);
            let resolveBundle!: (v: any) => void;
            vi.mocked(RpcApi.GetMemoryCommand).mockReturnValue(
                new Promise((resolve) => {
                    resolveBundle = resolve;
                }),
            );

            render(() => (
                <AgentCreateFromTemplateModalPanel
                    template={drifted}
                    onSubmit={vi.fn().mockResolvedValue(undefined)}
                    onCancel={vi.fn()}
                />
            ));
            // Let the already-resolved accounts fetch fully settle,
            // including the auto-pick effect's own commit, before the
            // bundle resolves.
            await flush();
            await flush();

            resolveBundle({ provider: "codex" });
            await flush();
            await flush();
            await flush();

            const select = screen.getByTestId(
                "create-from-template-identity-select",
            ) as HTMLSelectElement;
            expect(select.value).toBe("acct-codex");
        });
    });

    // ReAgent P2 on PR #2618: a provider can declare a `models` list in
    // the catalog without buildRuntimeArgs.ts actually wiring `--model`
    // for it (antigravity: providers/catalog.ts, 4-entry models list, no
    // --model branch) — offering a picker there would let the user pick
    // a model that's silently discarded at launch.
    it("hides the model picker for a provider whose models list has no --model wiring at launch (antigravity)", async () => {
        const antigravityTemplate = { ...template, provider: "antigravity" } as AgentDefinition;
        const onSubmit = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={antigravityTemplate}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        await flush();
        await flush();
        await flush();

        expect(screen.queryByTestId("create-from-template-model-select")).toBeNull();

        const submit = screen.getByTestId("create-from-template-submit");
        fireEvent.click(submit);
        await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
        expect(onSubmit.mock.calls[0][0].model).toBe("");
    });

    // ReAgent P1 on PR #2618: the model list must read through
    // getProvider() (the live, API-sourced-overlay-aware accessor), not
    // the raw static PROVIDERS catalog — otherwise a refreshed label
    // (setProviderModels, folded in at app-init from the authoritative
    // Models API) never reaches this modal. Exercises the REAL overlay
    // mechanism (model-overlay.ts) rather than mocking it, since
    // `modelOverlay` is module-level signal state shared across this
    // whole test file — this must be the LAST test in the file so the
    // overlay it sets can't leak into any test that runs after it.
    // Preserves "sonnet"'s `value`/`default` (only the `label` changes),
    // matching setProviderModels' own documented "label-only refresh"
    // contract, so this can't itself corrupt any earlier test's
    // `select.value` assertions either.
    it("shows a live-overlaid model label, not the stale static catalog label (ReAgent P1 on #2618)", async () => {
        const { setProviderModels } = await import("../providers");
        setProviderModels("claude", [
            { value: "claude-sonnet-5-5", label: "Claude Sonnet 5.5" },
        ]);

        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={vi.fn().mockResolvedValue(undefined)}
                onCancel={vi.fn()}
            />
        ));
        const select = screen.getByTestId(
            "create-from-template-model-select",
        ) as HTMLSelectElement;
        const sonnetOption = Array.from(select.options).find((o) => o.value === "sonnet")!;
        expect(sonnetOption.textContent).toBe("Sonnet 5.5");
        // value/default untouched by the label refresh.
        expect(select.value).toBe("sonnet");
    });
});
