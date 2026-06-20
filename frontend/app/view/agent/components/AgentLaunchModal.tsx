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

import { waveEventSubscribe } from "@/app/store/wps";
import {
    createLaunchFlowStore,
    continueLocksIdentity as flowContinueLocksIdentity,
    continueLocksMemory as flowContinueLocksMemory,
    hasMatchingBinding,
    realIdentities,
    realMemories,
} from "@/app/store/launch-flow-state";

import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { buildInstanceSlug, slugifyInstanceName } from "../defaults/instance-slug";
import { getProvider } from "../providers";
import { PreLaunchAuthPanel } from "./PreLaunchAuthPanel";
import { AuthFlowController } from "../auth";
import {
    loadAccounts,
    refreshAccountCache,
    subscribeAccountChanges,
} from "@/app/view/identity/identity-model";

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
    /** Two-tier picker (2026-05-24) — CLI-emitted session id of the
     *  prior instance. When non-empty, written to the new block's
     *  `agent:sessionid` meta so the spawned subprocess can pass
     *  `--resume <sid>` on its FIRST turn. Without this the
     *  reattached pane starts a fresh CLI session and re-injects the
     *  startup context. */
    continueSessionId?: string;
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
    /** When true, the OAuth Connect panel fires `startConnect()` once
     *  on mount — set by the OAuth-Connect → New Identity → launch
     *  round-trip so the user doesn't re-click Connect after naming
     *  the bundle. Spec SPEC_BUNDLE_MANAGEMENT_2026_05_22.md §2. */
    autoStartAuth?: boolean;
    /** Called when the "+" / empty-state New buttons fire, OR when the
     *  OAuth Connect panel needs the New Identity modal interposed
     *  (`purpose: "oauth-continue"`). The picker is expected to chain
     *  `modalLayer.replace(newIdentityRequest)` — this component doesn't
     *  know how to build that request.
     *
     *  The current launch form state is passed through so the picker
     *  can preserve it across the new-bundle round-trip; `purpose`
     *  tells the picker whether to set `autoStartAuth` on the return
     *  launch request. */
    onRequestNewIdentity?: (
        current: LaunchFormState,
        purpose: "create" | "oauth-continue",
    ) => void;
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
    /** Continuation context. `null` = "— New agent —". Round-trip
     *  preserved alongside the form fields so Continue mode survives
     *  the `+ New bundle` flow (otherwise an ambient-creds
     *  continuation drops out of Continue and the auth gate wrongly
     *  re-engages on return — see
     *  docs/analysis/LAUNCH_MODAL_CONTINUE_LOST_2026_05_22.md). */
    continueOfId: string | null;
}

