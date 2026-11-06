// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCreateForm: the paths the other test file leaves out. The submit's payload and what it hands
 * to onCreated, the busy state of every control, the keys, the model that follows a refreshed catalog,
 * the developer-image check, the runtime handed in by the panel (a container skips the host steps, so the
 * form must use what it is given), and the text the user reads. Written ahead of a split of the form.
 */

import { sleep } from "@/util/util";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

// Any mock, so a test can pass its own vi.fn() for a typed callback.
type Created = (agent: CreatedAgent) => Promise<void>;
type CreateResp = Awaited<ReturnType<typeof RpcApi.AgentDefCreateFromTemplateCommand>>;
const acct = (id: string, provider: string): Account => ({ id, name: id, provider }) as unknown as Account;
const bundle = (id: string): Bundle => ({ id, name: id, is_blank: false }) as unknown as Bundle;
const never = () => new Promise<CreateResp>(() => {});

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

import type { AgentDefinition, Bundle } from "@/app/store/rpc-api";
import { RpcApi } from "@/app/store/rpc-api";
import { resetCapabilities } from "@/app/store/toolchain-capabilities";
import type { Account } from "@/app/view/identity/identity-model";
import { refreshAccountCache } from "@/app/view/identity/identity-model";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
import { replaceProviderModels } from "../providers";
import { PROVIDERS } from "../providers/catalog";
import { AgentCreateForm, createCreateDraft, type CreateDraft, type CreatedAgent } from "./AgentCreateForm";
import type { RuntimeChoice } from "./use-runtime-choice";

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
    vi.mocked(refreshAccountCache).mockResolvedValue([acct("id-work", "claude")]);
    vi.mocked(RpcApi.ListBundlesCommand).mockResolvedValue([bundle("mem-notes")]);
    // Default: Docker daemon not reachable → host-only. Consumed via the
    // shared toolchain-capabilities store, not called directly by the
    // component — see docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
    vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: false });
    vi.mocked(RpcApi.ContainerImageCheckCommand).mockResolvedValue({ status: "public" });
    vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockResolvedValue(createResponse as unknown as CreateResp);
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
    onCreated?: Mock<Created>;
    onCancel?: Mock<() => void>;
    onBusyChange?: Mock<(busy: boolean) => void>;
    runtime?: RuntimeChoice;
}

