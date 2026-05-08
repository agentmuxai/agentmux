// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentLaunchModalPanel — the form rendered inside `<TabModalLayer>`
 * when the user clicks a definition card in the agent picker. Collects
 * the instance name + runtime (host vs container) and submits them to
 * the caller, which is responsible for calling launchForgeAgent with
 * the overrides.
 *
 * No Portal, no Modal v2 wrapper — the layer owns positioning, backdrop,
 * ESC, and backdrop-click semantics. This file contributes the form
 * panel only. See docs/specs/launch-modal-rearchitecture-2026-05-01.md.
 */

import { createMemo, createResource, createSignal, For, Show, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { buildInstanceSlug, slugifyInstanceName } from "../defaults/instance-slug";

export interface LaunchOverrides {
    /** Instance name — written into AGENTMUX_AGENT_ID and used to
     *  derive the working directory. */
    instanceName: string;
    /** "host" runs directly on the OS. "container" runs inside
     *  Docker/Podman. */
    agentType: "host" | "container";
    /** "local" pairs with "host"; "docker" pairs with "container". */
    environment: "local" | "docker";
    /** Only set when agentType === "container". */
    containerImage?: string;
    /** v7 — selected Identity bundle. "blank" = use ambient creds. */
    identityId?: string;
    /** v7 — selected Memory bundle. "blank" = vanilla CLI. */
    memoryId?: string;
}

interface AgentLaunchModalPanelProps {
    agent: ForgeAgent;
    onCancel: () => void;
    onSubmit: (overrides: LaunchOverrides) => Promise<void> | void;
}

export const AgentLaunchModalPanel = (props: AgentLaunchModalPanelProps): JSX.Element => {
    const catalog = createMemo(() => getCliCatalogEntry(props.agent.provider));
    const displayName = () => catalog()?.displayName ?? props.agent.name;

    const [name, setName] = createSignal("");
    const [runtime, setRuntime] = createSignal<"host" | "container">("host");
    const [image, setImage] = createSignal<string>("");
    const [submitting, setSubmitting] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const [showAdvanced, setShowAdvanced] = createSignal(false);

    // v7 — Identity + Memory bundle pickers. Both default to "blank"
    // which the resolver short-circuits as "no override". Lists fetched
    // once at mount; the blank singleton is always present. See
    // docs/specs/identity-forge-integration-and-vault-2026-05-08.md.
    const [identityId, setIdentityId] = createSignal<string>("blank");
    const [memoryId, setMemoryId] = createSignal<string>("blank");

    const [identities] = createResource<IdentityBundle[]>(async () => {
        try {
            return await RpcApi.ListIdentityBundlesCommand(TabRpcClient, {});
        } catch {
            return [];
        }
    });
    const [memories] = createResource<Memory[]>(async () => {
        try {
            return await RpcApi.ListMemoriesCommand(TabRpcClient, {});
        } catch {
            return [];
        }
    });

    const hasName = () => name().trim().length > 0;
    const canSubmit = () => !submitting() && slugifyInstanceName(name()).length > 0;
    const containerSupported = () => catalog()?.containerSupported ?? true;

    const resolvedImage = () => {
        const v = image().trim();
        if (v) return v;
        return catalog()?.containerImage ?? "";
    };

    const handleSubmit = async () => {
        if (!canSubmit()) return;
        setSubmitting(true);
        setError(null);
        try {
            await props.onSubmit({
                instanceName: name().trim(),
                agentType: runtime(),
                environment: runtime() === "container" ? "docker" : "local",
                containerImage: runtime() === "container" ? resolvedImage() : undefined,
                identityId: identityId(),
                memoryId: memoryId(),
            });
            // Success: layer closes the panel; we leave `submitting`
            // true so the button keeps its "Launching…" label until
            // unmount.
        } catch (e: any) {
            setError(String(e?.message ?? e));
            setSubmitting(false);
        }
    };

    // Enter submits; ESC and backdrop click are handled by the layer.
    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter" && canSubmit()) {
            e.preventDefault();
            void handleSubmit();
        }
    };

    return (
        <>
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">Launch {displayName()}</h2>
            </header>
            <div class="modal-panel-body">
                <div class="agent-launch-modal-body" onKeyDown={handleKeyDown}>
                    <Show when={catalog()}>
                        <p class="agent-launch-modal-blurb">
                            {catalog()?.popoverMarkdown}
                        </p>
                    </Show>

                    <label class="agent-launch-modal-field">
                        <span class="agent-launch-modal-label">Give this agent a name</span>
                        <input
                            class="agent-launch-modal-input"
                            type="text"
                            maxLength={64}
                            placeholder={displayName()}
                            value={name()}
                            onInput={(e) => setName(e.currentTarget.value)}
                            disabled={submitting()}
                            aria-label="Agent name"
                            // Autofocus so the user can start typing immediately.
                            // Layer renders us inside its panel; the focus is
                            // contained because the dimmed content beneath is
                            // marked `inert`.
                            // eslint-disable-next-line jsx-a11y/no-autofocus
                            autofocus
                        />
                        <span class="agent-launch-modal-hint">
                            So you can tell it apart from other agents. 1–64 characters.
                        </span>
                    </label>

                    <fieldset class="agent-launch-modal-field agent-launch-modal-runtime">
                        <legend class="agent-launch-modal-label">Where should it run?</legend>
                        <label class="agent-launch-modal-radio">
                            <input
                                type="radio"
                                name="agent-launch-runtime"
                                checked={runtime() === "host"}
                                onChange={() => setRuntime("host")}
                                disabled={submitting()}
                            />
                            <span>
                                <strong>On this computer</strong>
                                <span class="agent-launch-modal-hint">
                                    Fastest. The agent can read and change files on your machine.
                                </span>
                            </span>
                        </label>
                        <label
                            class="agent-launch-modal-radio"
                            classList={{ "agent-launch-modal-radio--disabled": !containerSupported() }}
                        >
                            <input
                                type="radio"
                                name="agent-launch-runtime"
                                checked={runtime() === "container"}
                                onChange={() => setRuntime("container")}
                                disabled={submitting() || !containerSupported()}
                            />
                            <span>
                                <strong>In a safe sandbox</strong>
                                <span class="agent-launch-modal-hint">
                                    {containerSupported()
                                        ? "Slower to start, but the agent can't touch files outside its own workspace. Recommended for untrusted tasks."
                                        : "Not available for this agent."}
                                </span>
                            </span>
                        </label>
                    </fieldset>

                    <fieldset class="agent-launch-modal-field agent-launch-modal-bundles">
                        <legend class="agent-launch-modal-label">Identity</legend>
                        <span class="agent-launch-modal-hint">
                            Identity bundles credentials per provider. Pick a bundle to
                            inject its accounts as env vars at launch (e.g. GITHUB_TOKEN,
                            ANTHROPIC_API_KEY); pick blank to use whatever's already in
                            your environment.
                        </span>

                        <label class="agent-launch-modal-bundle-row">
                            <span class="agent-launch-modal-bundle-row-label">Identity</span>
                            <select
                                class="agent-launch-modal-input"
                                value={identityId()}
                                onChange={(e) => setIdentityId(e.currentTarget.value)}
                                disabled={submitting()}
                                aria-label="Identity bundle"
                            >
                                <For each={identities() ?? []}>
                                    {(bundle) => (
                                        <option value={bundle.id}>
                                            {bundle.is_blank
                                                ? "— Blank (no creds) —"
                                                : bundle.name}
                                        </option>
                                    )}
                                </For>
                            </select>
                        </label>

                        {/*
                         * Memory dropdown is parked here pending the
                         * spawn-time content-injection layer (provider
                         * override, instructions, context files, MCP
                         * servers, skills). Until that lands, picking
                         * a non-blank Memory would be cosmetic — codex
                         * P2 on PR #751 caught this. The state hooks
                         * below stay in place; memoryId is forced to
                         * "blank" on the wire so the backend writes a
                         * blank reference. The Memory pane is still
                         * usable for managing bundles; the launch
                         * picker comes back in PR-F.4.
                         */}

                        <label class="agent-launch-modal-bundle-row" style={{ display: "none" }}>
                            <span class="agent-launch-modal-bundle-row-label">Memory</span>
                            <select
                                class="agent-launch-modal-input"
                                value={memoryId()}
                                onChange={(e) => setMemoryId(e.currentTarget.value)}
                                disabled={submitting()}
                                aria-label="Memory bundle"
                            >
                                <For each={memories() ?? []}>
                                    {(memory) => (
                                        <option value={memory.id}>
                                            {memory.is_blank
                                                ? "— Blank (vanilla CLI) —"
                                                : memory.name}
                                        </option>
                                    )}
                                </For>
                            </select>
                        </label>
                    </fieldset>

                    <Show when={error()}>
                        <div class="agent-launch-modal-error">{error()}</div>
                    </Show>

                    <details
                        class="agent-launch-modal-advanced"
                        open={showAdvanced()}
                        onToggle={(e) => setShowAdvanced(e.currentTarget.open)}
                    >
                        <summary class="agent-launch-modal-advanced-summary">
                            Advanced options
                        </summary>
                        <div class="agent-launch-modal-advanced-body">
                            <label
                                class="agent-launch-modal-field"
                                classList={{ "agent-launch-modal-field--disabled": runtime() !== "container" }}
                            >
                                <span class="agent-launch-modal-label">Override sandbox base</span>
                                <input
                                    class="agent-launch-modal-input"
                                    type="text"
                                    placeholder={catalog()?.containerImage ?? ""}
                                    value={image()}
                                    onInput={(e) => setImage(e.currentTarget.value)}
                                    disabled={submitting() || runtime() !== "container" || !containerSupported()}
                                    aria-label="Sandbox base image"
                                />
                                <span class="agent-launch-modal-hint">
                                    {runtime() === "container"
                                        ? "Leave blank unless you know exactly which base image you need."
                                        : "Only applies to the sandbox runtime."}
                                </span>
                            </label>

                            <Show when={hasName()}>
                                <div class="agent-launch-modal-preview">
                                    <span class="agent-launch-modal-preview-label">Its files will live in</span>
                                    <code>{buildInstanceSlug(name().trim())}</code>
                                </div>
                            </Show>
                        </div>
                    </details>
                </div>
            </div>
            <footer class="modal-panel-footer">
                <Button onClick={props.onCancel} disabled={submitting()}>
                    Cancel
                </Button>
                <Button onClick={() => void handleSubmit()} disabled={!canSubmit()}>
                    {submitting() ? "Launching…" : "Launch"}
                </Button>
            </footer>
        </>
    );
};

AgentLaunchModalPanel.displayName = "AgentLaunchModalPanel";
