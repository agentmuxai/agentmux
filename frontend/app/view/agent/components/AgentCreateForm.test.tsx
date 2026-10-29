// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for AgentCreateForm, the Create step of the new-agent panel
 * (SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md §2.3).
 *
 * Covered:
 *  - the name field opens empty, with the same ghost text for every provider
 *  - Create clones the template with `agentdefcreatefromtemplate` and hands
 *    the response to onCreated
 *  - a duplicate-name refusal is shown in words; any other error as is
 *  - there's no account field: the provider's first account is sent
 *    (SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md §3.6)
 *  - the Bundles field starts empty and sends the picks in order
 *  - the Create button is disabled while creating, and until the accounts
 *    and the provider have loaded
 *  - onBusyChange follows the create
 *  - the draft keeps what was typed across an unmount and remount
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { sleep } from "@/util/util";

// Any mock, so a test can pass its own vi.fn() for a typed callback.
type AnyMock = Mock<(...args: any[]) => any>;

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentDefCreateFromTemplateCommand: vi.fn(),
        ListBundlesCommand: vi.fn(),
        // Mount-time daemon probe (drives the host/container dropdown).
        // Default → no reachable runtime → host-only.
        ContainerRuntimeAvailableCommand: vi.fn(),
        // Whether the container's image can be had; decides the default runtime.
        ContainerImageCheckCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/view/identity/identity-model", () => ({
    refreshAccountCache: vi.fn(),
}));
// The form resolves the provider that actually gets cloned through the
// launch's own resolver (#2594); by default that is the template's own.
vi.mock("../agent-launch-env", () => ({
    resolveEffectiveLaunchProvider: vi.fn(async (t: { provider: string }) => t.provider),
}));

import { AgentCreateForm, createCreateDraft, type CreateDraft } from "./AgentCreateForm";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
import { resetCapabilities } from "@/app/store/toolchain-capabilities";
import { refreshAccountCache } from "@/app/view/identity/identity-model";
import { RpcApi } from "@/app/store/rpc-api";
import type { AgentDefinition } from "@/app/store/rpc-api";

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

const createResponse = { definition_id: "def-1", identity_id: "id-work", memory_id: "", slug: "mary" };

beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    // toolchain-capabilities is a module-level singleton shared across every
    // test in this file (and every real consumer in the app) — reset it so
    // one test's mocked ContainerRuntimeAvailableCommand result can't leak
    // into the next via the shared cache.
    resetCapabilities();
    vi.mocked(resolveEffectiveLaunchProvider).mockImplementation(async (t) => t.provider);
    vi.mocked(refreshAccountCache).mockResolvedValue([{ id: "id-work", name: "Work", provider: "claude" } as any]);
    vi.mocked(RpcApi.ListBundlesCommand).mockResolvedValue([{ id: "mem-notes", name: "Notes", is_blank: false } as any]);
    // Default: Docker daemon not reachable → host-only. Consumed via the
    // shared toolchain-capabilities store, not called directly by the
    // component — see docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
    vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: false });
    vi.mocked(RpcApi.ContainerImageCheckCommand).mockResolvedValue({ status: "public" });
    vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockResolvedValue(createResponse as any);
});

afterEach(() => cleanup());

const flush = () => sleep(0);
const settle = async () => {
    await flush();
    await flush();
    await flush();
};

interface RenderOpts {
    tpl?: AgentDefinition;
    draft?: CreateDraft;
    /** Typed into the draft before mounting. */
    name?: string;
    onCreated?: AnyMock;
    onCancel?: AnyMock;
    onBusyChange?: AnyMock;
}

function renderForm(opts: RenderOpts = {}) {
    const tpl = opts.tpl ?? template;
    const draft = opts.draft ?? createCreateDraft(tpl);
    if (opts.name != null) draft.setName(opts.name);
    const onCreated: AnyMock = opts.onCreated ?? vi.fn().mockResolvedValue(undefined);
    const onCancel: AnyMock = opts.onCancel ?? vi.fn();
    const onBusyChange: AnyMock = opts.onBusyChange ?? vi.fn();
    const result = render(() => (
        <AgentCreateForm template={tpl} draft={draft} onCreated={onCreated} onCancel={onCancel} onBusyChange={onBusyChange} />
    ));
    return { ...result, draft, onCreated, onCancel, onBusyChange };
}