export const AgentLaunchModalPanel = (props: AgentLaunchModalPanelProps): JSX.Element => {
    const catalog = createMemo(() => getCliCatalogEntry(props.agent.provider));
    const displayName = () => catalog()?.displayName ?? props.agent.name;

    // Form + submit state live in the launch-flow-state reducer slice
    // (Stage 2b/2c of SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md).
    // Reads `flow.state.X` track the field individually via Solid's
    // createStore; writes go through `flow.dispatch(...)`. Existing
    // accessor names (`name()`, `setName(v)`, `submitting()`, …) are
    // kept as thin wrappers so the template + handler call sites
    // stay unchanged.
    //
    // What's still local (Stage 2d candidates):
    //  - `identities`, `memories`, `selectedBundleBindings` — local
    //    resources. Stage 2d migrates them to the store with push-
    //    based binding events.
    //  - `namedAgents` (continuation list) — out of slice scope.
    //  - `authController` — auth state stays in its own controller
    //    (lifted to this component in Stage 1).
    // Reducer events drive side-effects. The store calls `eventSink`
    // every time the reducer emits one; we wire `FetchBindings` to
    // run the listidentitybindings RPC + dispatch back into the
    // store. Defining the store with the sink at creation time
    // means the `Opened` dispatch below (with potentially preselected
    // identityId) goes through the sink immediately.
    const flow = createLaunchFlowStore({
        eventSink: (event) => {
            if (event.type === "FetchBindings") {
                void fetchBindings(event.identityId);
            }
        },
    });
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
    const submitting = () => flow.state.submit.inFlight;
    const error = () => flow.state.submit.error;

    const initial = props.initialFormState ?? {};
    const [showAdvanced, setShowAdvanced] = createSignal(
        (initial.runtime ?? "host") !== "host" || (initial.image ?? "") !== "",
    );

    onCleanup(() => flow.dispatch({ type: "Closed" }));

    // Identities + Memories now live in the reducer slice
    // (Stage 2c.2 of SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md).
    // `loadIdentities` / `loadMemories` are async wrappers around the
    // RPCs that dispatch the loading lifecycle commands; the view
    // reads via `flow.state.identities.list` etc. Existing accessor
    // names (`identities()`, `memories()`) are kept as thin façades.
    const loadIdentities = async () => {
        flow.dispatch({ type: "IdentitiesLoading" });
        try {
            const list = await RpcApi.ListIdentityBundlesCommand(TabRpcClient, {});
            flow.dispatch({ type: "IdentitiesLoaded", list: list ?? [] });
        } catch (e: any) {
            flow.dispatch({ type: "IdentitiesFailed", error: String(e?.message ?? e) });
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
    void loadIdentities();
    void loadMemories();
    const identities = () => flow.state.identities.list;
    const memories = () => flow.state.memories.list;

    // Empty-state predicate via the slice's `realIdentities` /
    // `realMemories` selectors, which filter out any back-compat
    // `is_blank` singleton.
    const hasUserIdentities = createMemo(() => realIdentities(flow.state).length > 0);
    const hasUserMemories = createMemo(() => realMemories(flow.state).length > 0);

    // Auto-pick the first available bundle when nothing is selected
    // yet — saves a click for users with existing bundles. Gated on
    // `!isContinue()` so legacy-continuation rows don't get filled
    // with an unrelated bundle's credentials.
    createEffect(() => {
        if (isContinue()) return;
        if (identityId()) return;
        const firstReal = realIdentities(flow.state)[0];
        if (firstReal) setIdentityId(firstReal.id);
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
        identityId: identityId(),
        memoryId: memoryId(),
        // Capture continuation context so the `+ New bundle` round-trip
        // restores Continue mode on return; without this the launch
        // modal flips to New and an ambient-creds continuation's auth
        // gate wrongly re-engages.
        continueOfId: flow.state.form.continueOfId,
    });
    // The plain `+ New identity` button uses `purpose: "create"`. The
    // OAuth Connect (`needs-bundle`) interposition routes through the
    // same callback with `purpose: "oauth-continue"` — see the
    // PreLaunchAuthPanel `onRequestNewIdentity` wiring below.
    const handleNewIdentity = () =>
        props.onRequestNewIdentity?.(snapshot(), "create");
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
    // Continuation status is read from the reducer's `form.continueOfId`
    // (the source of truth) rather than from `continuedRow()` — the
    // latter depends on `namedAgents()` having loaded, so on round-trip
    // re-open isContinue() would transiently be false until that
    // resource lands, flipping the auth gate on for a tick. Reading the
    // form directly keeps the answer stable from the moment Opened
    // dispatches with a restored continueOfId.
    const isContinue = () => flow.state.form.continueOfId !== null;
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

    // ── Feature A — Continue / New view mode ──────────────────────────
    // SPEC_AGENT_LAUNCH_AND_MODAL_DISMISSAL §A. When this definition has
    // past instances, default to Continue (most-recent preselected) so
    // re-opening a configured agent is one step, not a dropdown hunt.
    const mostRecentInstance = (): NamedAgentRow | null => {
        const rows = namedAgents() ?? [];
        if (rows.length === 0) return null;
        return [...rows].sort((a, b) => (b.started_at ?? 0) - (a.started_at ?? 0))[0];
    };
    // Initial viewMode honors a restored continuation: if the snapshot
    // captured a `continueOfId`, the user was in Continue mode when
    // they left for the `+ New bundle` flow — restore them there. The
    // previous "+New buttons disabled while continuing → always restore
    // as New" assumption was false for ambient-creds continuations
    // (continueLocksIdentity is false when the carried identity is
    // empty, so the +New button stays enabled even in Continue mode).
    const [viewMode, setViewMode] = createSignal<"continue" | "new">(
        props.initialFormState?.continueOfId != null ? "continue" : "new",
    );
    // viewModeDecided suppresses the auto-decide effect when we're
    // restoring from a snapshot — the snapshot's continueOfId decides
    // it, not the most-recent-instance heuristic.
    let viewModeDecided = props.initialFormState != null;
    createEffect(() => {
        const rows = namedAgents();
        if (rows === undefined || viewModeDecided) return;
        viewModeDecided = true;
        const recent = mostRecentInstance();
        if (recent) {
            setViewMode("continue");
            handleContinueSelect(recent.instance_id);
        }
    });
    const enterNewMode = () => {
        setViewMode("new");
        handleContinueSelect(""); // clears continueOfId; unlocks identity/memory
    };
    const enterContinueMode = () => {
        setViewMode("continue");
        if (continueOfId()) return;
        const recent = mostRecentInstance();
        if (recent) handleContinueSelect(recent.instance_id);
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
    const authStateKind = () => flow.state.auth.kind;
    const onBundleCreated = (bundleId: string) => {
        // The new bundle was just persisted backend-side. Switch the
        // dropdown to it AND refetch the identities list so the new
        // row appears in the dropdown options (otherwise hover/options
        // would show stale data until reopen).
        //
        // Defense-in-depth — `PreLaunchAuthPanel` filters
        // `pending-bundle-for-...` placeholders before calling, but
        // guard here too so any future call site can't poison the
        // dropdown with a non-existent bundle row.
        if (!bundleId || bundleId.startsWith("pending-bundle-for-")) {
            return;
        }
        setIdentityId(bundleId);
        void loadIdentities();
    };
    const provider = createMemo(() => getProvider(props.agent.provider));

    // Bindings for the currently selected identity bundle — used by
    // authRequired() to detect non-blank bundles that have no
    // credential binding for the agent's provider (e.g. a "Work"
    // identity created via "+ New" but never connected to Claude/Codex
    // yet). Reagent + codex P1 on PR #910 round 3 — without this
    // check, "+ New" could bypass the OAuth gate entirely.
    // Bindings now live in the reducer slice (Stage 2c.3). The store
    // emits `FetchBindings` events whenever an identity is selected
    // (IdentityChanged / Opened / ContinueOfChanged for uncached id).
    // We respond by running the RPC + dispatching BindingsLoading →
    // BindingsLoaded. The reducer's `hasMatchingBinding(state, providerId)`
    // selector handles the race guard (loading → false) that the
    // legacy memo did inline.
    async function fetchBindings(id: string): Promise<void> {
        if (!id || id.startsWith("pending-bundle-for-")) return;
        flow.dispatch({ type: "BindingsLoading", identityId: id });
        try {
            const list = await RpcApi.ListIdentityBindingsCommand(TabRpcClient, {
                identity_id: id,
            });
            flow.dispatch({ type: "BindingsLoaded", identityId: id, bindings: list ?? [] });
        } catch {
            // Settle the loading flag with an empty list — same
            // safe-default the prior createResource catch returned.
            flow.dispatch({ type: "BindingsLoaded", identityId: id, bindings: [] });
        }
    }

    // Cross-tab + cross-pane reactivity: subscribe to the backend's
    // `identitybundlebindings:changed:<id>` push event so a bind/
    // unbind in another tab's Identity pane updates this modal
    // without a manual refetch. Re-subscribes on identityId change
    // since the event name embeds the identity id.
    createEffect(() => {
        const id = identityId();
        if (!id) return;
        const unsub = waveEventSubscribe({
            eventType: `identitybundlebindings:changed:${id}`,
            handler: () => {
                void (async () => {
                    // Refresh the account cache too. The backend's
                    // expiry probe (PR D) updates IdentityAccount.status
                    // and publishes this same event — without refreshing
                    // accounts here, `bundleBindingStatus` reads stale
                    // status from the in-memory cache until something
                    // unrelated triggers a refresh. codex P2 on #982.
                    void refreshAccountCache();
                    try {
                        const list = await RpcApi.ListIdentityBindingsCommand(TabRpcClient, {
                            identity_id: id,
                        });
                        flow.dispatch({
                            type: "BindingsChanged",
                            identityId: id,
                            bindings: list ?? [],
                        });
                    } catch {
                        // Best-effort; leave the cache unchanged on
                        // failure so the user can still launch with
                        // whatever the last good fetch saw.
                    }
                })();
            },
        });
        onCleanup(unsub);
    });

    const bundleHasMatchingBinding = createMemo(() => {
        const providerId = provider()?.id;
        if (!providerId) return false;
        return hasMatchingBinding(flow.state, providerId);
    });

    // Account-cache backed memo that resolves the bound account row for
    // the current (identity, provider) pair and exposes its `status`
    // string. Per spec §4.4 the canonical oauth-class values are
    // `"valid" | "expired" | "needs_reauth" | "unknown"`. Returns `null`
    // when there's no matching binding or the account row isn't in the
    // cache yet — the consumer (`PreLaunchAuthPanel`) treats `null` as
    // "no status info, use generic wording".
    //
    // The cache is the same one `IdentityPaneViewModel` subscribes to,
    // so a status flip pushed by the backend's expiry probe
    // (`identitybundlebindings:changed:<id>` → refreshes cache)
    // reactively updates this memo and re-renders the panel.
    const [accountCacheTick, setAccountCacheTick] = createSignal(0);
    const accountCacheUnsub = subscribeAccountChanges(() => {
        setAccountCacheTick((n) => n + 1);
    });
    onCleanup(accountCacheUnsub);
    const bundleBindingStatus = createMemo<string | null>(() => {
        accountCacheTick();
        const id = flow.state.form.identityId;
        const providerId = provider()?.id;
        if (!id || !providerId) return null;
        const bindings = flow.state.bindings[id] ?? [];
        const b = bindings.find((bb) => bb.provider === providerId);
        if (!b) return null;
        const acc = loadAccounts().find((a) => a.id === b.account_id);
        return acc?.status ?? null;
    });

    // True when the bound account is in an oauth-class state that
    // benefits from a Reconnect nudge — strictly a wording trigger, not
    // a launch-blocker (spec §4.4: "wording-only nudge"). The Launch
    // button stays enabled because the binding still counts; the CLI
    // will refresh on its first call.
    const bundleNeedsReconnectNudge = createMemo(() => {
        const s = bundleBindingStatus();
        return s === "needs_reauth" || s === "expired";
    });

    // Auth gate applies to fresh launches of OAuth providers when the
    // selected identity can't supply credentials for the agent's
    // provider. That's true when:
    //
    // - `blank`/empty is selected — ambient creds, OAuth flow runs once.
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
    //
    // Hard auth-blockers: launch CANNOT proceed without the user
    // completing OAuth. Drives both the panel mount AND the launch
    // gate.
    //
    // (Historical note: PRs A–E of SPEC_OAUTH_IDENTITY_BUNDLES wired
    //  oauth-class providers — including openclaw — through real
    //  per-bundle credential dirs, so the previous `provider.id ===
    //  "openclaw"` always-gate is no longer needed. The standard
    //  `hasMatchingBinding` path handles every oauth provider.
    //  PR F cleanup, 2026-05-22.)
    const authBlocksLaunch = () =>
        !isContinue()
        && provider()?.authType === "oauth"
        && (
            identityId() === ""
            || !bundleHasMatchingBinding()
        );
    // Soft nudges: show the Connect CTA (with status-aware wording)
    // when the binding is present but its account status is
    // `needs_reauth` / `expired`. Per spec §4.4 this is a "wording-only
    // nudge" — does NOT block Launch. The CLI's own refresh path
    // handles the rest on first call; the nudge just gives the user a
    // one-click path to refresh proactively.
    const authNeedsReconnectWording = () =>
        !isContinue()
        && provider()?.authType === "oauth"
        && bundleHasMatchingBinding()
        && bundleNeedsReconnectNudge();
    // Mount the panel for EITHER reason (hard block or soft nudge).
    const authRequired = () => authBlocksLaunch() || authNeedsReconnectWording();
    // Launch-readiness gate. Note this only consults `authBlocksLaunch`
    // — the wording-only path doesn't gate. The panel may still show a
    // `ConnectCta` in the nudge case, but `authReady` returns true and
    // Launch is clickable.
    const authReady = () => !authBlocksLaunch() || authStateKind() === "ready";
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
        flow.dispatch({ type: "SubmitClicked" });
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
                            for Claude, Codex, GitHub, AWS, …) with its <strong>Preset</strong>
                            (instructions, context, MCP servers, skills). Both are required —
                            pick existing ones or create new ones below.
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
                            <span class="agent-launch-modal-bundle-row-label">Preset</span>
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
                                        + New preset...
                                    </button>
                                }
                            >
                                <select
                                    class="agent-launch-modal-input"
                                    value={memoryId()}
                                    onChange={(e) => setMemoryId(e.currentTarget.value)}
                                    disabled={submitting() || continueLocksMemory()}
                                    aria-label="Preset"
                                >
                                    <Show when={!memoryId()}>
                                        <option value="" disabled>— Pick a preset —</option>
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
                                            ? "New preset..."
                                            : "Coming soon"
                                    }
                                    aria-label="New preset"
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
                            bindingStatus={bundleBindingStatus}
                            controller={authController}
                            onBundleCreated={onBundleCreated}
                            autoStartAuth={props.initialFormState != null && props.autoStartAuth === true}
                            onRequestNewIdentity={
                                props.onRequestNewIdentity
                                    ? () =>
                                          props.onRequestNewIdentity?.(
                                              snapshot(),
                                              "oauth-continue",
                                          )
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
