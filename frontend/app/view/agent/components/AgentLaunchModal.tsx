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

import { createEffect, createMemo, createResource, createSignal, For, onCleanup, Show, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

import {
    createLaunchFlowStore,
    continueLocksIdentity as flowContinueLocksIdentity,
    continueLocksMemory as flowContinueLocksMemory,
} from "@/app/store/launch-flow-state";

import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { buildInstanceSlug, slugifyInstanceName } from "../defaults/instance-slug";
import { getProvider } from "../providers";
import { PreLaunchAuthPanel } from "./PreLaunchAuthPanel";
import { AuthFlowController } from "../auth";

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
    /** Selected Identity bundle id. Required (non-empty) at submit;
     *  the form blocks Launch until the user picks or creates one. */
    identityId: string;
    /** Selected Memory bundle id. Required (non-empty) at submit. */
    memoryId: string;
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
    /** Initial form state — used by the "+ New" → create → replace-
     *  back flow so the user's in-progress edits (name, runtime,
     *  image, identity, memory) survive the round-trip. All fields
     *  default to the modal's normal initial values when omitted.
     *  Codex P2 on PR #910 rounds 6 + 7: rounds 1-6 only restored
     *  identity/memory selection; round 7 added the rest of the form. */
    initialFormState?: Partial<LaunchFormState>;
    /** Called when the "+" / empty-state New buttons fire. The picker
     *  is expected to chain `tabModal.replace(newIdentityRequest)` —
     *  this component doesn't know how to build that request.
     *
     *  The current launch form state is passed through so the picker
     *  can preserve it across the new-bundle round-trip. */
    onRequestNewIdentity?: (current: LaunchFormState) => void;
    onRequestNewMemory?: (current: LaunchFormState) => void;
}

/** Snapshot of the editable Launch form. Used to thread the user's
 *  in-progress edits through the new-identity / new-memory modal so
 *  returning to Launch doesn't reset typed-but-unsubmitted values. */
export interface LaunchFormState {
    name: string;
    runtime: "host" | "container";
    image: string;
    identityId: string;
    memoryId: string;
}

