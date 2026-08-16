// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentLaunchModalPanel — the form rendered inside the canonical `<ModalLayer>`
 * when the user clicks a definition card in the agent picker. Collects
 * the instance name + runtime (host vs container) and submits them to
 * the caller, which is responsible for calling launchAgentDefinition with
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
import { isAvailable, watchCapability } from "@/app/store/toolchain-capabilities";

import { createLaunchFlowStore, accountsForProvider, realMemories } from "@/app/store/launch-flow-state";

import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { defaultAgentName } from "../defaults/default-agent-name";
import { buildInstanceSlug, slugifyInstanceName } from "../defaults/instance-slug";
import { getProvider } from "../providers";
import { PreLaunchAuthPanel } from "./PreLaunchAuthPanel";
import { AuthFlowController } from "../auth";
import { refreshAccountCache, subscribeAccountChanges } from "@/app/view/identity/identity-model";
import { useContinueOrNewMode } from "../hooks/useContinueOrNewMode";
import { useLaunchAuthGate } from "../hooks/useLaunchAuthGate";

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
    /** Selected account id. Required (non-empty) at submit; the form
     *  blocks Launch until the user picks or creates one. Issue #1624
     *  PR-C Part B — was `identityId` (a bundle id). */
    accountId: string;
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
    /** Two-tier picker (2026-05-24) — CLI-emitted session id of the
     *  prior instance. When non-empty, written to the new block's
     *  `agent:sessionid` meta so the spawned subprocess can pass
     *  `--resume <sid>` on its FIRST turn. Without this the
     *  reattached pane starts a fresh CLI session and re-injects the
     *  startup context. */
    continueSessionId?: string;
    /** In-pane tabs, Phase 4 (SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md
     *  §4.1) — when set alongside `continueSessionId`, appends `--fork-session`
     *  to the spawned CLI's args so it inherits history up to the resume
     *  point and then diverges with a NEW session id, instead of continuing
     *  the same session (which `continueSessionId` alone would do). Only
     *  applied for the Claude provider specifically — the only one
     *  `--fork-session` was ever validated against
     *  (SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15 §6.4). Every other
     *  provider naturally falls back to "fork = fresh definition, fresh
     *  start" by simply not getting the flag. */
    forkSession?: boolean;
}

interface AgentLaunchModalPanelProps {
    agent: AgentDefinition;
    onCancel: () => void;
    onSubmit: (overrides: LaunchOverrides) => Promise<void> | void;
    /** Initial form state — used by the "+ New" → create → replace-
     *  back flow so the user's in-progress edits (name, runtime,
     *  image, identity, memory) survive the round-trip. All fields
     *  default to the modal's normal initial values when omitted.
     *  Codex P2 on PR #910 rounds 6 + 7: rounds 1-6 only restored
     *  identity/memory selection; round 7 added the rest of the form. */
    initialFormState?: Partial<LaunchFormState>;
    /** Called when the "+ Add account" button fires — the picker is
     *  expected to chain `modalLayer.replace(addAccountRequest)` — this
     *  component doesn't know how to build that request. The current
     *  launch form state is passed through so the picker can preserve
     *  it across the round-trip. Issue #1624 PR-C Part B — OAuth
     *  Connect no longer routes through this callback (it starts
     *  directly from `PreLaunchAuthPanel`); this now only fires for
     *  manual/API-key account creation. Replaces `onRequestNewIdentity`. */
    onRequestAddAccount?: (current: LaunchFormState) => void;
    onRequestNewMemory?: (current: LaunchFormState) => void;
}

/** Snapshot of the editable Launch form. Used to thread the user's
 *  in-progress edits through the new-identity / new-memory modal so
 *  returning to Launch doesn't reset typed-but-unsubmitted values. */
interface LaunchFormState {
    name: string;
    runtime: "host" | "container";
    image: string;
    accountId: string;
    memoryId: string;
    /** Continuation context. `null` = "— New agent —". Round-trip
     *  preserved alongside the form fields so Continue mode survives
     *  the `+ New bundle` flow (otherwise an ambient-creds
     *  continuation drops out of Continue and the auth gate wrongly
     *  re-engages on return — see
     *  docs/analysis/LAUNCH_MODAL_CONTINUE_LOST_2026_05_22.md). */
    continueOfId: string | null;
}

