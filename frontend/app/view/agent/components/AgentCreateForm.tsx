// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCreateForm — the Create step of the new-agent panel below a harness
 * card (SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md §2.3). It was the
 * "Create new agent from {template}" modal: same fields, same rules, now
 * inline. Collects a name, the runtime, the model, a custom endpoint where the
 * harness takes one, and the bundles (the account is the provider's first),
 * then clones the seeded template into a new user-owned definition with
 * `agentdefcreatefromtemplate` and hands it to the picker to launch.
 *
 * What the user typed lives in a `CreateDraft` the picker keeps per harness,
 * so moving to another card and back doesn't lose it.
 */

import { createEffect, createMemo, createResource, createSignal, For, onCleanup, onMount, Show, untrack, type Accessor, type JSX, type Setter } from "solid-js";

import { Button, Select, TextInput } from "@/app/element/ui";
import { BundleListEditor } from "./BundleListEditor";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { containerImageNote, shouldPreselectContainer, type ContainerImageState } from "../defaults/container-default";
import { PROVIDERS } from "../providers/catalog";
import { familyKey, getProvider } from "../providers";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
import { providerSupportsModelFlag } from "../buildRuntimeArgs";
import { isAvailable, watchCapability } from "@/app/store/toolchain-capabilities";
import { refreshAccountCache, type Account } from "@/app/view/identity/identity-model";
import type { AgentDefinition, Bundle } from "@/app/store/rpc-api";
// The field layout the bundle modal and this form share.
import "./AgentNewBundleModal.scss";

/** What the user has entered for one harness, kept while the picker lives. */
export interface CreateDraft {
    name: Accessor<string>;
    setName: Setter<string>;
    bundleIds: Accessor<string[]>;
    setBundleIds: Setter<string[]>;
    runtime: Accessor<"host" | "container">;
    setRuntime: Setter<"host" | "container">;
    /** The user chose the runtime: stop defaulting it. */
    runtimeTouched: boolean;
    model: Accessor<string>;
    setModel: Setter<string>;
    /** The user chose the model: stop defaulting it. */
    modelTouched: boolean;
    modelVendorBaseUrl: Accessor<string>;
    setModelVendorBaseUrl: Setter<string>;
}

/** A fresh draft for `template`. The name starts empty: the field never
 *  suggests the template's name (SPEC_CREATE_AGENT_NAME_NO_PREFILL_2026_10_06.md). */
export function createCreateDraft(template: AgentDefinition): CreateDraft {
    const [name, setName] = createSignal("");
    const [bundleIds, setBundleIds] = createSignal<string[]>([]);
    const [runtime, setRuntime] = createSignal<"host" | "container">("host");
    const [model, setModel] = createSignal("");
    const [modelVendorBaseUrl, setModelVendorBaseUrl] = createSignal(template.model_vendor_base_url ?? "");
    return {
        name,
        setName,
        bundleIds,
        setBundleIds,
        runtime,
        setRuntime,
        runtimeTouched: false,
        model,
        setModel,
        modelTouched: false,
        modelVendorBaseUrl,
        setModelVendorBaseUrl,
    };
}

/** What `agentdefcreatefromtemplate` returned, plus what the launch needs. */
export interface CreatedAgent {
    definitionId: string;
    identityId: string;
    memoryId: string;
    name: string;
    agentType: "host" | "container";
    model: string;
    slug: string;
}

interface AgentCreateFormProps {
    /** Seeded template the user clicked. */
    template: AgentDefinition;
    draft: CreateDraft;
    /** Launches the new agent. The form stays busy until it resolves. */
    onCreated: (agent: CreatedAgent) => Promise<void>;
    onCancel: () => void;
    /** Creating: the panel can't be closed meanwhile. */
    onBusyChange?: (busy: boolean) => void;
}

/**
 * The submit error as the form shows it. The server's duplicate-name refusal
 * reaches us prefixed with its RPC name (`agentdefcreatefromtemplate: an agent
 * named "X" already exists`); say it in words instead, with the name as typed.
 * Every other error passes through unchanged.
 */
export function friendlySubmitError(message: string, typedName: string): string {
    if (/an agent named .* already exists/.test(message)) {
        return `You already have an agent named ${typedName}. Choose another name.`;
    }
    return message;
}

export const AgentCreateForm = (props: AgentCreateFormProps): JSX.Element => {
    const draft = props.draft;
    const [accountId, setAccountId] = createSignal("");
    const [allAccounts, setAllAccounts] = createSignal<Account[]>([]);
    // The account list has been fetched (or failed to): Create waits for it,
    // since the first account is used without a field to show it.
    const [accountsReady, setAccountsReady] = createSignal(false);
    const [bundles, setBundles] = createSignal<Bundle[]>([]);
    const [submitting, setSubmitting] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    createEffect(() => props.onBusyChange?.(submitting()));
    onCleanup(() => props.onBusyChange?.(false));

    // Through the same resolver `agentdefcreatefromtemplate` uses for the
    // clone's provider server-side (template.rs), so this form offers
    // accounts, models and endpoint support for the provider that actually
    // gets cloned: the template's own, or its bundle's when it has none
    // (SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md §3.1). Falls back to
    // `props.template.provider` while loading or on failure. The provider
    // picks the account the agent is created with, and the form has no
    // account field to correct it, so Create waits for this (`canSubmit`).
    const [resolvedTemplateProviderId] = createResource(() => props.template, resolveEffectiveLaunchProvider);
    const effectiveProviderId = () => resolvedTemplateProviderId() ?? props.template.provider;

    // Derived, not filtered once at fetch time, so it reacts whichever of the
    // account fetch and the provider resolution lands first (#2596).
    const accounts = createMemo(() => allAccounts().filter((a) => a.provider === effectiveProviderId()));

    // Only providers that declare `baseUrlEnvVar` (currently just claude) can
    // be redirected to a custom endpoint; the backend rejects an override for
    // any other (agent_define::validate_vendor_base_url).
    const supportsCustomEndpoint = createMemo(() => !!PROVIDERS[effectiveProviderId()]?.baseUrlEnvVar);

    // ── Model (which model this harness runs) ────────────────────────
    // The card picked the HARNESS (the CLI); this picks WHICH MODEL it runs,
    // from its own list. Read through getProvider() so it sees the live,
    // API-sourced labels (PR #2618), and gated on providerSupportsModelFlag
    // so a harness with no `--model` wiring doesn't offer a choice the launch
    // would drop. Hidden when neither applies.
    const modelOptions = createMemo(() =>
        providerSupportsModelFlag(effectiveProviderId()) ? getProvider(effectiveProviderId())?.models ?? [] : [],
    );
    // Re-pick the default whenever modelOptions() changes (it starts from the
    // fallback provider before the resolution lands, #2596), until the user
    // chooses. After that, follow a concrete option value that the live
    // catalog replaced under them (#2990).
    createEffect(() => {
        const opts = modelOptions();
        if (!draft.modelTouched) {
            draft.setModel(opts.find((m) => m.default)?.value ?? opts[0]?.value ?? "");
            return;
        }
        const current = untrack(draft.model);
        if (!current || opts.some((m) => m.value === current)) return;
        const replacement = opts.find((m) => familyKey(m.value) === familyKey(current));
        if (replacement) draft.setModel(replacement.value);
    });
    const pickModel = (v: string) => {
        draft.modelTouched = true;
        draft.setModel(v);
    };

    // ── Runtime (host vs container) ────────────────────────────────
    // Decided here, when instantiating the template (a template is
    // runtime-agnostic). The container is offered only when the CLI can run
    // in one and Docker is reachable, and host is the default so the new agent
    // always starts.
    const containerSupported = createMemo(() => getCliCatalogEntry(effectiveProviderId())?.containerSupported ?? true);
    // The shared toolchain-capabilities store (a daemon ping), so this can
    // never disagree with the Toolchain or the launch pre-flight about Docker
    // (#1576; RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md).
    const canPickContainer = () => containerSupported() && isAvailable("docker");
    // Poll while the form is open so starting Docker Desktop unlocks it.
    onMount(() => onCleanup(watchCapability("docker")));

    // Can the container's image be had? Asked once Docker answers; only a
    // definite refusal counts against the container.
    const [imageAccess, setImageAccess] = createSignal<ContainerImageState>(undefined);
    let imageCheckStarted = false;
    createEffect(() => {
        if (imageCheckStarted || !canPickContainer()) return;
        imageCheckStarted = true;
        void (async () => {
            try {
                const r = await RpcApi.ContainerImageCheckCommand(
                    TabRpcClient,
                    { image: props.template.container_image ?? "" },
                    { timeout: 10000 },
                );
                setImageAccess(r?.status ?? "unknown");
            } catch {
                setImageAccess("unknown");
            }
        })();
    });

    // Default the runtime once the probes land, never to a mode that can't
    // run, until the user chooses.
    createEffect(() => {
        if (draft.runtimeTouched) return;
        draft.setRuntime(
            shouldPreselectContainer({
                templateType: props.template.agent_type,
                containerSupported: containerSupported(),
                dockerAvailable: isAvailable("docker"),
                image: imageAccess(),
            })
                ? "container"
                : "host",
        );
    });
    const pickRuntime = (v: "host" | "container") => {
        draft.runtimeTouched = true;
        draft.setRuntime(v);
    };

    // Accounts and bundles. An empty account ("No auth") is allowed: the agent
    // can be created before any account exists, and the launch gate asks for
    // one before an oauth-class agent runs.
    onMount(() => {
        void (async () => {
            try {
                setAllAccounts(await refreshAccountCache());
            } catch {
                /* non-fatal; the agent can still be created without one */
            }
            setAccountsReady(true);
            try {
                setBundles((await RpcApi.ListBundlesCommand(TabRpcClient, {})) ?? []);
            } catch {
                /* non-fatal */
            }
        })();
    });

    // The provider's first account, re-picked whenever `accounts()` changes
    // (it depends on the async provider resolution, #2596). No account field:
    // it's changed later in the Stash (SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md
    // §3.6). System bundles are workspace policy, not a per-agent choice
    // (#2782).
    const realBundles = createMemo(() => bundles().filter((m) => !m.is_blank && !m.is_system));
    createEffect(() => {
        setAccountId(accounts()[0]?.id ?? "");
    });

    const canSubmit = () =>
        draft.name().trim().length > 0
        && draft.name().trim().length <= 200
        && accountsReady()
        && !resolvedTemplateProviderId.loading
        && !submitting();

    const submit = async () => {
        if (!canSubmit()) return;
        const name = draft.name().trim();
        const agentType = draft.runtime();
        const bundleIds = draft.bundleIds();
        const model = modelOptions().length > 0 ? draft.model() : "";
        setSubmitting(true);
        setError(null);
        try {
            const resp = await RpcApi.AgentDefCreateFromTemplateCommand(TabRpcClient, {
                template_id: props.template.id,
                name,
                identity_id: accountId(),
                // Only echoed back, for the launch's record of a bundle (first
                // pick); the agent still gets a fresh bundle of its own.
                memory_id: bundleIds[0] ?? "",
                bundle_ids: bundleIds,
                // Persisted on the new definition so later launches use it.
                agent_type: agentType,
                // Always sent: the form always has a concrete value.
                model_vendor_base_url: supportsCustomEndpoint() ? draft.modelVendorBaseUrl().trim() : "",
            });
            await props.onCreated({
                definitionId: resp.definition_id,
                identityId: resp.identity_id,
                memoryId: resp.memory_id,
                name,
                agentType,
                model,
                slug: resp.slug,
            });
        } catch (e) {
            setError(friendlySubmitError((e as Error)?.message ?? String(e), name));
        } finally {
            setSubmitting(false);
        }
    };

    // Enter in a single-line field submits once the form is valid.
    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
            e.preventDefault();
            void submit();
        }
    };

    return (
        <div class="agent-create-form" data-testid="agent-create-form">
            <Show when={modelOptions().length > 0}>
                <p class="agent-create-form-hint agent-new-bundle-modal-harness-hint">
                    {props.template.name} is the <strong>harness</strong>, the CLI tool that runs this agent. Pick which{" "}
                    <strong>model</strong> it uses below; the harness stays the same either way.
                </p>
            </Show>
            <label class="agent-new-bundle-modal-field">
                <span class="agent-new-bundle-modal-label">Name</span>
                <TextInput
                    class="agent-new-bundle-modal-input"
                    autofocus
                    placeholder="Choose a name"
                    value={draft.name()}
                    onInput={(e) => draft.setName(e.currentTarget.value)}
                    onKeyDown={onKeyDown}
                    maxlength={200}
                    disabled={submitting()}
                    data-testid="create-from-template-name-input"
                />
                <span class="agent-new-bundle-modal-hint">
                    Name it for its job, like Reviewer or Docs writer, so you can tell your agents apart.
                </span>
            </label>
            <label class="agent-new-bundle-modal-field">
                <span class="agent-new-bundle-modal-label">Runtime</span>
                <Select
                    class="agent-new-bundle-modal-input"
                    value={draft.runtime()}
                    onChange={(v) => pickRuntime(v as "host" | "container")}
                    disabled={submitting()}
                    data-testid="create-from-template-runtime-select"
                >
                    <option value="host">On this computer (host)</option>
                    <option value="container" disabled={!canPickContainer()}>
                        In a safe sandbox (container)
                        {canPickContainer() ? "" : " — Docker not detected"}
                    </option>
                </Select>
                <Show when={draft.runtime() === "host"}>
                    <span class="agent-new-bundle-modal-hint">
                        Runs directly on your machine with full access to your files, environment, and credentials.
                    </span>
                </Show>
                <Show when={draft.runtime() === "container"}>
                    <span class="agent-new-bundle-modal-hint">
                        Isolated Docker sandbox — the agent only sees its workspace.
                    </span>
                </Show>
                <Show when={containerSupported() && containerImageNote(imageAccess())}>
                    {(note) => (
                        <span class="agent-new-bundle-modal-hint" data-testid="create-from-template-image-note">
                            {note()}
                        </span>
                    )}
                </Show>
                <Show when={!containerSupported()}>
                    <span class="agent-new-bundle-modal-hint">{props.template.name} can only run on the host.</span>
                </Show>
            </label>
            <Show when={modelOptions().length > 0}>
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">Model</span>
                    <Select
                        class="agent-new-bundle-modal-input"
                        value={draft.model()}
                        onChange={(v) => pickModel(v)}
                        disabled={submitting()}
                        data-testid="create-from-template-model-select"
                    >
                        <For each={modelOptions()}>{(m) => <option value={m.value}>{m.label}</option>}</For>
                    </Select>
                    <span class="agent-new-bundle-modal-hint">
                        Which model {props.template.name} runs with. Changeable later from the agent pane's runtime
                        picker.
                    </span>
                </label>
            </Show>
            <Show when={supportsCustomEndpoint()}>
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">Model Vendor / Custom Endpoint</span>
                    <TextInput
                        class="agent-new-bundle-modal-input"
                        placeholder={`Default (${PROVIDERS[effectiveProviderId()]?.baseUrlEnvVar})`}
                        value={draft.modelVendorBaseUrl()}
                        onInput={(e) => draft.setModelVendorBaseUrl(e.currentTarget.value)}
                        onKeyDown={onKeyDown}
                        disabled={submitting()}
                        data-testid="create-from-template-vendor-base-url-input"
                    />
                    <span class="agent-new-bundle-modal-hint">
                        Redirect this agent's harness at a custom API endpoint (e.g. a proxy or alternate model
                        backend) instead of the default vendor. Leave blank to use the default.
                    </span>
                </label>
            </Show>
            {/* Several controls, so a group, not a <label>. Starts empty: the
                agent gets Global Memory and its own bundle. */}
            <div class="agent-new-bundle-modal-field" role="group" aria-labelledby="create-from-template-bundles-label">
                <span class="agent-new-bundle-modal-label" id="create-from-template-bundles-label">
                    Bundles
                </span>
                <BundleListEditor
                    bundles={realBundles()}
                    value={draft.bundleIds()}
                    onChange={draft.setBundleIds}
                    disabled={submitting()}
                    testId="create-from-template-bundles"
                />
                <span class="agent-new-bundle-modal-hint">
                    In order: when two bundles name the same skill, the first wins.
                </span>
            </div>
            <Show when={error()}>
                <div class="agent-new-bundle-modal-error" role="alert" data-testid="create-from-template-error">
                    {error()}
                </div>
            </Show>
            <div class="agent-create-actions">
                <Button tone="neutral" onClick={() => props.onCancel()} disabled={submitting()}>
                    Cancel
                </Button>
                <Button
                    tone="accent"
                    busy={submitting()}
                    onClick={() => void submit()}
                    disabled={!canSubmit()}
                    data-testid="create-from-template-submit"
                >
                    {submitting() ? "Creating…" : "Create and start"}
                </Button>
            </div>
        </div>
    );
};

AgentCreateForm.displayName = "AgentCreateForm";
