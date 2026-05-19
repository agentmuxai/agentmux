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
import { getProvider } from "../providers";
import { PreLaunchAuthPanel } from "./PreLaunchAuthPanel";
import type { AuthState } from "../auth";

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
    /** v8 — when set, this launch is a continuation of a prior named
     *  agent. The id is recorded as `parent_instance_id` on the new
     *  row so the lineage is queryable. */
    continueOfInstanceId?: string;
    /** v8 — when set (paired with `continueOfInstanceId`), the
     *  launch flow uses this exact path instead of calling
     *  `allocate_agent_workdir`. Reuses the prior agent's files,
     *  configs, and conversation context. */
    workDirOverride?: string;
}

interface AgentLaunchModalPanelProps {
    agent: ForgeAgent;
    onCancel: () => void;
    onSubmit: (overrides: LaunchOverrides) => Promise<void> | void;
    /** Auto-select this Identity bundle on mount. Used by the
     *  "+ New" → create → replace-back flow so the freshly-created
     *  bundle shows up as the picker's choice. */
    preselectedIdentityId?: string;
    /** Same for Memory. Phase γ. */
    preselectedMemoryId?: string;
    /** Called when the "+" / empty-state New buttons fire. The picker
     *  is expected to chain `tabModal.replace(newIdentityRequest)` —
     *  this component doesn't know how to build that request. */
    onRequestNewIdentity?: () => void;
    onRequestNewMemory?: () => void;
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
    const [identityId, setIdentityId] = createSignal<string>(
        props.preselectedIdentityId ?? "blank",
    );
    const [memoryId, setMemoryId] = createSignal<string>(
        props.preselectedMemoryId ?? "blank",
    );

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

    // Empty-state predicate — the backend always returns the implicit
    // "blank" singleton, so an empty-of-user-bundles list still has
    // length 1. Treat any list where every entry is `is_blank` as
    // empty for the "show only New button" branch.
    const hasUserIdentities = createMemo(() =>
        (identities() ?? []).some((b) => !b.is_blank),
    );
    const hasUserMemories = createMemo(() =>
        (memories() ?? []).some((m) => !m.is_blank),
    );

    // "+ New ..." buttons delegate to picker-injected callbacks that
    // chain `tabModal.replace(newBundleRequest)`. The picker owns the
    // chain because it can rebuild the Launch request with
    // preselectedIdentityId/preselectedMemoryId after creation. A
    // missing callback (Phase β ships Identity wiring only; Memory
    // wiring lands in Phase γ) keeps the button visible but disabled
    // with a "coming soon" hint — see reagent P2 on PR #910.
    const handleNewIdentity = () => props.onRequestNewIdentity?.();
    const handleNewMemory = () => props.onRequestNewMemory?.();

    // v8 — "Continue agent" dropdown. Filters to instances of the
    // CURRENT definition (server-side; a global cap would let older
    // rows of this definition fall off when users have many agents
    // across definitions). Empty list = no past launches for this
    // definition, dropdown hides itself.
    const [namedAgents] = createResource<NamedAgentRow[]>(async () => {
        try {
            return await RpcApi.ListNamedAgentsCommand(TabRpcClient, {
                limit: 200,
                definition_id: props.agent.id,
            });
        } catch {
            return [];
        }
    });

    /** Empty = "— New agent —" (default). Non-empty = continuing that
     *  past instance; name + identity + memory are pre-filled and
     *  locked. */
    const [continueOfId, setContinueOfId] = createSignal<string>("");

    const continuedRow = createMemo(() =>
        (namedAgents() ?? []).find((r) => r.instance_id === continueOfId()) ?? null,
    );
    const isContinue = () => continuedRow() != null;

    const handleContinueSelect = (id: string) => {
        setContinueOfId(id);
        const row = (namedAgents() ?? []).find((r) => r.instance_id === id);
        if (row) {
            setName(row.instance_name);
            setIdentityId(row.identity_id || "blank");
            setMemoryId(row.memory_id || "blank");
        } else {
            // Selecting "— New agent —" releases the lock. Clear the
            // fields back to defaults so the user doesn't accidentally
            // submit a brand-new launch carrying the previously
            // continued agent's name + bundles (which would collide
            // on allocate_agent_workdir and inject the wrong creds).
            setName("");
            setIdentityId("blank");
            setMemoryId("blank");
        }
    };

    const formatRelative = (ms: number): string => {
        if (!ms) return "";
        const delta = Date.now() - ms;
        if (delta < 60_000) return "just now";
        if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`;
        if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`;
        return `${Math.floor(delta / 86_400_000)}d ago`;
    };

    const hasName = () => name().trim().length > 0;
    const containerSupported = () => catalog()?.containerSupported ?? true;

