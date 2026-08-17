// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCreateFromTemplateModalPanel — Phase 1 two-tier picker modal
 * (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md).
 *
 * Opens when the user clicks a card in the picker's Templates section.
 * Collects a name + identity + memory, then in `onSubmit` the layer
 * clones the seeded template into a new user-owned definition via
 * `agentdefcreatefromtemplate`, immediately launches it with the
 * picked bindings, and closes.
 *
 * Why a separate modal (not a flag on AgentLaunchModal): the launch
 * modal carries a lot of additional UX (runtime/container picker,
 * continuation dropdown, OAuth pre-launch panel, "+ New bundle"
 * affordances, NamedAgents resource). For a template-create the
 * minimum surface is name + bindings; mixing it into the launch modal
 * would push its complexity onto a flow that doesn't need it. The
 * canonical modal panel styles (`modal-panel-*`) and shared bundle
 * styles (`agent-new-bundle-modal-*`) are reused so this fits the
 * universal modal system per `feedback_use_universal_modal_system`.
 */

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { PROVIDERS } from "../providers/catalog";
import { isAvailable, watchCapability } from "@/app/store/toolchain-capabilities";
import { refreshAccountCache, type Account } from "@/app/view/identity/identity-model";

interface CreateFromTemplateFormData {
    name: string;
    accountId: string;
    memoryId: string;
    /** Where the new agent runs. Chosen here at instantiation time —
     *  it is NOT a property of the template (a template is runtime-
     *  agnostic). "container" requires a reachable Docker runtime. */
    agentType: "host" | "container";
    /** Custom model vendor base URL override (e.g. a proxy in front of
     *  Anthropic's API). Empty string = use the harness's default vendor
     *  endpoint. Only meaningful — and only shown in the form — when the
     *  template's provider declares `baseUrlEnvVar` in the catalog. */
    modelVendorBaseUrl: string;
    /** Initial model choice — a value from the template's harness's own
     *  `models` list (e.g. "opus" for claude, "gpt-5.5" for codex). Empty
     *  string when the harness declares no models list; the launch path
     *  falls back to that harness's own default. Distinct from
     *  `modelVendorBaseUrl` above: this picks WHICH model the harness
     *  runs, not WHO serves it. */
    model: string;
}

interface AgentCreateFromTemplateModalPanelProps {
    /** Seeded template the user clicked. */
    template: AgentDefinition;
    /** Suggested initial name (defaults to template name). */
    initialName?: string;
    /** Called when the user clicks Create with valid data. The layer
     *  wraps this with the create-then-launch RPC chain. */
    onSubmit: (formData: CreateFromTemplateFormData) => Promise<void>;
    onCancel: () => void;
}