export const AgentLaunchModalPanel = (props: AgentLaunchModalPanelProps): JSX.Element => {
    const catalog = createMemo(() => getCliCatalogEntry(props.agent.provider));
    const displayName = () => catalog()?.displayName ?? props.agent.name;

    // Form state lives in the launch-flow-state reducer slice (Stage
    // 2b of SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md). Reads
    // `flow.state.form.X` track the field individually via Solid's
    // createStore; writes go through `flow.dispatch(...)`. Existing
    // accessor names (`name()`, `setName(v)`, …) are kept as thin
    // wrappers so the template + handler call sites stay unchanged.
    //
    // What's NOT migrated yet:
    //  - `submitting` / `error` — `SubmitFailed` would auto-clear
    //    `inFlight`, breaking the catch-path order in handleSubmit
    //    where setError() precedes setSubmitting(false). Stage 2c
    //    can refactor handleSubmit to dispatch SubmitClicked /
    //    SubmitFailed directly instead of routing through the legacy
    //    setter pairs.
    //  - `identities`, `memories`, `selectedBundleBindings` — still
    //    local resources. Stage 2c migrates them to the store with
    //    push-based binding events.
    //  - `namedAgents` (continuation list) — out of slice scope.
    //  - `authController` — auth state stays in its own controller
    //    (lifted to this component in Stage 1).
    const flow = createLaunchFlowStore();
    flow.dispatch({ type: "Opened", initial: props.initialFormState });
    const name = () => flow.state.form.name;
    const setName = (v: string) => flow.dispatch({ type: "NameChanged", name: v });
    const runtime = () => flow.state.form.runtime;
    const setRuntime = (v: "host" | "container") =>
        flow.dispatch({ type: "RuntimeChanged", runtime: v });
    const image = () => flow.state.form.image;
    const setImage = (v: string) => flow.dispatch({ type: "ImageChanged", image: v });
    const identityId = () => flow.state.form.identityId;
    const setIdentityId = (v: string) =>
        flow.dispatch({ type: "IdentityChanged", identityId: v });
    const memoryId = () => flow.state.form.memoryId;
    const setMemoryId = (v: string) =>
        flow.dispatch({ type: "MemoryChanged", memoryId: v });

    const [submitting, setSubmitting] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    const initial = props.initialFormState ?? {};
    const [showAdvanced, setShowAdvanced] = createSignal(
        (initial.runtime ?? "host") !== "host" || (initial.image ?? "") !== "",
    );

    onCleanup(() => flow.dispatch({ type: "Closed" }));

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

    // Empty-state predicate. The backend may still return an
    // implicit `is_blank` singleton (for back-compat with older
    // clients); filter it out so the dropdown only shows real,
    // user-created bundles.
    const hasUserIdentities = createMemo(() =>
        (identities() ?? []).some((b) => !b.is_blank),
    );
    const hasUserMemories = createMemo(() =>
        (memories() ?? []).some((m) => !m.is_blank),
    );

    // Auto-pick the first available bundle when nothing is selected
    // yet — saves a click for users with existing bundles (spec
    // §3.1.2). Gated on `!isContinue()` so legacy-continuation rows
    // (where handleContinueSelect deliberately clears the carry-over
    // to "") don't silently get filled with an unrelated bundle's
    // credentials. In continuation mode the user must explicitly pick.
    createEffect(() => {
        if (isContinue()) return;
        if (identityId()) return;
        const firstReal = (identities() ?? []).find((b) => !b.is_blank);
        if (firstReal) setIdentityId(firstReal.id);
    });
    createEffect(() => {
        if (isContinue()) return;
        if (memoryId()) return;
        const firstReal = (memories() ?? []).find((m) => !m.is_blank);
        if (firstReal) setMemoryId(firstReal.id);
    });

    // "+ New ..." buttons delegate to picker-injected callbacks that
    // chain `tabModal.replace(newBundleRequest)`. The picker owns the
    // chain because it can rebuild the Launch request with
    // preselectedIdentityId/preselectedMemoryId after creation. A
    // missing callback (Phase β ships Identity wiring only; Memory
    // wiring lands in Phase γ) keeps the button visible but disabled
    // with a "coming soon" hint — see reagent P2 on PR #910.
    const snapshot = (): LaunchFormState => ({
        name: name(),
        runtime: runtime(),
        image: image(),
        identityId: identityId(),
        memoryId: memoryId(),
    });
    const handleNewIdentity = () => props.onRequestNewIdentity?.(snapshot());
    const handleNewMemory = () => props.onRequestNewMemory?.(snapshot());

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

    /** "" = "— New agent —" (default). Non-empty = continuing that
     *  past instance; name + identity + memory are pre-filled and
     *  locked. Mirrors the store's `form.continueOfId` (which uses
     *  `null` instead of "" — the dropdown uses "" as its UI sentinel
     *  for the placeholder option). */
    const continueOfId = () => flow.state.form.continueOfId ?? "";

    const continuedRow = createMemo(() => {
        const id = continueOfId();
        if (!id) return null;
        return (namedAgents() ?? []).find((r) => r.instance_id === id) ?? null;
    });
    const isContinue = () => continuedRow() != null;
    // Per-bundle continuation locks come from the slice's selectors.
    // Local memos read flow.state.form so they invalidate when the
    // selection or carry-over identity changes.
    const continueLocksIdentity = createMemo(() =>
        flowContinueLocksIdentity(flow.state),
    );
    const continueLocksMemory = createMemo(() =>
        flowContinueLocksMemory(flow.state),
    );

    const handleContinueSelect = (rawId: string) => {
        const id = rawId === "" ? null : rawId;
        const row =
            id === null
                ? null
                : (namedAgents() ?? []).find((r) => r.instance_id === id) ?? null;
        // Legacy rows may carry "" or "blank" identity_id/memory_id
        // from before the blank-removal. Treat both as "no carry-over"
        // so the user must pick a real bundle for the continuation.
        const carry = row
            ? {
                  name: row.instance_name,
                  identityId:
                      row.identity_id && row.identity_id !== "blank"
                          ? row.identity_id
                          : "",
                  memoryId:
                      row.memory_id && row.memory_id !== "blank"
                          ? row.memory_id
                          : "",
              }
            : undefined;
        flow.dispatch({ type: "ContinueOfChanged", continueOfId: id, carry });
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
    // The Launch modal OWNS the AuthFlowController (lifted from
    // PreLaunchAuthPanel — see
    // docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md). The
    // controller's lifetime spans the whole modal mount, so a brief
    // re-render that unmounts the conditionally-rendered Connect
    // panel no longer destroys in-flight auth state.
    const authController = new AuthFlowController();
    onCleanup(() => authController.dispose());
    const authStateKind = () => authController.state().kind;
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
        if (!bundleId || bundleId.startsWith("pending-bundle-for-")) {
            return;
        }
        setIdentityId(bundleId);
    };
    const provider = createMemo(() => getProvider(props.agent.provider));

    // Bindings for the currently selected identity bundle — used by
    // authRequired() to detect non-blank bundles that have no
    // credential binding for the agent's provider (e.g. a "Work"
    // identity created via "+ New" but never connected to Claude/Codex
    // yet). Reagent + codex P1 on PR #910 round 3 — without this
    // check, "+ New" could bypass the OAuth gate entirely.
    const [selectedBundleBindings] = createResource(
        () => identityId(),
        async (id) => {
            if (!id || id.startsWith("pending-bundle-for-")) {
                return [] as IdentityBinding[];
            }
            try {
                return await RpcApi.ListIdentityBindingsCommand(TabRpcClient, {
                    identity_id: id,
                });
            } catch {
                return [] as IdentityBinding[];
            }
        },
    );

    // Wrapped in createMemo (not a bare accessor) so downstream
    // effects only re-fire on actual boolean transitions. Codex P2
    // on PR #910 round 5: without the memo, when bindings finish
    // loading the value stays false (no binding yet) but the
    // underlying resource read changes, which re-ran
    // PreLaunchAuthPanel's createEffect — which calls
    // controller.selected, which cancels any in-flight `waiting`
    // OAuth session. Memo's === dedupe makes the loading→loaded
    // false→false transition a no-op.
    const bundleHasMatchingBinding = createMemo(() => {
        const id = identityId();
        if (!id) return false;
        // Treat the loading state as "no binding" so a fast-launch
        // race can't slip through the gate while bindings refetch.
        if (selectedBundleBindings.loading) return false;
        const providerId = provider()?.id;
        if (!providerId) return false;
        return (selectedBundleBindings() ?? []).some(
            (b) => b.provider === providerId,
        );
    });

    // Auth gate applies to fresh launches of OAuth providers when the
    // selected identity can't supply credentials for the agent's
    // provider. That's true when:
    //
    // - `blank`/empty is selected — ambient creds, OAuth flow runs once.
    // - OpenClaw provider — until identity bundles include openclaw
    //   auth profiles, gate ALWAYS (Phase α addition, 2026-05-17).
    //   Lifts once identity-bundles-include-openclaw lands (planned
    //   with Phase δ persistence work).
    // - A non-blank bundle without a matching provider binding —
    //   e.g. "+ New" just created an empty "Work" bundle. Reagent +
    //   codex P1 on PR #910 round 3.
    //
    // Bypasses:
    // - `isContinue` — prior launch already produced creds.
    // - API-key providers (kimi/pi) — until the backend
    //   `auth.submitapikey` persists bundles (PR C-2), the gate would
    //   deadlock. Their existing `launch-flow.ts` Phase 2 prompts for
    //   the key in-line. Reagent + codex P1 on #847.
    const authRequired = () =>
        !isContinue()
        && provider()?.authType === "oauth"
        && (
            identityId() === ""
            || provider()?.id === "openclaw"
            || !bundleHasMatchingBinding()
        );
    const authReady = () => !authRequired() || authStateKind() === "ready";
    const canSubmit = () =>
        !submitting()
        && slugifyInstanceName(name()).length > 0
        && identityId() !== ""
        && memoryId() !== ""
        && authReady();

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
                            (notes, instructions, project context). Both are required —
                            pick existing bundles or create new ones below.
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
                                            continueLocksIdentity() ||
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
                                    disabled={submitting() || continueLocksIdentity()}
                                    aria-label="Identity bundle"
                                >
                                    <Show when={!identityId()}>
                                        <option value="" disabled>— Pick an identity —</option>
                                    </Show>
                                    <For each={(identities() ?? []).filter((b) => !b.is_blank)}>
                                        {(bundle) => (
                                            <option value={bundle.id}>{bundle.name}</option>
                                        )}
                                    </For>
                                </select>
                                <button
                                    type="button"
                                    class="agent-launch-modal-bundle-new-btn"
                                    onClick={handleNewIdentity}
                                    disabled={
                                        submitting() ||
                                        continueLocksIdentity() ||
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
                                            continueLocksMemory() ||
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
                                    disabled={submitting() || continueLocksMemory()}
                                    aria-label="Memory bundle"
                                >
                                    <Show when={!memoryId()}>
                                        <option value="" disabled>— Pick a memory —</option>
                                    </Show>
                                    <For each={(memories() ?? []).filter((m) => !m.is_blank)}>
                                        {(memory) => (
                                            <option value={memory.id}>{memory.name}</option>
                                        )}
                                    </For>
                                </select>
                                <button
                                    type="button"
                                    class="agent-launch-modal-bundle-new-btn"
                                    onClick={handleNewMemory}
                                    disabled={
                                        submitting() ||
                                        continueLocksMemory() ||
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
                     * Shows the Connect CTA whenever the selected
                     * identity can't supply creds for this provider —
                     * blank singleton, openclaw, or a non-blank bundle
                     * without a matching binding (e.g. "+ New" just
                     * created an empty bundle). The panel uses
                     * `hasMatchingBinding` to compute its own outcome
                     * so non-blank-but-empty bundles don't shortcut
                     * to `ready`.
                     */}
                    <Show when={authRequired()}>
                        <PreLaunchAuthPanel
                            provider={provider()}
                            identityId={identityId}
                            hasMatchingBinding={bundleHasMatchingBinding}
                            controller={authController}
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