    // Pre-launch OAuth (spec: SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md).
    // The PreLaunchAuthPanel owns the AuthFlowController; we mirror
    // its state.kind here to gate the Launch button. Existing
    // non-blank bundles bypass the gate — they came from a prior
    // OAuth, so trust them (PR B-4 / PR D will tighten this with a
    // per-bundle binding check).
    const [authStateKind, setAuthStateKind] = createSignal<AuthState["kind"]>("idle");
    const onAuthStateChange = (s: AuthState) => setAuthStateKind(s.kind);
    const onBundleCreated = (bundleId: string) => {
        // The new bundle was just persisted backend-side. Force the
        // resource to refetch and switch the dropdown to it.
        // `createResource.refetch()` isn't exposed by destructure — we
        // re-trigger by re-mounting the resource via a key signal.
        // Lightweight alternative: optimistically set the dropdown
        // and let the next refetch (on dropdown change or modal
        // reopen) load the row. The bundle id alone is what the
        // launch payload needs.
        //
        // Codex P2 on #847 round 9: defense-in-depth — `PreLaunchAuthPanel`
        // already filters `pending-bundle-for-...` placeholders before
        // invoking this callback, but guard here too so any future
        // call site that bypasses the panel filter can't poison the
        // dropdown with a non-existent bundle row.
        if (!bundleId || bundleId === "blank" || bundleId.startsWith("pending-bundle-for-")) {
            return;
        }
        setIdentityId(bundleId);
    };
    const provider = createMemo(() => getProvider(props.agent.provider));
    // Auth gate applies ONLY to fresh launches of OAuth providers with
    // the blank singleton selected.
    //
    // - `isContinue` bypasses: prior launch already produced creds.
    // - API-key providers (kimi/pi) bypass: until the backend
    //   `auth.submitapikey` persists bundles (PR C-2), the gate would
    //   deadlock — the user can't reach `ready` because save is a
    //   stub. Their existing `launch-flow.ts` Phase 2 prompts for the
    //   key in-line. Reagent + codex P1 on #847.
    // - OpenClaw: even with a non-blank identity selected, no identity
    //   has been openclaw-authed yet (Phase α addition, 2026-05-17).
    //   Until identity bundles include openclaw auth profiles, gate
    //   ALWAYS — the user needs to run the OpenAI OAuth flow before
    //   `openclaw acp` will spawn. Lifts once identity-bundles-include-
    //   openclaw lands (planned with Phase δ persistence work).
    const authRequired = () =>
        !isContinue()
        && provider()?.authType === "oauth"
        && (
            identityId() === "blank"
            || identityId() === ""
            || provider()?.id === "openclaw"
        );
    const authReady = () => !authRequired() || authStateKind() === "ready";
    const canSubmit = () =>
        !submitting() && slugifyInstanceName(name()).length > 0 && authReady();

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
            const row = continuedRow();
            await props.onSubmit({
                instanceName: name().trim(),
                agentType: runtime(),
                environment: runtime() === "container" ? "docker" : "local",
                containerImage: runtime() === "container" ? resolvedImage() : undefined,
                identityId: identityId(),
                memoryId: memoryId(),
                // v8 — when continuing a past agent, thread the id +
                // working directory through. Launch flow uses
                // workDirOverride to skip allocate_agent_workdir.
                continueOfInstanceId: row?.instance_id || undefined,
                workDirOverride: row?.working_directory || undefined,
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
    // Skip when focus is on a button or select so Enter triggers the
    // browser's native button activation / dropdown open, not the form
    // submit. Reagent P1 on PR #909 — without this guard, pressing
    // Enter on the "+ New identity/memory" button submits the launch
    // instead of opening the bundle-creation modal.
    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key !== "Enter" || !canSubmit()) return;
        const target = e.target as HTMLElement | null;
        const tag = target?.tagName;
        if (tag === "BUTTON" || tag === "SELECT") return;
        e.preventDefault();
        void handleSubmit();
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

                    {/* v8 — "Continue agent" dropdown. Only shows if
                        there are past named agents of this definition.
                        Picking one pre-fills + locks name/identity/
                        memory; the launch becomes a continuation that
                        reuses the prior working directory. */}
                    <Show when={(namedAgents() ?? []).length > 0}>
                        <label class="agent-launch-modal-field">
                            <span class="agent-launch-modal-label">Continue an existing agent</span>
                            <select
                                class="agent-launch-modal-input"
                                value={continueOfId()}
                                onChange={(e) => handleContinueSelect(e.currentTarget.value)}
                                disabled={submitting()}
                                aria-label="Continue an existing agent"
                            >
                                <option value="">— New agent —</option>
                                <For each={namedAgents() ?? []}>
                                    {(row) => {
                                        const parts = [
                                            row.instance_name,
                                            row.identity_name?.trim() || "(ambient creds)",
                                            row.memory_name?.trim() || "(vanilla CLI)",
                                        ];
                                        if (row.started_at) parts.push(formatRelative(row.started_at));
                                        return (
                                            <option value={row.instance_id}>
                                                {parts.join(" · ")}
                                            </option>
                                        );
                                    }}
                                </For>
                            </select>
                            <span class="agent-launch-modal-hint">
                                Picks up where the agent left off — same files,
                                same identity, same memory.
                            </span>
                        </label>
                    </Show>

                    <label class="agent-launch-modal-field">
                        <span class="agent-launch-modal-label">
                            {isContinue() ? "Agent name" : "Give this agent a name"}
                        </span>
                        <input
                            class="agent-launch-modal-input"
                            type="text"
                            maxLength={64}
                            placeholder={displayName()}
                            value={name()}
                            onInput={(e) => setName(e.currentTarget.value)}
                            disabled={submitting() || isContinue()}
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
                        <legend class="agent-launch-modal-label">Profile</legend>
                        <span class="agent-launch-modal-hint">
                            A Profile groups the agent's <strong>Identity</strong> (credentials
                            for Claude, Codex, GitHub, AWS, …) with its <strong>Memory</strong>
                            (notes, instructions, project context). Blank uses whatever's
                            already in your environment.
                        </span>

