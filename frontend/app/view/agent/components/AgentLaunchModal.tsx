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
import type { Account } from "@/app/view/identity/identity-model";
import { loadAccounts } from "@/app/view/identity/identity-model";

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
    /** Per-instance identity overrides (issue #678 Phase 2). One entry
     *  per provider; absence means "use the definition default" (the
     *  backend falls back to db_forge_agent_identities). */
    identities?: { account_id: string; provider: string }[];
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

    // ── Identity picker (issue #678 Phase 2) ──────────────────────────
    // Per-provider account selection. `null` for a provider means "use
    // the definition default" (backend falls back to
    // db_forge_agent_identities). The map key is the provider id.
    const [identityChoice, setIdentityChoice] = createSignal<Record<string, string | null>>({});

    // Load the definition-level bindings so we can pre-fill the picker.
    // The list is small (one row per provider) so the call is cheap.
    const [agentBindings] = createResource(
        () => props.agent.id,
        async (agentId) => {
            try {
                return await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, { agent_id: agentId });
            } catch {
                return [] as ForgeAgentIdentity[];
            }
        },
    );

    const allAccounts = createMemo<Account[]>(() => loadAccounts());

    // Group accounts by provider — drives the picker rows. We only
    // render rows for providers that have at least one account; users
    // who want a provider that has no accounts go to the identity
    // widget to add one.
    const accountsByProvider = createMemo<Map<string, Account[]>>(() => {
        const m = new Map<string, Account[]>();
        for (const a of allAccounts()) {
            if (!m.has(a.provider)) m.set(a.provider, []);
            m.get(a.provider)!.push(a);
        }
        return m;
    });

    // Pre-fill identityChoice from agent bindings whenever they load.
    // We only initialize entries we haven't already touched (so the
    // resource refresh doesn't clobber a user's pick).
    createMemo(() => {
        const bindings = agentBindings();
        if (!bindings) return;
        const current = identityChoice();
        const next: Record<string, string | null> = { ...current };
        let changed = false;
        for (const b of bindings) {
            if (next[b.provider] === undefined) {
                next[b.provider] = b.account_id;
                changed = true;
            }
        }
        if (changed) setIdentityChoice(next);
    });

    /** Build the LaunchOverrides.identities array from the picker state. */
    const collectIdentities = (): { account_id: string; provider: string }[] => {
        const choice = identityChoice();
        const out: { account_id: string; provider: string }[] = [];
        for (const [provider, account_id] of Object.entries(choice)) {
            if (account_id) out.push({ account_id, provider });
        }
        return out;
    };

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
                identities: collectIdentities(),
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

                    <Show when={accountsByProvider().size > 0}>
                        <fieldset class="agent-launch-modal-field agent-launch-modal-identities">
                            <legend class="agent-launch-modal-label">Identity</legend>
                            <span class="agent-launch-modal-hint">
                                Credentials to inject when this instance launches. Pre-filled
                                from the agent's defaults; override per-launch as needed.
                            </span>
                            <For each={Array.from(accountsByProvider().entries())}>
                                {([provider, accounts]) => (
                                    <div class="agent-launch-modal-identity-row">
                                        <label class="agent-launch-modal-identity-provider">
                                            {provider}
                                        </label>
                                        <select
                                            class="agent-launch-modal-input"
                                            value={identityChoice()[provider] ?? ""}
                                            onChange={(e) => {
                                                const v = e.currentTarget.value;
                                                setIdentityChoice((prev) => ({
                                                    ...prev,
                                                    [provider]: v === "" ? null : v,
                                                }));
                                            }}
                                            disabled={submitting()}
                                            aria-label={`Identity for ${provider}`}
                                        >
                                            <option value="">— None (use ambient) —</option>
                                            <For each={accounts}>
                                                {(acc) => (
                                                    <option value={acc.id}>
                                                        {acc.display_name?.trim() || acc.name}
                                                    </option>
                                                )}
                                            </For>
                                        </select>
                                    </div>
                                )}
                            </For>
                        </fieldset>
                    </Show>

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