export const AgentCreateFromTemplateModalPanel = (
    props: AgentCreateFromTemplateModalPanelProps,
): JSX.Element => {
    const [name, setName] = createSignal(props.initialName ?? props.template.name);
    const [accountId, setAccountId] = createSignal("");
    const [memoryId, setMemoryId] = createSignal("");
    const [accounts, setAccounts] = createSignal<Account[]>([]);
    const [memories, setMemories] = createSignal<Memory[]>([]);
    const [submitting, setSubmitting] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const [modelVendorBaseUrl, setModelVendorBaseUrl] = createSignal(
        props.template.model_vendor_base_url ?? "",
    );

    // Only providers that declare `baseUrlEnvVar` (currently just claude)
    // can actually be redirected to a custom endpoint — see
    // agent_define::validate_vendor_base_url on the backend, which rejects
    // a non-empty override for any other provider.
    const supportsCustomEndpoint = createMemo(
        () => !!PROVIDERS[props.template.provider]?.baseUrlEnvVar,
    );

    // ── Model (which model this harness runs) ────────────────────────
    // The template card you clicked already picked the HARNESS (the CLI
    // — "Claude Code", "Codex", etc.); this picks WHICH MODEL that
    // harness runs, from its own model list (same list
    // AgentRuntimeDropup's in-session picker reads via
    // getProvider(id)?.models). Hidden entirely when the harness
    // declares no models list — nothing to choose between.
    const modelOptions = createMemo(() => PROVIDERS[props.template.provider]?.models ?? []);
    const [model, setModel] = createSignal(
        modelOptions().find((m) => m.default)?.value ?? modelOptions()[0]?.value ?? "",
    );

    // ── Runtime (host vs container) ────────────────────────────────
    // Runtime is decided HERE, when instantiating the template — it's
    // not a property of the template. The container option is only
    // offered when (a) the CLI can run in a container and (b) Docker is
    // actually reachable on this machine. We default to host so the new
    // agent always actually starts; container can never be the silent
    // default on a box without Docker (the bug this fixes).
    const [runtime, setRuntime] = createSignal<"host" | "container">("host");
    // Once the user touches the dropdown we stop auto-defaulting so a
    // Docker-availability change can't yank their choice out from under them.
    let runtimeTouched = false;

    const containerSupported = createMemo(
        () => getCliCatalogEntry(props.template.provider)?.containerSupported ?? true,
    );
    // Reads the shared toolchain-capabilities store rather than probing
    // Docker itself — this is what guarantees this modal can never disagree
    // with the Toolchain widget or the launch pre-flight check about whether
    // Docker is actually available. That store's "docker" entry is checked
    // via a DAEMON ping (ContainerRuntimeAvailableCommand), not just whether
    // the CLI binary is on PATH — a binary-only check would false-positive
    // when Docker is installed but the daemon is stopped, re-creating the
    // exact trap this modal exists to avoid: defaulting to a container agent
    // that then can't start (codex P1 on #1576). See
    // docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
    const canPickContainer = () => containerSupported() && isAvailable("docker");

    // Poll while this modal is open so a user who starts Docker Desktop
    // mid-flow sees the option unlock within a few seconds, no restart.
    onMount(() => onCleanup(watchCapability("docker")));

    // Default-pick the runtime once the probe lands: honour the
    // template's suggested mode when container is genuinely usable,
    // otherwise fall back to host. Never auto-selects a mode that can't
    // run. Skipped once the user has made an explicit choice.
    createEffect(() => {
        if (runtimeTouched) return;
        setRuntime(
            canPickContainer() && props.template.agent_type === "container" ? "container" : "host",
        );
    });

    const pickRuntime = (v: "host" | "container") => {
        runtimeTouched = true;
        setRuntime(v);
    };

    // Load account + memory lists on mount. The account list is
    // filtered to the template's own provider (accounts are provider-
    // specific — issue #1624 PR-C Part B). Bindings are stored on the
    // db_agent_instances row at launch time; the empty string sentinel
    // means "ambient creds / vanilla CLI" so an empty selection is OK.
    onMount(() => {
        void (async () => {
            try {
                const list = await refreshAccountCache();
                setAccounts(list.filter((a) => a.provider === props.template.provider));
            } catch {
                /* non-fatal; user can still create without binding */
            }
            try {
                const list = await RpcApi.ListMemoriesCommand(TabRpcClient, {});
                setMemories(list ?? []);
            } catch {
                /* non-fatal */
            }
        })();
    });

    // Default-pick the first available account once the list lands,
    // matching the launch modal's UX (saves a click for users with
    // existing accounts). `is_blank` rows are a bundle-only concept —
    // accounts have no such thing.
    const realMemories = createMemo(() => memories().filter((m) => !m.is_blank));
    createEffect(() => {
        if (accountId()) return;
        const first = accounts()[0];
        if (first) setAccountId(first.id);
    });
    createEffect(() => {
        if (memoryId()) return;
        const first = realMemories()[0];
        if (first) setMemoryId(first.id);
    });

    const canSubmit = () =>
        name().trim().length > 0
        && name().trim().length <= 200
        && !submitting();

    const submit = async () => {
        if (!canSubmit()) return;
        setSubmitting(true);
        setError(null);
        try {
            await props.onSubmit({
                name: name().trim(),
                accountId: accountId(),
                memoryId: memoryId(),
                agentType: runtime(),
                modelVendorBaseUrl: supportsCustomEndpoint() ? modelVendorBaseUrl().trim() : "",
                model: modelOptions().length > 0 ? model() : "",
            });
            // Layer unmounts via close-on-success. Reset is defensive.
            setSubmitting(false);
        } catch (e) {
            setError((e as Error)?.message ?? String(e));
            setSubmitting(false);
        }
    };

    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            void submit();
        }
    };

    return (
        <>
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">Create new agent from {props.template.name}</h2>
                <p class="modal-panel-description">
                    A new user-owned agent will be cloned from this template.
                    The template stays untouched and can be used again.
                </p>
                <Show when={modelOptions().length > 0}>
                    <p class="modal-panel-description agent-new-bundle-modal-harness-hint">
                        {props.template.name} is the <strong>harness</strong> — the CLI tool
                        that runs this agent. Pick which <strong>model</strong> it uses below;
                        the harness stays the same either way.
                    </p>
                </Show>
            </header>
            <div class="modal-panel-body agent-new-bundle-modal-body">
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">Name</span>
                    <input
                        type="text"
                        class="agent-new-bundle-modal-input"
                        autofocus
                        placeholder={props.template.name}
                        value={name()}
                        onInput={(e) => setName(e.currentTarget.value)}
                        onKeyDown={onKeyDown}
                        maxlength={200}
                        disabled={submitting()}
                        data-testid="create-from-template-name-input"
                    />
                </label>
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">Runtime</span>
                    <select
                        class="agent-new-bundle-modal-input"
                        value={runtime()}
                        onChange={(e) =>
                            pickRuntime(e.currentTarget.value as "host" | "container")}
                        disabled={submitting()}
                        data-testid="create-from-template-runtime-select"
                    >
                        <option value="host">On this computer (host)</option>
                        <option value="container" disabled={!canPickContainer()}>
                            In a safe sandbox (container)
                            {canPickContainer() ? "" : " — Docker not detected"}
                        </option>
                    </select>
                    <Show when={runtime() === "host"}>
                        <span class="agent-new-bundle-modal-hint">
                            Runs directly on your machine with full access to your files,
                            environment, and credentials.
                        </span>
                    </Show>
                    <Show when={runtime() === "container"}>
                        <span class="agent-new-bundle-modal-hint">
                            Isolated Docker sandbox — the agent only sees its workspace.
                        </span>
                    </Show>
                    <Show when={!containerSupported()}>
                        <span class="agent-new-bundle-modal-hint">
                            {props.template.name} can only run on the host.
                        </span>
                    </Show>
                </label>
                <Show when={modelOptions().length > 0}>
                    <label class="agent-new-bundle-modal-field">
                        <span class="agent-new-bundle-modal-label">Model</span>
                        <select
                            class="agent-new-bundle-modal-input"
                            value={model()}
                            onChange={(e) => setModel(e.currentTarget.value)}
                            disabled={submitting()}
                            data-testid="create-from-template-model-select"
                        >
                            <For each={modelOptions()}>
                                {(m) => <option value={m.value}>{m.label}</option>}
                            </For>
                        </select>
                        <span class="agent-new-bundle-modal-hint">
                            Which model {props.template.name} runs with. Changeable later
                            from the agent pane's runtime picker.
                        </span>
                    </label>
                </Show>
                <Show when={supportsCustomEndpoint()}>
                    <label class="agent-new-bundle-modal-field">
                        <span class="agent-new-bundle-modal-label">Model Vendor / Custom Endpoint</span>
                        <input
                            type="text"
                            class="agent-new-bundle-modal-input"
                            placeholder={`Default (${PROVIDERS[props.template.provider]?.baseUrlEnvVar})`}
                            value={modelVendorBaseUrl()}
                            onInput={(e) => setModelVendorBaseUrl(e.currentTarget.value)}
                            onKeyDown={onKeyDown}
                            disabled={submitting()}
                            data-testid="create-from-template-vendor-base-url-input"
                        />
                        <span class="agent-new-bundle-modal-hint">
                            Redirect this agent's harness at a custom API endpoint
                            (e.g. a proxy or alternate model backend) instead of the
                            default vendor. Leave blank to use the default.
                        </span>
                    </label>
                </Show>
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">Identity</span>
                    <select
                        class="agent-new-bundle-modal-input"
                        value={accountId()}
                        onChange={(e) => setAccountId(e.currentTarget.value)}
                        disabled={submitting()}
                        data-testid="create-from-template-identity-select"
                    >
                        <option value="">(ambient credentials)</option>
                        <For each={accounts()}>
                            {(a) => (
                                <option value={a.id}>{a.display_name?.trim() || a.name}</option>
                            )}
                        </For>
                    </select>
                </label>
                <label class="agent-new-bundle-modal-field">
                    <span class="agent-new-bundle-modal-label">Memory</span>
                    <select
                        class="agent-new-bundle-modal-input"
                        value={memoryId()}
                        onChange={(e) => setMemoryId(e.currentTarget.value)}
                        disabled={submitting()}
                        data-testid="create-from-template-memory-select"
                    >
                        <option value="">(vanilla CLI)</option>
                        <For each={realMemories()}>
                            {(m) => <option value={m.id}>{m.name}</option>}
                        </For>
                    </select>
                </label>
                <Show when={error()}>
                    <div class="agent-new-bundle-modal-error" data-testid="create-from-template-error">
                        {error()}
                    </div>
                </Show>
            </div>
            <footer class="modal-panel-footer">
                <Button onClick={() => props.onCancel()} disabled={submitting()} data-modal-dismiss>
                    Cancel
                </Button>
                <Button
                    onClick={() => void submit()}
                    className="green solid"
                    disabled={!canSubmit()}
                    data-testid="create-from-template-submit"
                >
                    {submitting() ? "Creating…" : "Create"}
                </Button>
            </footer>
        </>
    );
};

AgentCreateFromTemplateModalPanel.displayName = "AgentCreateFromTemplateModalPanel";