                        <div class="agent-launch-modal-bundle-row">
                            <span class="agent-launch-modal-bundle-row-label">Identity</span>
                            <Show
                                when={hasUserIdentities()}
                                fallback={
                                    <button
                                        type="button"
                                        class="agent-launch-modal-bundle-empty-btn"
                                        onClick={handleNewIdentity}
                                        disabled={
                                            submitting() ||
                                            isContinue() ||
                                            !props.onRequestNewIdentity
                                        }
                                        title={
                                            props.onRequestNewIdentity
                                                ? undefined
                                                : "Coming soon"
                                        }
                                    >
                                        + New identity bundle...
                                    </button>
                                }
                            >
                                <select
                                    class="agent-launch-modal-input"
                                    value={identityId()}
                                    onChange={(e) => setIdentityId(e.currentTarget.value)}
                                    disabled={submitting() || isContinue()}
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
                                <button
                                    type="button"
                                    class="agent-launch-modal-bundle-new-btn"
                                    onClick={handleNewIdentity}
                                    disabled={
                                        submitting() ||
                                        isContinue() ||
                                        !props.onRequestNewIdentity
                                    }
                                    title={
                                        props.onRequestNewIdentity
                                            ? "New identity bundle..."
                                            : "Coming soon"
                                    }
                                    aria-label="New identity bundle"
                                >
                                    +
                                </button>
                            </Show>
                        </div>

                        {/*
                         * Memory dropdown — companion to Identity.
                         * The wire selection rides through to the
                         * backend via `memoryId` on LaunchOverrides;
                         * the spawn-time content-injection layer
                         * (provider override, instructions, context
                         * files, MCP servers, skills) ships in PR-F.4
                         * and will start consuming the selection.
                         * Until then, picking a non-blank Memory is
                         * visible UX scaffolding that records the
                         * user's intent without changing runtime
                         * behavior.
                         */}

                        <div class="agent-launch-modal-bundle-row">
                            <span class="agent-launch-modal-bundle-row-label">Memory</span>
                            <Show
                                when={hasUserMemories()}
                                fallback={
                                    <button
                                        type="button"
                                        class="agent-launch-modal-bundle-empty-btn"
                                        onClick={handleNewMemory}
                                        disabled={
                                            submitting() ||
                                            isContinue() ||
                                            !props.onRequestNewMemory
                                        }
                                        title={
                                            props.onRequestNewMemory
                                                ? undefined
                                                : "Coming soon"
                                        }
                                    >
                                        + New memory bundle...
                                    </button>
                                }
                            >
                                <select
                                    class="agent-launch-modal-input"
                                    value={memoryId()}
                                    onChange={(e) => setMemoryId(e.currentTarget.value)}
                                    disabled={submitting() || isContinue()}
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
                                <button
                                    type="button"
                                    class="agent-launch-modal-bundle-new-btn"
                                    onClick={handleNewMemory}
                                    disabled={
                                        submitting() ||
                                        isContinue() ||
                                        !props.onRequestNewMemory
                                    }
                                    title={
                                        props.onRequestNewMemory
                                            ? "New memory bundle..."
                                            : "Coming soon"
                                    }
                                    aria-label="New memory bundle"
                                >
                                    +
                                </button>
                            </Show>
                        </div>
                    </fieldset>

                    {/*
                     * Pre-launch OAuth panel (spec:
                     * SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md).
                     * Shows the Connect CTA when the user has the
                     * blank singleton picked. Non-blank bundles are
                     * trusted (PR B-4 / D will tighten this with a
                     * per-bundle binding lookup).
                     */}
                    <Show when={authRequired()}>
                        <PreLaunchAuthPanel
                            provider={provider()}
                            identityId={identityId}
                            onStateChange={onAuthStateChange}
                            onBundleCreated={onBundleCreated}
                            disabled={submitting()}
                        />
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
                    {submitting()
                        ? isContinue() ? "Continuing…" : "Launching…"
                        : isContinue() ? "Continue" : "Launch"}
                </Button>
            </footer>
        </>
    );
};

AgentLaunchModalPanel.displayName = "AgentLaunchModalPanel";