function renderForm(opts: RenderOpts = {}) {
    const tpl = opts.tpl ?? template;
    const draft = opts.draft ?? createCreateDraft(tpl);
    if (opts.name != null) draft.setName(opts.name);
    const onCreated: Mock<Created> = opts.onCreated ?? vi.fn<Created>().mockResolvedValue(undefined);
    const onCancel: Mock<() => void> = opts.onCancel ?? vi.fn();
    const onBusyChange: Mock<(busy: boolean) => void> = opts.onBusyChange ?? vi.fn();
    const result = render(() => (
        <AgentCreateForm
            template={tpl}
            draft={draft}
            runtime={opts.runtime}
            onCreated={onCreated}
            onCancel={onCancel}
            onBusyChange={onBusyChange}
        />
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

/** A runtime the panel could hand in: Docker reachable unless said otherwise. */
function fakeRuntime(over: Partial<{ [K in keyof RuntimeChoice]: unknown }> = {}) {
    const pickRuntime = vi.fn();
    const runtime = {
        containerSupported: () => true,
        canPickContainer: () => true,
        imageAccess: () => undefined,
        deciding: () => false,
        pickRuntime,
        ...over,
    } as unknown as RuntimeChoice;
    return { runtime, pickRuntime };
}

const containerTemplate = { ...template, agent_type: "container" } as AgentDefinition;
const runtimeSelect = () => screen.getByTestId("create-from-template-runtime-select") as HTMLSelectElement;
const imageSelect = () => screen.getByTestId("create-from-template-image-select") as HTMLSelectElement;
const modelSelect = () => screen.getByTestId("create-from-template-model-select") as HTMLSelectElement;
const vendorInput = () => screen.getByTestId("create-from-template-vendor-base-url-input") as HTMLInputElement;

async function containerOn(over: Partial<RenderOpts> = {}) {
    vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: true });
    const r = renderForm({ tpl: containerTemplate, ...over });
    await waitFor(() => expect(runtimeSelect().value).toBe("container"));
    return r;
}

describe("the draft", () => {
    it("opens as a native agent with no bundles, the standard image and nothing touched", () => {
        const d = createCreateDraft(template);
        expect(d.name()).toBe("");
        expect(d.runtime()).toBe("host");
        expect(d.containerImage()).toBe("standard");
        expect(d.bundleIds()).toEqual([]);
        expect(d.model()).toBe("");
        expect(d.runtimeTouched).toBe(false);
        expect(d.modelTouched).toBe(false);
    });

    it("starts the custom endpoint from the template's own", () => {
        expect(
            createCreateDraft({
                ...template,
                model_vendor_base_url: "https://proxy.example",
            } as AgentDefinition).modelVendorBaseUrl()
        ).toBe("https://proxy.example");
        expect(createCreateDraft(template).modelVendorBaseUrl()).toBe("");
    });
});

describe("what Create sends and hands on", () => {
    it("hands on what the server returned, not what the form had", async () => {
        vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockResolvedValue({
            definition_id: "def-9",
            identity_id: "id-from-server",
            memory_id: "mem-from-server",
            slug: "slug-9",
        } as unknown as CreateResp);
        vi.mocked(RpcApi.ListBundlesCommand).mockResolvedValue([bundle("mem-notes")]);
        const { onCreated } = renderForm({ name: "Mary" });
        await settle();
        fireEvent.change(screen.getByTestId("bundle-list-editor-add"), { target: { value: "mem-notes" } });
        fireEvent.click(submitButton());
        await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
        expect(onCreated.mock.calls[0][0]).toEqual({
            definitionId: "def-9",
            identityId: "id-from-server",
            memoryId: "mem-from-server",
            name: "Mary",
            agentType: "host",
            model: "sonnet",
            slug: "slug-9",
        });
    });

    it("hands on the container runtime it created the agent with", async () => {
        const { onCreated } = await containerOn({ name: "Boxed" });
        await settle();
        fireEvent.click(submitButton());
        await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
        expect(onCreated.mock.calls[0][0].agentType).toBe("container");
    });

    it("asks for no image when a native agent's draft still holds the developer choice", async () => {
        const draft = createCreateDraft(template);
        draft.setContainerImage("developer");
        renderForm({ draft, name: "Mary" });
        await settle();
        expect((await submitAndGetPayload()).container_image).toBeUndefined();
    });

    it("sends the account the list gives first, not the last", async () => {
        vi.mocked(refreshAccountCache).mockResolvedValue([acct("acct-1", "claude"), acct("acct-2", "claude")]);
        renderForm();
        await settle();
        expect(await submittedAccount()).toBe("acct-1");
    });

    it("trims the custom endpoint, and sends none for a harness that takes none even if the draft holds one", async () => {
        renderForm({ name: "Mary" });
        await settle();
        fireEvent.input(vendorInput(), { target: { value: "  https://proxy.example  " } });
        expect((await submitAndGetPayload()).model_vendor_base_url).toBe("https://proxy.example");
        cleanup();
        vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockClear();
        const codex = { ...template, provider: "codex" } as AgentDefinition;
        const draft = createCreateDraft(codex);
        draft.setModelVendorBaseUrl("https://left-over.example");
        renderForm({ tpl: codex, draft, name: "Mary" });
        await settle();
        expect((await submitAndGetPayload()).model_vendor_base_url).toBe("");
    });

    it("sends no model for a harness without a models list, even if the draft holds one", async () => {
        const unknown = { ...template, provider: "not-a-real-provider" } as AgentDefinition;
        const draft = createCreateDraft(unknown);
        draft.modelTouched = true;
        draft.setModel("stale");
        const { onCreated } = renderForm({ tpl: unknown, draft, name: "Mary" });
        await settle();
        fireEvent.click(submitButton());
        await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
        expect(onCreated.mock.calls[0][0].model).toBe("");
    });

    it("leaves the draft's model empty for a harness without a models list", async () => {
        const unknown = { ...template, provider: "not-a-real-provider" } as AgentDefinition;
        const { draft } = renderForm({ tpl: unknown });
        await settle();
        expect(draft.model()).toBe("");
    });
});

describe("the name", () => {
    const longName = (n: number) => "a".repeat(n);

    it("takes up to 200 characters after trimming, and no more", async () => {
        renderForm();
        await settle();
        fireEvent.input(nameInput(), { target: { value: longName(200) } });
        expect(submitButton().disabled).toBe(false);
        fireEvent.input(nameInput(), { target: { value: longName(201) } });
        expect(submitButton().disabled).toBe(true);
        fireEvent.input(nameInput(), { target: { value: `  ${longName(200)}  ` } });
        expect(submitButton().disabled).toBe(false);
    });

    it("is a field that opens focused and stops at 200 characters", async () => {
        renderForm();
        await flush();
        expect(nameInput().maxLength).toBe(200);
        expect(document.activeElement).toBe(nameInput());
    });

    it("says the harness by the template's name", () => {
        renderForm();
        expect(document.querySelector(".agent-create-form-hint")?.textContent).toMatch(/^Claude Code is the harness/);
    });
});

describe("the keys", () => {
    it("Enter submits and is kept from the browser, in the name and in the endpoint", async () => {
        renderForm({ name: "Mary" });
        await settle();
        const notPrevented = fireEvent.keyDown(nameInput(), { key: "Enter" });
        expect(notPrevented).toBe(false);
        await waitFor(() => expect(RpcApi.AgentDefCreateFromTemplateCommand).toHaveBeenCalledTimes(1));
        await settle();
        fireEvent.keyDown(vendorInput(), { key: "Enter" });
        await waitFor(() => expect(RpcApi.AgentDefCreateFromTemplateCommand).toHaveBeenCalledTimes(2));
    });

    it("Shift+Enter and other keys are left to the browser", async () => {
        renderForm({ name: "Mary" });
        await settle();
        expect(fireEvent.keyDown(nameInput(), { key: "Enter", shiftKey: true })).toBe(true);
        expect(fireEvent.keyDown(nameInput(), { key: "a" })).toBe(true);
        await flush();
        expect(RpcApi.AgentDefCreateFromTemplateCommand).not.toHaveBeenCalled();
    });

    it("Enter while an input method is composing does not submit", async () => {
        renderForm({ name: "Mary" });
        await settle();
        fireEvent.keyDown(nameInput(), { key: "Enter", isComposing: true });
        await flush();
        expect(RpcApi.AgentDefCreateFromTemplateCommand).not.toHaveBeenCalled();
    });

    it("Enter while creating sends nothing more", async () => {
        vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockReturnValue(never());
        renderForm({ name: "Mary" });
        await settle();
        fireEvent.click(submitButton());
        await waitFor(() => expect(RpcApi.AgentDefCreateFromTemplateCommand).toHaveBeenCalledTimes(1));
        fireEvent.keyDown(nameInput(), { key: "Enter" });
        await flush();
        expect(RpcApi.AgentDefCreateFromTemplateCommand).toHaveBeenCalledTimes(1);
    });
});

describe("while creating", () => {
    it("locks every control, marks Create busy, and clears the busy flag when the pane closes", async () => {
        vi.mocked(RpcApi.ListBundlesCommand).mockResolvedValue([bundle("mem-notes")]);
        vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockReturnValue(never());
        const { onBusyChange, unmount } = await containerOn({ name: "Mary" });
        await settle();
        fireEvent.click(submitButton());
        await waitFor(() => expect(onBusyChange).toHaveBeenLastCalledWith(true));
        expect(submitButton().getAttribute("aria-busy")).toBe("true");
        expect(nameInput().disabled).toBe(true);
        expect(runtimeSelect().disabled).toBe(true);
        expect(imageSelect().disabled).toBe(true);
        expect(modelSelect().disabled).toBe(true);
        expect(vendorInput().disabled).toBe(true);
        expect((screen.getByTestId("bundle-list-editor-add") as HTMLSelectElement).disabled).toBe(true);
        expect((screen.getByText("Cancel") as HTMLButtonElement).disabled).toBe(true);
        unmount();
        expect(onBusyChange).toHaveBeenLastCalledWith(false);
    });

    it("starts from not busy and with no error showing", async () => {
        const { onBusyChange } = renderForm({ name: "Mary" });
        await settle();
        expect(onBusyChange).toHaveBeenLastCalledWith(false);
        expect(screen.queryByTestId("create-from-template-error")).toBeNull();
        expect(submitButton().getAttribute("aria-busy")).toBeNull();
    });
});

describe("the error", () => {
    it("is shown as an alert, as the server worded it, and goes away when Create is tried again", async () => {
        vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockRejectedValueOnce(new Error("the registry is down"));
        renderForm({ name: "Mary" });
        await settle();
        fireEvent.click(submitButton());
        const err = await screen.findByRole("alert");
        expect(err.textContent).toBe("the registry is down");
        vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockReturnValue(never());
        fireEvent.click(submitButton());
        await waitFor(() => expect(RpcApi.AgentDefCreateFromTemplateCommand).toHaveBeenCalledTimes(2));
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("shows a refusal that is not an Error as its text", async () => {
        vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockRejectedValue("boom");
        renderForm({ name: "Mary" });
        await settle();
        fireEvent.click(submitButton());
        expect((await screen.findByTestId("create-from-template-error")).textContent).toBe("boom");
    });

    it("names the duplicate by the trimmed name", async () => {
        vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mockRejectedValue(
            new Error('agentdefcreatefromtemplate: an agent named "Reviewer" already exists')
        );
        renderForm();
        await settle();
        fireEvent.input(nameInput(), { target: { value: "  Reviewer  " } });
        fireEvent.click(submitButton());
        expect((await screen.findByTestId("create-from-template-error")).textContent).toBe(
            "You already have an agent named Reviewer. Choose another name."
        );
    });
});

describe("the accounts and bundles lists", () => {
    it("keeps Create disabled while the accounts load, even once the provider has", async () => {
        let resolveAccounts!: (v: Account[]) => void;
        vi.mocked(refreshAccountCache).mockReturnValue(
            new Promise((r) => {
                resolveAccounts = r;
            })
        );
        renderForm({ name: "Mary" });
        await settle();
        expect(submitButton().disabled).toBe(true);
        fireEvent.click(submitButton());
        await flush();
        expect(RpcApi.AgentDefCreateFromTemplateCommand).not.toHaveBeenCalled();
        resolveAccounts([]);
        await settle();
        expect(submitButton().disabled).toBe(false);
    });

    it("copes with a bundle list that comes back empty-handed", async () => {
        vi.mocked(RpcApi.ListBundlesCommand).mockResolvedValue(undefined as unknown as Bundle[]);
        renderForm({ name: "Mary" });
        await settle();
        expect(screen.getByRole("group", { name: "Bundles" })).toBeTruthy();
        expect(submitButton().disabled).toBe(false);
    });
});

describe("the provider the template resolves to", () => {
    it("offers the custom endpoint of the resolved provider", async () => {
        vi.mocked(resolveEffectiveLaunchProvider).mockResolvedValue("claude");
        renderForm({ tpl: { ...template, provider: "codex" } as AgentDefinition });
        await settle();
        expect(vendorInput().placeholder).toBe(`Default (${PROVIDERS.claude.baseUrlEnvVar})`);
    });

    it("offers the models of the resolved provider when the template's own offers none", async () => {
        vi.mocked(resolveEffectiveLaunchProvider).mockResolvedValue("claude");
        renderForm({ tpl: { ...template, provider: "not-a-real-provider" } as AgentDefinition });
        await settle();
        expect(Array.from(modelSelect().options).map((o) => o.value)).toContain("opus");
    });

    it("tells a template whose resolved provider has no container image that it runs natively", async () => {
        vi.mocked(resolveEffectiveLaunchProvider).mockResolvedValue("codex");
        renderForm();
        await settle();
        expect(screen.getByText("Claude Code can only run as a Native agent.")).toBeTruthy();
    });
});

describe("the runtime the panel hands in", () => {
    it("is the one the form follows: its container option and its pick", async () => {
        const { runtime, pickRuntime } = fakeRuntime();
        renderForm({ runtime });
        await settle();
        const containerOpt = Array.from(runtimeSelect().options).find((o) => o.value === "container")!;
        // Docker is not reachable in this test, so the form's own choice would have disabled it.
        expect(containerOpt.disabled).toBe(false);
        expect(containerOpt.textContent).not.toContain("Docker not detected");
        fireEvent.change(runtimeSelect(), { target: { value: "container" } });
        expect(pickRuntime).toHaveBeenCalledWith("container");
    });

    it("lets a container agent skip the host steps: the form shows the image choice and asks for the agent as a container", async () => {
        const { runtime } = fakeRuntime();
        const draft = createCreateDraft(template);
        draft.setRuntime("container");
        const { onCreated } = renderForm({ runtime, draft, name: "Boxed" });
        await settle();
        expect(imageSelect()).toBeTruthy();
        fireEvent.click(submitButton());
        await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
        expect(onCreated.mock.calls[0][0].agentType).toBe("container");
        expect(vi.mocked(RpcApi.AgentDefCreateFromTemplateCommand).mock.calls[0][1].agent_type).toBe("container");
    });

    it("says why a harness without a container image runs natively, and offers no image choice", async () => {
        const { runtime } = fakeRuntime({
            containerSupported: () => false,
            canPickContainer: () => false,
            imageAccess: () => "denied",
        });
        const draft = createCreateDraft(template);
        draft.setRuntime("container");
        renderForm({ runtime, draft });
        await settle();
        expect(screen.getByText("Claude Code can only run as a Native agent.")).toBeTruthy();
        expect(screen.queryByTestId("create-from-template-image-note")).toBeNull();
        expect(screen.queryByTestId("create-from-template-image-select")).toBeNull();
    });

    it("says what the registry said about the image, only where a container is possible", async () => {
        const { runtime } = fakeRuntime({ imageAccess: () => "not_found" });
        renderForm({ runtime });
        await settle();
        expect(screen.getByTestId("create-from-template-image-note").textContent).toMatch(/wasn't found/);
        expect(screen.queryByText(/can only run as a Native agent/)).toBeNull();
    });
});

describe("the runtime row", () => {
    it("says Docker was not found on the container option, and only then", async () => {
        renderForm();
        await settle();
        const opt = () => Array.from(runtimeSelect().options).find((o) => o.value === "container")!;
        expect(opt().textContent).toContain("Docker not detected");
        cleanup();
        vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: true });
        resetCapabilities();
        renderForm();
        await waitFor(() => expect(opt().disabled).toBe(false));
        expect(opt().textContent).not.toContain("Docker not detected");
    });
});

describe("the container image choice", () => {
    it("describes the image chosen", async () => {
        await containerOn();
        expect(screen.getByTestId("create-from-template-image-choice-note").textContent).toBe(
            "The smallest image: the agent, git, GitHub CLI and Python."
        );
        fireEvent.change(imageSelect(), { target: { value: "developer" } });
        expect(screen.getByTestId("create-from-template-image-choice-note").textContent).toMatch(
            /^Adds Rust, a C toolchain/
        );
    });

    it("checks the developer image once, with a ten-second limit, and drops the warning when standard is chosen again", async () => {
        await containerOn();
        vi.mocked(RpcApi.ContainerImageCheckCommand).mockClear();
        vi.mocked(RpcApi.ContainerImageCheckCommand).mockResolvedValue({ status: "denied" });
        fireEvent.change(imageSelect(), { target: { value: "developer" } });
        const note = await screen.findByTestId("create-from-template-developer-image-note");
        expect(note.textContent).toMatch(/registry refuses access/);
        expect(vi.mocked(RpcApi.ContainerImageCheckCommand).mock.calls[0][2]).toEqual({ timeout: 10000 });
        fireEvent.change(imageSelect(), { target: { value: "standard" } });
        expect(screen.queryByTestId("create-from-template-developer-image-note")).toBeNull();
        fireEvent.change(imageSelect(), { target: { value: "developer" } });
        await settle();
        expect(vi.mocked(RpcApi.ContainerImageCheckCommand)).toHaveBeenCalledTimes(1);
    });

    it("does not check the developer image where no container can run", async () => {
        const draft = createCreateDraft(template);
        draft.setContainerImage("developer");
        renderForm({ draft });
        await settle();
        expect(RpcApi.ContainerImageCheckCommand).not.toHaveBeenCalled();
    });
});

describe("the model", () => {
    const models = (...values: string[]) => values.map((value) => ({ value, label: value }));

    afterEach(() => replaceProviderModels("claude", []));

    it("follows the same family when the live catalog replaces the model picked", async () => {
        const { draft } = renderForm();
        await settle();
        fireEvent.change(modelSelect(), { target: { value: "sonnet" } });
        replaceProviderModels("claude", models("claude-opus-6", "claude-sonnet-6"));
        await settle();
        expect(draft.model()).toBe("claude-sonnet-6");
    });

    it("keeps a model the live catalog still offers, even when another of its family comes first", async () => {
        replaceProviderModels("claude", models("claude-sonnet-6", "claude-sonnet-5"));
        const { draft } = renderForm();
        await settle();
        fireEvent.change(modelSelect(), { target: { value: "claude-sonnet-5" } });
        replaceProviderModels("claude", models("claude-sonnet-6", "claude-sonnet-5", "claude-opus-6"));
        await settle();
        expect(draft.model()).toBe("claude-sonnet-5");
    });

    it("keeps the model picked when the live catalog has nothing of its family", async () => {
        const { draft } = renderForm();
        await settle();
        fireEvent.change(modelSelect(), { target: { value: "haiku" } });
        replaceProviderModels("claude", models("claude-opus-6", "claude-sonnet-6"));
        await settle();
        expect(draft.model()).toBe("haiku");
    });

    it("re-picks the default when the live catalog changes and nothing has been picked", async () => {
        const { draft } = renderForm();
        await settle();
        replaceProviderModels("claude", models("claude-opus-6", "claude-sonnet-6"));
        await settle();
        expect(draft.model()).toBe("claude-opus-6");
    });

    it("says which model the harness runs with, by the template's name", () => {
        renderForm();
        expect(screen.getByText(/^Which model Claude Code runs with\./)).toBeTruthy();
    });

    it("lists the options by their labels", () => {
        renderForm();
        const sonnet = Array.from(modelSelect().options).find((o) => o.value === "sonnet")!;
        expect(sonnet.textContent).toBe(PROVIDERS.claude.models!.find((m) => m.value === "sonnet")!.label);
    });
});