export const AgentLaunchModalPanel = (props: AgentLaunchModalPanelProps): JSX.Element => {
    // Resolve the effective provider through the agent's bound ABF
    // bundle, not `props.agent.provider` directly — same fix as
    // `resolveEffectiveLaunchProvider` (agent-launch-env.ts), applied
    // here because this modal independently gates which accounts are
    // offered and drives the entire pre-launch auth flow
    // (`PreLaunchAuthPanel`, below) from the provider it resolves, all
    // BEFORE the launch RPC ever reaches the backend's already-correct
    // resolution (PR #2592; see issue #2594 for the full remaining
    // scope). `agent.provider` can drift post-creation via
    // `agent.define`'s `if_exists=update` path while the bundle's own
    // copy is backend-enforced immutable — without this, a user could
    // be offered accounts for / walked through auth for the WRONG
    // provider.
    //
    // `createResource` rather than an inline async fetch inside the
    // memo below: memos must stay synchronous, so the fetch lives here
    // and downstream memos read `effectiveProviderId()`, which starts
    // as `props.agent.provider` (safe default) and reactively updates
    // once the bundle resolves — same fallback-on-anything-but-success
    // semantics as `resolveEffectiveLaunchProvider`.
    const [boundBundle] = createResource(
        () => props.agent.memory_id || undefined,
        (memoryId) => RpcApi.GetMemoryCommand(TabRpcClient, { id: memoryId }).catch(() => undefined),
    );
    const effectiveProviderId = createMemo(() => boundBundle()?.provider || props.agent.provider);

    const catalog = createMemo(() => getCliCatalogEntry(effectiveProviderId()));
    const displayName = () => catalog()?.displayName ?? props.agent.name;
    // Declared early — `createMemo`'s first run is synchronous (unlike
    // `createEffect`, which defers to after setup completes), so any
    // memo below that reads `provider()` needs the binding to already
    // exist at the point it's created.
    const provider = createMemo(() => getProvider(effectiveProviderId()));

    // Form + submit state live in the launch-flow-state reducer slice
    // (Stage 2b/2c of SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md).
    // Reads `flow.state.X` track the field individually via Solid's
    // createStore; writes go through `flow.dispatch(...)`. Existing
    // accessor names (`name()`, `setName(v)`, `submitting()`, …) are
    // kept as thin wrappers so the template + handler call sites
    // stay unchanged.
    //
    // What's still local (Stage 2d candidates):
    //  - `memories` — local resource.
    //  - `namedAgents` (continuation list) — out of slice scope.
    //  - `authController` — auth state stays in its own controller
    //    (lifted to this component in Stage 1).
    // Reducer events drive side-effects. The store calls `eventSink`
    // every time the reducer emits one; the only wrapped event left
    // is `Auth` (issue #1624 PR-C Part B removed `FetchBindings` —
    // accounts load once and are filtered client-side, no per-
    // selection binding fetch needed). Auth events are handled by the
    // AuthFlowController itself, not here.
    const flow = createLaunchFlowStore({
        eventSink: () => {},
    });
    flow.dispatch({ type: "Opened", initial: props.initialFormState });
    const name = () => flow.state.form.name;
    const setName = (v: string) => flow.dispatch({ type: "NameChanged", name: v });
    const runtime = () => flow.state.form.runtime;
    const setRuntime = (v: "host" | "container") =>
        flow.dispatch({ type: "RuntimeChanged", runtime: v });
    const image = () => flow.state.form.image;
    const setImage = (v: string) => flow.dispatch({ type: "ImageChanged", image: v });
    const accountId = () => flow.state.form.accountId;
    const setAccountId = (v: string) =>
        flow.dispatch({ type: "AccountChanged", accountId: v });
    const memoryId = () => flow.state.form.memoryId;
    const setMemoryId = (v: string) =>
        flow.dispatch({ type: "MemoryChanged", memoryId: v });
    const submitting = () => flow.state.submit.inFlight;
    const error = () => flow.state.submit.error;

    const initial = props.initialFormState ?? {};
    const [showAdvanced, setShowAdvanced] = createSignal(
        (initial.runtime ?? "host") !== "host" || (initial.image ?? "") !== "",
    );

    onCleanup(() => flow.dispatch({ type: "Closed" }));

    // Accounts + Memories now live in the reducer slice
    // (Stage 2c.2 of SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md).
    // `loadAccountsIntoForm` / `loadMemories` are async wrappers that
    // dispatch the loading lifecycle commands; the view reads via
    // `flow.state.accounts.list` etc. Issue #1624 PR-C Part B —
    // accounts source from the shared account cache
    // (`identity-model.ts`) instead of `ListIdentityBundlesCommand`,
    // and load once (no per-selection binding fetch needed — an
    // account's `provider` is already known client-side).
    const loadAccountsIntoForm = async () => {
        flow.dispatch({ type: "AccountsLoading" });
        try {
            const list = await refreshAccountCache();
            flow.dispatch({ type: "AccountsLoaded", list });
        } catch (e: any) {
            flow.dispatch({ type: "AccountsFailed", error: String(e?.message ?? e) });
        }
    };
    const loadMemories = async () => {
        flow.dispatch({ type: "MemoriesLoading" });
        try {
            const list = await RpcApi.ListMemoriesCommand(TabRpcClient, {});
            flow.dispatch({ type: "MemoriesLoaded", list: list ?? [] });
        } catch (e: any) {
            flow.dispatch({ type: "MemoriesFailed", error: String(e?.message ?? e) });
        }
    };
    void loadAccountsIntoForm();
    void loadMemories();
    const memories = () => flow.state.memories.list;

    // Cross-tab + cross-pane reactivity: the shared account cache
    // (`identity-model.ts`) self-subscribes to the backend's
    // `identityaccounts:changed` broadcast as of #2474 and refreshes
    // itself, so this modal only needs to mirror cache updates into its
    // flow state — no separate event subscription or second RPC round-trip
    // (this replaces the hand-rolled `waveEventSubscribe` workaround that
    // predated the cache's own live sync). Keeps the dropdown + status
    // live when an account is created/verified from another tab or this
    // modal's own OAuth/API-key flows, without a manual reopen.
    createEffect(() => {
        const unsub = subscribeAccountChanges((list) => {
            flow.dispatch({ type: "AccountsLoaded", list });
        });
        onCleanup(unsub);
    });

    // Empty-state predicate — accounts for the agent's own provider.
    const hasAccountsForProvider = createMemo(
        () => accountsForProvider(flow.state, provider()?.id ?? "").length > 0,
    );
    const hasUserMemories = createMemo(() => realMemories(flow.state).length > 0);

    // Auto-pick the first available account for this provider when
    // nothing is selected yet — saves a click for users with existing
    // accounts. Gated on `!isContinue()` so legacy-continuation rows
    // don't get filled with an unrelated account's credentials.
    createEffect(() => {
        if (isContinue()) return;
        if (accountId()) return;
        const first = accountsForProvider(flow.state, provider()?.id ?? "")[0];
        if (first) setAccountId(first.id);
    });
    createEffect(() => {
        if (isContinue()) return;
        if (memoryId()) return;
        const firstReal = realMemories(flow.state)[0];
        if (firstReal) setMemoryId(firstReal.id);
    });

    // "+ New ..." buttons delegate to picker-injected callbacks that
    // chain `modalLayer.replace(newBundleRequest)`. The picker owns the
    // chain because it can rebuild the Launch request with
    // preselectedIdentityId/preselectedMemoryId after creation. A
    // missing callback (Phase β ships Identity wiring only; Memory
    // wiring lands in Phase γ) keeps the button visible but disabled
    // with a "coming soon" hint — see reagent P2 on PR #910.
    const snapshot = (): LaunchFormState => ({
        name: name(),
        runtime: runtime(),
        image: image(),
        accountId: accountId(),
        memoryId: memoryId(),
        // Capture continuation context so the `+ New bundle` round-trip
        // restores Continue mode on return; without this the launch
        // modal flips to New and an ambient-creds continuation's auth
        // gate wrongly re-engages.
        continueOfId: flow.state.form.continueOfId,
    });
    // OAuth Connect no longer routes through this callback (issue
    // #1624 PR-C Part B) — it fires only for the "+ Add account"
    // (manual/API-key) path now.
    const handleAddAccount = () => props.onRequestAddAccount?.(snapshot());
    const handleNewMemory = () => props.onRequestNewMemory?.(snapshot());

    // ── Feature A — Continue / New view mode ──────────────────────────
    // SPEC_AGENT_LAUNCH_AND_MODAL_DISMISSAL §A. Extracted to
    // useContinueOrNewMode (modularization pass, 2026-07-23) — owns the
    // "Continue an existing agent" dropdown, the New/Continue toggle,
    // and everything that decides which past instance (if any) this
    // launch continues. `flow` is passed by reference so the hook's
    // reads/dispatches stay tracked against the live store.
    const {
        namedAgents,
        continueOfId,
        continuedRow,
        isContinue,
        continueLocksIdentity,
        continueLocksMemory,
        handleContinueSelect,
        viewMode,
        enterNewMode,
        enterContinueMode,
    } = useContinueOrNewMode({
        flow,
        agentId: props.agent.id,
        hasInitialFormState: props.initialFormState != null,
        initialContinueOfId: props.initialFormState?.continueOfId,
    });

    // Default agent name (#780) — pre-fill so the user can click Launch
    // immediately instead of needing to type something first. Runs once,
    // after useContinueOrNewMode's own effect has settled viewMode for
    // this open (Continue mode already prefills the name from the
    // continued row via handleContinueSelect's carry-over, so this only
    // applies in New mode). Checking `name() === ""` at fire time —
    // rather than a separate isDirty flag — is enough to respect both a
    // user who typed before this resolved and a round-tripped
    // `initialFormState.name`: provider is fixed for this component's
    // whole lifetime (one AgentDefinition per modal instance), so
    // there's no "recompute on provider change" case to handle here,
    // unlike the original spec's multi-provider-picker assumption.
    let defaultNameApplied = false;
    createEffect(() => {
        const rows = namedAgents();
        if (rows === undefined || defaultNameApplied || viewMode() !== "new") return;
        defaultNameApplied = true;
        if (name() !== "") return;
        const existing = new Set(rows.map((r) => r.instance_name));
        setName(defaultAgentName(displayName(), existing));
    });

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

    // Live Docker-availability signal, purely informational here — see the
    // inline hint near the sandbox radio below. Deliberately NOT part of the
    // gating logic: unlike the create-from-template modal, this modal also
    // drives relaunch/continue of already-configured container agents, and
    // hard-disabling on a possibly-stale probe at modal-open time risks
    // blocking a legitimate continue. `containerSupported` (does this
    // provider ship a container image at all) remains the only thing that
    // actually disables the radio. See
    // docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
    onCleanup(watchCapability("docker"));

    // If the catalog says this provider can't run in a container, coerce
    // runtime to "host" regardless of the form default ("container").
    // The radio is disabled in the UI but that only prevents interaction —
    // it does not change the value, so without this guard a host-only
    // provider opened from AgentPicker would submit runtime:"container"
    // and the backend would fall back to the Claude image.
    createEffect(() => {
        if (!containerSupported() && runtime() === "container") {
            setRuntime("host");
        }
    });

    // Pre-launch OAuth (spec: SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md).
    // The Launch modal OWNS the AuthFlowController (lifted from
    // PreLaunchAuthPanel — see
    // docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md). The
    // controller's lifetime spans the whole modal mount, so a brief
    // re-render that unmounts the conditionally-rendered Connect
    // panel no longer destroys in-flight auth state.
    // Auth state is owned by the slice (Stage 2d.2). The controller
    // continues to orchestrate side-effects (OAuth start, polling,
    // openExternal) but reads + writes state through the slice via
    // the externalGetState / externalDispatch hooks. So
    // `flow.state.auth` is the single source of truth.
    const authController = new AuthFlowController({
        externalGetState: () => flow.state.auth,
        externalDispatch: (cmd) => flow.dispatch({ type: "Auth", cmd }),
    });
    onCleanup(() => authController.dispose());
    const onAccountCreated = (accId: string) => {
        // The new account was just persisted backend-side (OAuth or
        // API-key). Switch the dropdown to it AND refresh the account
        // cache so the new row appears in the dropdown options
        // (otherwise hover/options would show stale data until reopen).
        //
        // Defense-in-depth — guard against an empty id so a future call
        // site can't poison the dropdown with a non-existent row.
        if (!accId) return;
        setAccountId(accId);
        void loadAccountsIntoForm();
    };

    // Auth-gating logic (spec: SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md).
    // Extracted to useLaunchAuthGate (modularization pass, 2026-07-23) —
    // `flow` is passed by reference so its reads stay tracked against
    // the live store; `provider`/`isContinue` are threaded through as
    // accessors since the hook has no other way to derive them.
    const {
        authBlocksLaunch,
        authNeedsReconnectWording,
        authRequired,
        authReady,
        accountSupplies,
        selectedAccountStatus,
    } = useLaunchAuthGate({ flow, provider, isContinue });
    const canSubmit = () =>
        !submitting()
        && slugifyInstanceName(name()).length > 0
        && accountId() !== ""
        && memoryId() !== ""
        && authReady();

    const resolvedImage = () => {
        const v = image().trim();
        if (v) return v;
        return catalog()?.containerImage ?? "";
    };

    const handleSubmit = async () => {
        if (!canSubmit()) return;
        flow.dispatch({ type: "SubmitClicked" });
        try {
            const row = continuedRow();
            await props.onSubmit({
                instanceName: name().trim(),
                agentType: runtime(),
                environment: runtime() === "container" ? "docker" : "local",
                containerImage: runtime() === "container" ? resolvedImage() : undefined,
                accountId: accountId(),
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
            flow.dispatch({ type: "SubmitFailed", error: String(e?.message ?? e) });
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

                    {/* Feature A — Continue / New view mode
                        (SPEC_AGENT_LAUNCH_AND_MODAL_DISMISSAL §A). When this
                        definition has past instances the modal opens in
                        Continue mode: the dropdown picks which instance to
                        resume (pre-fills + locks name/identity/memory) and a
                        toggle drops to the full New-agent form. No past
                        instances ⇒ no toggle, New is the only path. */}
                    <Show when={(namedAgents() ?? []).length > 0}>
                        <Show
                            when={viewMode() === "continue"}
                            fallback={
                                <button
                                    type="button"
                                    class="agent-launch-modal-mode-toggle"
                                    onClick={enterContinueMode}
                                    disabled={submitting()}
                                >
                                    ↩ Continue an existing agent
                                </button>
                            }
                        >
                            <label class="agent-launch-modal-field">
                                <span class="agent-launch-modal-label">Continue an existing agent</span>
                                <select
                                    class="agent-launch-modal-input"
                                    value={continueOfId()}
                                    onChange={(e) => handleContinueSelect(e.currentTarget.value)}
                                    disabled={submitting()}
                                    aria-label="Continue an existing agent"
                                >
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
                            <button
                                type="button"
                                class="agent-launch-modal-mode-toggle"
                                onClick={enterNewMode}
                                disabled={submitting()}
                            >
                                + Start a new agent instead
                            </button>
                        </Show>
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
                                {/* Non-blocking — see the `watchCapability("docker")` comment
                                    above for why this doesn't gate `disabled`. */}
                                <Show when={containerSupported() && !isAvailable("docker")}>
                                    <span class="agent-launch-modal-hint agent-launch-modal-hint--warn">
                                        Docker daemon not detected — start Docker Desktop, then try again.
                                    </span>
                                </Show>
                            </span>
                        </label>
                    </fieldset>

                    <Show when={runtime() === "host"}>
                        <div class="agent-launch-modal-host-warning" role="note">
                            <i class="fa-solid fa-server" aria-hidden="true" />
                            <span>
                                <strong>Full system access.</strong>{" "}
                                This agent runs directly on your machine and can read any file,
                                use any credential, and run any command your account can.
                                Use only for admin or system-level tasks — sandbox mode is
                                recommended for all regular work.
                            </span>
                        </div>
                    </Show>

                    <fieldset class="agent-launch-modal-field agent-launch-modal-bundles">
                        <legend class="agent-launch-modal-label">Profile</legend>
                        <span class="agent-launch-modal-hint">
                            A Profile groups the agent's <strong>Identity</strong> (credentials
                            for Claude, Codex, GitHub, AWS, …) with its <strong>Bundle</strong>
                            (instructions, context, MCP servers, skills). Both are required —
                            pick existing ones or create new ones below.
                        </span>

                        <div class="agent-launch-modal-bundle-row">
                            <span class="agent-launch-modal-bundle-row-label">Identity</span>
                            <Show
                                when={hasAccountsForProvider()}
                                fallback={
                                    <button
                                        type="button"
                                        class="agent-launch-modal-bundle-empty-btn"
                                        onClick={handleAddAccount}
                                        disabled={
                                            submitting() ||
                                            continueLocksIdentity() ||
                                            !props.onRequestAddAccount
                                        }
                                        title={
                                            props.onRequestAddAccount
                                                ? undefined
                                                : "Coming soon"
                                        }
                                    >
                                        + Add {provider()?.displayName ?? "account"}...
                                    </button>
                                }
                            >
                                <select
                                    class="agent-launch-modal-input"
                                    value={accountId()}
                                    onChange={(e) => setAccountId(e.currentTarget.value)}
                                    disabled={submitting() || continueLocksIdentity()}
                                    aria-label="Account"
                                >
                                    <Show when={!accountId()}>
                                        <option value="" disabled>— Pick an account —</option>
                                    </Show>
                                    <For each={accountsForProvider(flow.state, provider()?.id ?? "")}>
                                        {(account) => (
                                            <option value={account.id}>
                                                {account.display_name?.trim() || account.name}
                                            </option>
                                        )}
                                    </For>
                                </select>
                                <button
                                    type="button"
                                    class="agent-launch-modal-bundle-new-btn"
                                    onClick={handleAddAccount}
                                    disabled={
                                        submitting() ||
                                        continueLocksIdentity() ||
                                        !props.onRequestAddAccount
                                    }
                                    title={
                                        props.onRequestAddAccount
                                            ? `Add ${provider()?.displayName ?? "account"}...`
                                            : "Coming soon"
                                    }
                                    aria-label="Add account"
                                >
                                    +
                                </button>
                            </Show>
                        </div>

                        {/*
                         * Preset dropdown — companion to Identity.
                         * The wire selection rides through to the
                         * backend via `memoryId` on LaunchOverrides;
                         * the spawn-time content-injection layer
                         * (instructions, context files, MCP servers,
                         * skills — presets are provider-agnostic) ships
                         * in PR-F.4 and will start consuming the
                         * selection. Until then, picking a non-blank
                         * preset is visible UX scaffolding that records
                         * the user's intent without changing runtime
                         * behavior.
                         */}

                        <div class="agent-launch-modal-bundle-row">
                            <span class="agent-launch-modal-bundle-row-label">Bundle</span>
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
                                        + New bundle...
                                    </button>
                                }
                            >
                                <select
                                    class="agent-launch-modal-input"
                                    value={memoryId()}
                                    onChange={(e) => setMemoryId(e.currentTarget.value)}
                                    disabled={submitting() || continueLocksMemory()}
                                    aria-label="Bundle"
                                >
                                    <Show when={!memoryId()}>
                                        <option value="" disabled>— Pick a bundle —</option>
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
                                            ? "New bundle..."
                                            : "Coming soon"
                                    }
                                    aria-label="New bundle"
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
                     * account can't supply creds for this provider — no
                     * account selected, or a selected account for a
                     * different provider. Issue #1624 PR-C Part B: OAuth
                     * Connect starts directly (no "+ New identity"
                     * interposition) — the backend mints the account.
                     */}
                    <Show when={authRequired()}>
                        <PreLaunchAuthPanel
                            provider={provider()}
                            accountId={accountId}
                            accountSuppliesProvider={accountSupplies}
                            accountStatus={selectedAccountStatus}
                            controller={authController}
                            onAccountCreated={onAccountCreated}
                            onRequestAddAccount={
                                props.onRequestAddAccount
                                    ? () => props.onRequestAddAccount?.(snapshot())
                                    : undefined
                            }
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
                <Button onClick={props.onCancel} disabled={submitting()} data-modal-dismiss>
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