const nameInput = () => screen.getByTestId("create-from-template-name-input") as HTMLInputElement;
const submitButton = () => screen.getByTestId("create-from-template-submit") as HTMLButtonElement;

/** Clicks Create and returns what was sent to `agentdefcreatefromtemplate`. */
async function submitAndGetPayload() {
    fireEvent.click(submitButton());
    await waitFor(() => expect(RpcApi.AgentDefCreateFromTemplateCommand).toHaveBeenCalledTimes(1));
    return vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mock.calls[0][1];
}

/** Types a name, clicks Create, and returns the account that was sent. */
async function submittedAccount(): Promise<string> {
    fireEvent.input(nameInput(), { target: { value: "Mary" } });
    return (await submitAndGetPayload()).identity_id;
}

describe("AgentCreateForm", () => {
    // SPEC_CREATE_AGENT_NAME_NO_PREFILL_2026_10_06.md: the field used to open
    // holding the template's name, so agents piled up as Claude, Claude2, ….
    // It opens empty for every provider, with the same ghost text.
    it.each([
        ["Claude Code", "claude"],
        ["Codex", "codex"],
    ])("opens the name field empty with ghost text for %s", (name, provider) => {
        renderForm({ tpl: { ...template, name, provider } as AgentDefinition });
        expect(nameInput().value).toBe("");
        expect(nameInput().placeholder).toBe("Choose a name");
        expect(submitButton().disabled).toBe(true);
        expect(submitButton().textContent).toContain("Create and start");
    });

    it("enables Create once a name is typed", async () => {
        renderForm();
        // Create also waits for the accounts and provider to load.
        await flush();
        fireEvent.input(nameInput(), { target: { value: "Reviewer" } });
        expect(submitButton().disabled).toBe(false);
    });

    it("Enter does not submit an empty name", async () => {
        renderForm();
        await settle();
        fireEvent.keyDown(nameInput(), { key: "Enter" });
        await flush();
        expect(RpcApi.AgentDefCreateFromTemplateCommand).not.toHaveBeenCalled();
    });

    it("Enter in the name field submits once the form is valid", async () => {
        const { onCreated } = renderForm();
        await settle();
        fireEvent.input(nameInput(), { target: { value: "Mary" } });
        fireEvent.keyDown(nameInput(), { key: "Enter" });
        await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    });

    it("Shift+Enter does not submit", async () => {
        renderForm({ name: "Mary" });
        await settle();
        fireEvent.keyDown(nameInput(), { key: "Enter", shiftKey: true });
        await flush();
        expect(RpcApi.AgentDefCreateFromTemplateCommand).not.toHaveBeenCalled();
    });

    it("Create clones the template and hands the new agent to onCreated", async () => {
        const { onCreated } = renderForm();
        // Lists load.
        await settle();
        fireEvent.input(nameInput(), { target: { value: "  Mary  " } });

        const payload = await submitAndGetPayload();
        expect(payload).toEqual({
            template_id: "tpl-claude",
            // Trimmed.
            name: "Mary",
            // The first account for the template's provider.
            identity_id: "id-work",
            // No bundle is picked for the user.
            memory_id: "",
            bundle_ids: [],
            // No Docker → runtime defaults to host (never a mode that can't
            // actually start).
            agent_type: "host",
            model_vendor_base_url: "",
        });

        await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
        expect(onCreated).toHaveBeenCalledWith({
            definitionId: "def-1",
            identityId: "id-work",
            memoryId: "",
            name: "Mary",
            agentType: "host",
            // claude's catalog default.
            model: "sonnet",
            slug: "mary",
        });
    });

    it("Cancel calls onCancel", async () => {
        const { onCancel } = renderForm();
        fireEvent.click(screen.getByText("Cancel"));
        expect(onCancel).toHaveBeenCalledTimes(1);
    });

    it("disables the container option when no Docker runtime is found", async () => {
        renderForm();
        // Let the mount-time Docker probe settle.
        await flush();
        await flush();
        const select = screen.getByTestId("create-from-template-runtime-select") as HTMLSelectElement;
        const containerOpt = Array.from(select.options).find((o) => o.value === "container")!;
        expect(containerOpt.disabled).toBe(true);
        expect(select.value).toBe("host");
    });

    it("defaults to container when Docker is available and the template suggests it", async () => {
        vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: true });
        renderForm({ tpl: { ...template, agent_type: "container" } as AgentDefinition, name: "Mary" });
        // Wait for the probe to resolve and the default-pick effect to run.
        const select = screen.getByTestId("create-from-template-runtime-select") as HTMLSelectElement;
        await waitFor(() => expect(select.value).toBe("container"));
        const containerOpt = Array.from(select.options).find((o) => o.value === "container")!;
        expect(containerOpt.disabled).toBe(false);
        await settle();
        expect((await submitAndGetPayload()).agent_type).toBe("container");
    });

    describe("the remembered runtime", () => {
        const containerTemplate = { ...template, agent_type: "container" } as AgentDefinition;

        it("remembers an explicit Native over the template's container default", async () => {
            vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: true });
            renderForm({ tpl: containerTemplate });
            const select = screen.getByTestId("create-from-template-runtime-select") as HTMLSelectElement;
            await waitFor(() => expect(select.value).toBe("container"));
            fireEvent.change(select, { target: { value: "host" } });
            cleanup();
            renderForm({ tpl: containerTemplate });
            const next = screen.getByTestId("create-from-template-runtime-select") as HTMLSelectElement;
            await waitFor(() => expect(RpcApi.ContainerImageCheckCommand).toHaveBeenCalled());
            await settle();
            expect(next.value).toBe("host");
        });

        it("remembers an explicit Container for a template that suggests Native", async () => {
            vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: true });
            renderForm({ tpl: template });
            const select = screen.getByTestId("create-from-template-runtime-select") as HTMLSelectElement;
            await waitFor(() => expect(Array.from(select.options).find((o) => o.value === "container")!.disabled).toBe(false));
            fireEvent.change(select, { target: { value: "container" } });
            cleanup();
            renderForm({ tpl: template });
            const next = screen.getByTestId("create-from-template-runtime-select") as HTMLSelectElement;
            await waitFor(() => expect(next.value).toBe("container"));
        });

        it("falls back to Native when the remembered Container can't run", async () => {
            localStorage.setItem("agentCreate:lastRuntime", "container");
            renderForm({ tpl: containerTemplate });
            const select = screen.getByTestId("create-from-template-runtime-select") as HTMLSelectElement;
            await settle();
            expect(select.value).toBe("host");
        });

        it("does not remember a default it picked itself", async () => {
            vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: true });
            renderForm({ tpl: containerTemplate });
            const select = screen.getByTestId("create-from-template-runtime-select") as HTMLSelectElement;
            await waitFor(() => expect(select.value).toBe("container"));
            expect(localStorage.getItem("agentCreate:lastRuntime")).toBeNull();
        });

        it("explains Container and Native in the runtime row", () => {
            renderForm({ tpl: template });
            expect(screen.getByTestId("create-from-template-runtime-note").textContent).toBe(
                "Container: runs in Docker, sandboxed. Native: runs on this computer with your files and tools.",
            );
        });
    });

    describe("when the container image can't be had", () => {
        const containerTemplate = { ...template, agent_type: "container", container_image: "ghcr.io/x/y:latest" } as AgentDefinition;

        const renderContainerTemplate = () => {
            vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: true });
            renderForm({ tpl: containerTemplate });
            return screen.getByTestId("create-from-template-runtime-select") as HTMLSelectElement;
        };

        it("asks about the template's own image", async () => {
            const select = renderContainerTemplate();
            await waitFor(() => expect(select.value).toBe("container"));
            expect(RpcApi.ContainerImageCheckCommand).toHaveBeenCalledWith(
                expect.anything(),
                { image: "ghcr.io/x/y:latest" },
                expect.anything(),
            );
        });

        it.each(["denied", "not_found"] as const)("defaults to host and says why when the registry says %s", async (status) => {
            vi.mocked(RpcApi.ContainerImageCheckCommand).mockResolvedValue({ status });
            const select = renderContainerTemplate();
            const note = await screen.findByTestId("create-from-template-image-note");
            expect(note.textContent).toMatch(/Native/);
            expect(select.value).toBe("host");
            // Still selectable: the user may know better.
            const containerOpt = Array.from(select.options).find((o) => o.value === "container")!;
            expect(containerOpt.disabled).toBe(false);
        });

        it("keeps the container default when the check fails or has no answer", async () => {
            vi.mocked(RpcApi.ContainerImageCheckCommand).mockRejectedValue(new Error("offline"));
            const select = renderContainerTemplate();
            await waitFor(() => expect(select.value).toBe("container"));
            expect(screen.queryByTestId("create-from-template-image-note")).toBeNull();
        });

        it("does not override a runtime the user picked while the check was running", async () => {
            let answer!: (v: { status: "denied" }) => void;
            vi.mocked(RpcApi.ContainerImageCheckCommand).mockReturnValue(
                new Promise((resolve) => {
                    answer = resolve as typeof answer;
                }),
            );
            const select = renderContainerTemplate();
            await waitFor(() => expect(RpcApi.ContainerImageCheckCommand).toHaveBeenCalled());
            fireEvent.change(select, { target: { value: "container" } });
            answer({ status: "denied" });
            await screen.findByTestId("create-from-template-image-note");
            expect(select.value).toBe("container");
        });
    });

    it("shows an error from the create RPC", async () => {
        vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockRejectedValue(new Error("container runtime went away"));
        const { onCreated } = renderForm({ name: "Reviewer" });
        await settle();
        fireEvent.click(submitButton());
        const err = await screen.findByTestId("create-from-template-error");
        expect(err.textContent).toContain("container runtime went away");
        expect(onCreated).not.toHaveBeenCalled();
        // The form is usable again.
        expect(submitButton().disabled).toBe(false);
    });

    it("shows an error from onCreated (the launch)", async () => {
        const onCreated = vi.fn().mockRejectedValue(new Error("the launch failed"));
        renderForm({ name: "Reviewer", onCreated });
        await settle();
        fireEvent.click(submitButton());
        const err = await screen.findByTestId("create-from-template-error");
        expect(err.textContent).toContain("the launch failed");
    });

    it("says a duplicate name in words and keeps the typed name", async () => {
        vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockRejectedValue(
            new Error('agentdefcreatefromtemplate: an agent named "Reviewer" already exists'),
        );
        renderForm();
        await settle();
        fireEvent.input(nameInput(), { target: { value: "Reviewer" } });
        fireEvent.click(submitButton());
        const err = await screen.findByTestId("create-from-template-error");
        expect(err.textContent).toBe("You already have an agent named Reviewer. Choose another name.");
        expect(nameInput().value).toBe("Reviewer");
    });

    it("trims whitespace and rejects empty names", async () => {
        renderForm();
        await settle();
        fireEvent.input(nameInput(), { target: { value: "   " } });
        expect(submitButton().disabled).toBe(true);
        fireEvent.click(submitButton());
        await flush();
        expect(RpcApi.AgentDefCreateFromTemplateCommand).not.toHaveBeenCalled();
    });

    it("is busy while creating: the button says so, the fields lock, onBusyChange follows", async () => {
        let resolveCreate!: (v: typeof createResponse) => void;
        vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockReturnValue(
            new Promise((r) => {
                resolveCreate = r;
            }) as any,
        );
        const { onBusyChange, onCreated } = renderForm({ name: "Mary" });
        await settle();
        expect(onBusyChange).not.toHaveBeenCalledWith(true);

        fireEvent.click(submitButton());
        await waitFor(() => expect(onBusyChange).toHaveBeenLastCalledWith(true));
        expect(submitButton().textContent).toContain("Creating…");
        expect(submitButton().disabled).toBe(true);
        expect(nameInput().disabled).toBe(true);
        // A second click while creating sends nothing more.
        fireEvent.click(submitButton());

        resolveCreate(createResponse);
        await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
        await waitFor(() => expect(onBusyChange).toHaveBeenLastCalledWith(false));
        expect(RpcApi.AgentDefCreateFromTemplateCommand).toHaveBeenCalledTimes(1);
        expect(submitButton().textContent).toContain("Create and start");
    });

    it("stays busy until onCreated (the launch) resolves", async () => {
        let finishLaunch!: () => void;
        const onCreated = vi.fn(
            () =>
                new Promise<void>((r) => {
                    finishLaunch = r;
                }),
        );
        const { onBusyChange } = renderForm({ name: "Mary", onCreated });
        await settle();
        fireEvent.click(submitButton());
        await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
        expect(onBusyChange).toHaveBeenLastCalledWith(true);
        finishLaunch();
        await waitFor(() => expect(onBusyChange).toHaveBeenLastCalledWith(false));
    });

    it("keeps what was typed in the draft across an unmount and remount", async () => {
        const draft = createCreateDraft(template);
        const first = renderForm({ draft });
        fireEvent.input(nameInput(), { target: { value: "Mary" } });
        first.unmount();
        expect(screen.queryByTestId("create-from-template-name-input")).toBeNull();

        renderForm({ draft });
        expect(nameInput().value).toBe("Mary");
    });

    it("keeps a picked model in the draft across a remount", async () => {
        const draft = createCreateDraft(template);
        const first = renderForm({ draft });
        fireEvent.change(screen.getByTestId("create-from-template-model-select"), { target: { value: "opus" } });
        first.unmount();

        renderForm({ draft });
        expect((screen.getByTestId("create-from-template-model-select") as HTMLSelectElement).value).toBe("opus");
    });

    it("shows the Model Vendor / Custom Endpoint field for a provider that declares baseUrlEnvVar (claude) and sends it", async () => {
        renderForm({ name: "Reviewer" });
        await settle();
        const input = screen.getByTestId("create-from-template-vendor-base-url-input") as HTMLInputElement;
        fireEvent.input(input, { target: { value: "https://my-proxy.example.com" } });
        expect((await submitAndGetPayload()).model_vendor_base_url).toBe("https://my-proxy.example.com");
    });

    it("hides the Model Vendor / Custom Endpoint field for a provider that doesn't declare baseUrlEnvVar, and sends an empty override", async () => {
        renderForm({ tpl: { ...template, provider: "codex" } as AgentDefinition, name: "Reviewer" });
        await settle();
        expect(screen.queryByTestId("create-from-template-vendor-base-url-input")).toBeNull();
        expect((await submitAndGetPayload()).model_vendor_base_url).toBe("");
    });

    // #2594 follow-up: harness-then-model creation flow. The template
    // card already picked the harness (claude); these cover the model
    // picker that lets the user choose WHICH model that harness runs.
    describe("model picker (harness-then-model creation flow)", () => {
        it("renders a model select defaulting to the harness's own default model", () => {
            renderForm();
            const select = screen.getByTestId("create-from-template-model-select") as HTMLSelectElement;
            // claude's catalog entry marks "sonnet" as `default: true`.
            expect(select.value).toBe("sonnet");
            expect(Array.from(select.options).map((o) => o.value)).toContain("opus");
        });

        it("shows the harness-vs-model explanatory hint for a harness with a models list", () => {
            renderForm();
            expect(screen.getByText(/is the/)).toHaveTextContent("harness");
        });

        it("hands the user's picked model to onCreated", async () => {
            const { onCreated } = renderForm({ name: "Reviewer" });
            await settle();
            fireEvent.change(screen.getByTestId("create-from-template-model-select"), { target: { value: "opus" } });
            fireEvent.click(submitButton());
            await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
            expect(onCreated.mock.calls[0][0].model).toBe("opus");
        });

        it("hides the model select, the hint, and hands an empty model for a harness with no models list", async () => {
            const { onCreated } = renderForm({
                tpl: { ...template, provider: "not-a-real-provider" } as AgentDefinition,
                name: "Reviewer",
            });
            await settle();
            expect(screen.queryByTestId("create-from-template-model-select")).toBeNull();
            expect(screen.queryByText(/is the/)).toBeNull();
            fireEvent.click(submitButton());
            await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
            expect(onCreated.mock.calls[0][0].model).toBe("");
        });
    });

    // Every provider-dependent decision in this form (model list, account
    // filter, container support, vendor-endpoint field) must resolve through
    // the launch's own resolver, not the template's possibly-drifted
    // `.provider` column directly; `agentdefcreatefromtemplate` resolves the
    // clone's provider the same way server-side (template.rs, #2607). A
    // template with no provider of its own takes its bundle's
    // (SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md §3.1) (#2618).
    describe("resolves the provider through resolveEffectiveLaunchProvider", () => {
        const drifted = { ...template, provider: "", memory_id: "mem-1" } as AgentDefinition;

        it("asks the resolver about the template it was given", async () => {
            renderForm({ tpl: drifted });
            await settle();
            // createResource also passes its own fetcher info as a second argument.
            expect(vi.mocked(resolveEffectiveLaunchProvider).mock.calls[0][0]).toBe(drifted);
        });

        it("shows the resolved (bundle) provider's models, not the drifted column's", async () => {
            vi.mocked(resolveEffectiveLaunchProvider).mockResolvedValue("claude");
            renderForm({ tpl: drifted });
            await settle();
            const select = (await screen.findByTestId("create-from-template-model-select")) as HTMLSelectElement;
            const options = Array.from(select.options).map((o) => o.value);
            // claude's models, not codex's (gpt-5.x family).
            expect(options).toContain("opus");
            expect(options).not.toContain("gpt-5.5");
        });

        it("filters accounts by the resolved (bundle) provider, not the drifted column's", async () => {
            vi.mocked(resolveEffectiveLaunchProvider).mockResolvedValue("claude");
            vi.mocked(refreshAccountCache).mockResolvedValue([
                { id: "acct-codex", name: "Codex Work", provider: "codex" } as any,
                { id: "acct-claude", name: "Claude Work", provider: "claude" } as any,
            ]);
            renderForm({ tpl: drifted });
            await settle();
            expect(await submittedAccount()).toBe("acct-claude");
        });

        it("does not send an account filtered against the stale fallback provider before the resolution lands", async () => {
            // Accounts resolve BEFORE the provider, so the account pick runs
            // against the fallback provider first: that is what makes the
            // race reproducible.
            vi.mocked(refreshAccountCache).mockResolvedValue([
                { id: "acct-claude", name: "Claude Work", provider: "claude" } as any,
                { id: "acct-codex", name: "Codex Work", provider: "codex" } as any,
            ]);
            let resolveProvider!: (v: string) => void;
            vi.mocked(resolveEffectiveLaunchProvider).mockReturnValue(
                new Promise((resolve) => {
                    resolveProvider = resolve;
                }),
            );
            renderForm({ tpl: { ...drifted, provider: "claude" } as AgentDefinition });
            // Let the already-resolved accounts fetch settle, including the
            // account pick's own commit, before the provider resolves.
            await flush();
            await flush();

            resolveProvider("codex");
            await settle();
            expect(await submittedAccount()).toBe("acct-codex");
        });

        it("keeps Create disabled until the accounts and the provider have loaded", async () => {
            // There's no account field to correct a premature pick, so Create
            // must not submit with an empty or stale-provider account.
            let resolveAccounts!: (v: any) => void;
            vi.mocked(refreshAccountCache).mockReturnValue(
                new Promise((r) => {
                    resolveAccounts = r;
                }),
            );
            let resolveProvider!: (v: string) => void;
            vi.mocked(resolveEffectiveLaunchProvider).mockReturnValue(
                new Promise((r) => {
                    resolveProvider = r;
                }),
            );
            renderForm({ tpl: drifted });
            fireEvent.input(nameInput(), { target: { value: "Mary" } });
            await flush();
            expect(submitButton().disabled).toBe(true);
            resolveAccounts([{ id: "acct-codex", name: "Codex Work", provider: "codex" }]);
            await flush();
            expect(submitButton().disabled).toBe(true);
            resolveProvider("codex");
            await flush();
            await flush();
            expect(submitButton().disabled).toBe(false);
            expect((await submitAndGetPayload()).identity_id).toBe("acct-codex");
        });

        it("still creates (with no account) when the account list can't be read", async () => {
            vi.mocked(refreshAccountCache).mockRejectedValue(new Error("offline"));
            renderForm({ name: "Mary" });
            await settle();
            expect(submitButton().disabled).toBe(false);
            expect((await submitAndGetPayload()).identity_id).toBe("");
        });

        it("has no account field, and a Bundles field", async () => {
            renderForm();
            await flush();
            expect(screen.queryByTestId("create-from-template-identity-select")).toBeNull();
            expect(screen.queryByText("Identity")).toBeNull();
            expect(screen.getByRole("group", { name: "Bundles" })).toBeTruthy();
        });

        it("offers only real bundles: no blank or system ones", async () => {
            vi.mocked(RpcApi.ListBundlesCommand).mockResolvedValue([
                { id: "mem-notes", name: "Notes", is_blank: false } as any,
                { id: "mem-blank", name: "Blank", is_blank: true } as any,
                { id: "mem-system", name: "Workspace policy", is_system: true } as any,
            ]);
            renderForm();
            await settle();
            const add = screen.getByTestId("bundle-list-editor-add") as HTMLSelectElement;
            expect(Array.from(add.options).filter((o) => o.value).map((o) => o.value)).toEqual(["mem-notes"]);
        });

        it("sends the bundles picked, in order", async () => {
            vi.mocked(RpcApi.ListBundlesCommand).mockResolvedValue([
                { id: "mem-notes", name: "Notes", is_blank: false } as any,
                { id: "mem-style", name: "Style", is_blank: false } as any,
            ]);
            renderForm();
            await settle();
            const add = () => screen.getByTestId("bundle-list-editor-add") as HTMLSelectElement;
            const real = Array.from(add().options).filter((o) => o.value).map((o) => o.value);
            expect(real).toEqual(["mem-notes", "mem-style"]);
            // Pick every bundle, then move the last to the front.
            for (const value of real) fireEvent.change(add(), { target: { value } });
            const ups = screen.getAllByRole("button", { name: "Move up" });
            fireEvent.click(ups[ups.length - 1]);

            fireEvent.input(nameInput(), { target: { value: "Mary" } });
            const payload = await submitAndGetPayload();
            expect(payload.bundle_ids).toEqual(["mem-style", "mem-notes"]);
            // The first pick is echoed back for the launch's record.
            expect(payload.memory_id).toBe("mem-style");
        });
    });

    // Antigravity's models are `agy models`' ids, and buildRuntimeArgs passes
    // the one picked as --model (SPEC_ANTIGRAVITY_HARNESS_REAL_CLI_2026_10_06.md),
    // so the picker is offered and its default is what gets used. A harness
    // whose `--model` isn't wired gets no picker at all, since the launch
    // would drop the choice.
    it("offers agy's own models for antigravity, defaulting to the catalog's default", async () => {
        const { onCreated } = renderForm({ tpl: { ...template, provider: "antigravity" } as AgentDefinition, name: "Reviewer" });
        await settle();
        const select = screen.getByTestId("create-from-template-model-select") as HTMLSelectElement;
        expect(select.value).toBe("gemini-3.8-flash-medium");
        expect(Array.from(select.options).map((o) => o.value)).toContain("gemini-3.1-pro-high");

        fireEvent.click(submitButton());
        await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
        expect(onCreated.mock.calls[0][0].model).toBe("gemini-3.8-flash-medium");
    });

    // The model list reads through getProvider(), so a label refreshed from
    // the Models API reaches this form (#2618). The overlay is module-level
    // state, so this stays the LAST test; it only relabels, never revalues.
    it("shows a live-overlaid model label, not the stale static catalog label", async () => {
        const { setProviderModels } = await import("../providers");
        setProviderModels("claude", [{ value: "claude-sonnet-5-5", label: "Claude Sonnet 5.5" }]);

        renderForm();
        const select = screen.getByTestId("create-from-template-model-select") as HTMLSelectElement;
        const sonnetOption = Array.from(select.options).find((o) => o.value === "sonnet")!;
        expect(sonnetOption.textContent).toBe("Sonnet 5.5");
        // value/default untouched by the label refresh.
        expect(select.value).toBe("sonnet");
    });
});
