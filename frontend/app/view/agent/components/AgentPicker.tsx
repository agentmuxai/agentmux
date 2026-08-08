// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPicker — shown when an agent pane has no agentId in block meta.
 *
 * Two-tier layout (Phase 1 — SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md):
 *  - **My Agents** (top): user-owned agents (`is_seeded = 0`). Each
 *    row shows the agent's current Option E session state (preview,
 *    node count, last-active timestamp). Click = reattach via the
 *    existing `continueOfInstanceId + workDirOverride` flow.
 *  - **+ New from template** (bottom): seeded templates
 *    (`is_seeded = 1`). Click = open the create-from-template modal,
 *    which clones the template into a new user agent + immediately
 *    launches it.
 *
 * Templates by Phase 1 invariant carry NO session zone (the startup
 * migration `migrate_promote_template_sessions_v1` evicts any
 * pre-existing template-session into a user agent). The template
 * cards therefore never show the "+ New" pill or auto-continue.
 *
 * Pre-existing flows kept intact:
 *  - Modifier-key force-modal on a My Agents row (Shift / Ctrl / Alt /
 *    Cmd) bypasses auto-continue.
 *  - Install + prereq modals layered on top of template-create flow
 *    (templates with un-installed CLIs still go through their install
 *    modal before the create-from-template modal opens).
 *
 * See docs/specs/SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md (legacy)
 * and docs/specs/SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md (current).
 */

import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { waveEventSubscribe } from "@/app/store/wps";
import { useModalLayer, type LaunchFormStateWire } from "@/element/modal-layer";
import { getPlatform } from "@/util/platformutil";
import { refocusNode } from "@/app/store/global";
import { getOpenDefinitionMap } from "@/app/store/agent-pane-state-store";
import { subscribeToPaneLifecycle } from "@/app/store/agent-pane-registration";
import type { AgentViewModel } from "../agent-model";
import { getProvider } from "../providers";
import { looksLikeRealAccountId } from "../identity-carry-over";
import { loadAccounts } from "@/app/view/identity/identity-model";
import { AgentCard } from "./AgentCard";
import { AgentActionBar } from "./AgentActionBar";
import { HiddenTemplatesSection } from "./HiddenTemplatesSection";
import type { LaunchOverrides } from "./AgentLaunchModal";
import { MyAgentsList } from "./MyAgentsList";

// ── Carried-over identity_id validation (#2463 Finding 1) ──────────────────
//
// `looksLikeRealAccountId`'s UUID-shape check alone isn't sufficient here:
// pre-#1624-PR-C identity-bundle ids were ALSO UUID-formatted, just not
// account ids — a legacy row carrying one would pass the shape check, still
// hit LinkAgentIdentityCommand's FOREIGN KEY constraint against db_accounts,
// and still get silently swallowed by the write-through's try/catch (codex
// P1 on this fix). Cross-checking against the live account cache closes
// that gap. `loadAccounts()` is a synchronous best-effort read of a cache
// primed at app startup — by the time a user clicks reattach/fork on an
// existing row, priming has long since completed.
function realAccountIdOrEmpty(id: string): string {
    if (!looksLikeRealAccountId(id)) return "";
    return loadAccounts().some((a) => a.id === id) ? id : "";
}

// ── useAgentDefinitions hook ───────────────────────────────────────────────────────

/**
 * Reactive accessor for the current agent-definition list. Subscribes to
 * `agents:changed` and refetches when that event fires.
 */
export function useAgentDefinitions(): () => AgentDefinition[] {
    const [agents, setAgents] = createSignal<AgentDefinition[]>([]);

    onMount(() => {
        let cancelled = false;

        async function load() {
            try {
                const result = await RpcApi.ListAgentDefinitionsCommand(TabRpcClient);
                if (!cancelled) setAgents(result ?? []);
            } catch {
                // silently ignore
            }
        }

        load();

        const unsub = waveEventSubscribe({
            eventType: "agents:changed",
            handler: () => load(),
        });

        onCleanup(() => {
            cancelled = true;
            unsub();
        });
    });

    return agents;
}

/**
 * Reactive definitionId → open blockId map. Refreshes on pane register/
 * unregister (`subscribeToPaneLifecycle`) AND on `agents:changed` so newly-
 * forked definitions also appear. Extracted from what was AgentPicker's own
 * inline signal so other consumers (the agent-pane fork tab strip,
 * SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.1) can share it
 * instead of re-deriving the same map.
 */
export function useOpenDefinitionMap(): [() => Map<string, string>, () => void] {
    const [openDefinitions, setOpenDefinitions] = createSignal<Map<string, string>>(new Map());
    const refresh = () => setOpenDefinitions(getOpenDefinitionMap());
    onMount(refresh);
    const unsubAgentsChanged = waveEventSubscribe({
        eventType: "agents:changed",
        handler: refresh,
    });
    const unsubPaneLifecycle = subscribeToPaneLifecycle(refresh);
    onCleanup(unsubAgentsChanged);
    onCleanup(unsubPaneLifecycle);
    return [openDefinitions, refresh];
}

// ── AgentPicker component ───────────────────────────────────────────────────────

interface AgentPickerProps {
    model: AgentViewModel;
}

export const AgentPicker = (props: AgentPickerProps): JSX.Element => {
    const [launching, setLaunching] = createSignal<string | null>(null);
    const [nodejsError, setNodejsError] = createSignal<string | null>(null);
    const agents = useAgentDefinitions();
    const modalLayer = useModalLayer();

    // Reactive map of definition_id → blockId for panes currently open.
    const [openDefinitions, refreshOpenDefinitions] = useOpenDefinitionMap();

    // Per-agent install state, keyed by agent.id.
    //   undefined = not yet checked / non-npm provider (no install needed)
    //   true      = present in the per-version cache
    //   false     = needs install — card shows the bottom-right ribbon
    const [installState, setInstallState] = createSignal<Record<string, boolean | undefined>>({});

    // Phase 1 two-tier picker (reagent P2 on #1011): the auto-continue
    // session-state cache + probe lived here for the Option E PR-2
    // default-click path. After the two-tier rewrite, templates carry
    // `hasCurrentSession={false}` by invariant and `handleSelect` is
    // unreachable from any card render, so the cache + probe + the
    // entire session-state branch became dead. Deleted. If a future
    // tier needs per-agent session probing, re-introduce the cache
    // there scoped to that tier's data source — not here.

    const checkInstalled = async (agent: AgentDefinition) => {
        const prov = getProvider(agent.provider);
        // Non-npm providers (kimi via pip, system-PATH CLIs) don't go
        // through the install modal — never show the ribbon.
        if (!prov?.npmPackage || prov.npmPackage.length === 0) {
            setInstallState((s) => ({ ...s, [agent.id]: undefined }));
            return;
        }
        try {
            const r = await RpcApi.InstallCheckCommand(TabRpcClient, {
                providerId: prov.id,
                cliCommand: prov.cliCommand,
            });
            setInstallState((s) => ({ ...s, [agent.id]: r.installed }));
        } catch {
            // Treat as needs-install — better to over-prompt than miss.
            setInstallState((s) => ({ ...s, [agent.id]: false }));
        }
    };

    // Build the launch-agent request descriptor. Separated from the
    // open call so the install→launch chain can hand the same shape
    // to `modalLayer.replace()` for a crossfade (no shell teardown).
    //
    // `initialFormState` carries the user's in-progress launch-form
    // edits through the "+ Add account / + New memory" round-trip —
    // name, runtime, image, account, memory. Spec: Phase β of
    // SPEC_LAUNCH_MODAL_PROFILE_SECTION_2026_05_18.md; codex P2 on
    // round 7 expanded preservation from identity+memory only to the
    // full form.
    const buildLaunchRequest = (
        agent: AgentDefinition,
        initialFormState?: Partial<LaunchFormStateWire>,
    ) => ({
        kind: "launch-agent" as const,
        agent,
        originBlockId: props.model.blockId,
        onSubmit: async (overrides: LaunchOverrides) => {
            setLaunching(agent.id);
            try {
                await props.model.launchAgentDefinition(agent, overrides);
                if (props.model.nodejsError) {
                    // Surface the missing-Node banner inside the
                    // picker after the layer closes the modal —
                    // matches the pre-refactor behaviour.
                    setNodejsError(props.model.nodejsError);
                    props.model.nodejsError = null;
                }
            } finally {
                setLaunching(null);
            }
        },
        initialFormState,
        // OAuth Connect no longer routes through this callback (issue
        // #1624 PR-C Part B) — it starts directly from the launch
        // modal's auth panel. This now only fires for the "+ Add
        // account" (manual/API-key) button.
        onRequestAddAccount: (current: LaunchFormStateWire) => {
            // Thread the user's whole live form snapshot through the
            // add-account round-trip so name/runtime/image/memory
            // survive alongside the freshly-created account id.
            modalLayer.replace({
                kind: "add-account" as const,
                originBlockId: props.model.blockId,
                provider: agent.provider,
                onCreated: (id: string) => {
                    modalLayer.replace(
                        buildLaunchRequest(agent, {
                            ...current,
                            accountId: id,
                        }),
                    );
                },
                onCancel: () => {
                    modalLayer.replace(buildLaunchRequest(agent, current));
                },
            });
        },
        onRequestNewMemory: (current: LaunchFormStateWire) => {
            // Mirror of onRequestAddAccount above — thread the live
            // form snapshot through the new-memory round-trip so the
            // user's other edits (name, runtime, image, account)
            // survive alongside the freshly-created memory id.
            modalLayer.replace({
                kind: "new-memory" as const,
                originBlockId: props.model.blockId,
                onCreated: (id: string) => {
                    modalLayer.replace(
                        buildLaunchRequest(agent, {
                            ...current,
                            memoryId: id,
                        }),
                    );
                },
                onCancel: () => {
                    modalLayer.replace(buildLaunchRequest(agent, current));
                },
            });
        },
    });

    const openLaunchModal = (agent: AgentDefinition) => {
        modalLayer.open(buildLaunchRequest(agent));
    };

    // Cascade follow-up (2026-05-23) — reattach via Recent Sessions.
    // We bypass the launch modal entirely (no Identity / Memory picks
    // to make — the row carries them) and call launchAgentDefinition
    // directly with the same `continueOfInstanceId + workDirOverride`
    // shape the modal's Continue dropdown produces. That keeps the
    // continuation path single-sourced: same backend RPCs, same
    // CreateAgentInstance lineage, same env injection. If the
    // definition for this row no longer exists (rare, but possible if
    // the user deleted it), fall back to the launch modal for the
    // first matching definition so the user can pick a substitute.
    // See: MyAgentsList.tsx file header for the full mechanism
    // discussion.
    const handleReattach = async (row: RecentSessionRow) => {
        const def = agents().find((a) => a.id === row.definition_id);
        if (!def) {
            // Definition gone — log + no-op rather than spawn into a
            // missing definition. The frontend logger forwards to the
            // sidecar so this shows up in muxlog. Surfacing a toast is
            // a UX polish follow-up; for the cascade-recovery flow,
            // not crashing the picker is the priority.
            // eslint-disable-next-line no-console
            console.warn(
                `recent-session reattach: definition ${row.definition_id} not found`,
            );
            return;
        }
        setLaunching(def.id);
        try {
            await props.model.launchAgentDefinition(def, {
                instanceName: row.instance_name,
                agentType: (def.agent_type as "host" | "container") || "host",
                environment: def.agent_type === "container" ? "docker" : "local",
                // #2463 Finding 1: a legacy row's identity_id can carry a
                // stale/legacy value from before the account-linking system
                // — forwarding it unfiltered as accountId crashes the
                // write-through's FOREIGN KEY insert (see
                // identity-carry-over.ts's header comment and
                // realAccountIdOrEmpty's above). memory_id does NOT need
                // this: unlike account_id it has no FK constraint, and
                // legitimate bundle ids are routinely non-UUID ("blank",
                // "seed-*" — memory_bundles.rs/bundle.rs) rather than legacy
                // garbage, so filtering it would silently drop a real
                // carry-over (reagent P2 on this PR).
                accountId: realAccountIdOrEmpty(row.identity_id),
                memoryId: row.memory_id,
                continueOfInstanceId: row.instance_id,
                workDirOverride: row.working_directory,
                // Carry the CLI-emitted session id forward so the new
                // block's spawn sees `--resume <sid>` on the FIRST
                // turn — otherwise the CLI starts a fresh session
                // and re-injects the startup context (the 2026-05-24
                // "click Maks → startup context replayed" report).
                continueSessionId: row.session_id ?? "",
            });
        } finally {
            setLaunching(null);
        }
    };

    const handleFork = async (row: RecentSessionRow, branchLabel: string): Promise<void> => {
        const def = agents().find((a) => a.id === row.definition_id);
        if (!def) return;
        setLaunching(row.definition_id);
        try {
            const forkedDef = await RpcApi.ForkAgentDefinitionCommand(TabRpcClient, {
                source_id: row.definition_id,
                branch_label: branchLabel,
            });
            // refreshOpenDefinitions after fork so the new def won't
            // also show as "active" before it's actually launched.
            refreshOpenDefinitions();
            await props.model.launchAgentDefinition(forkedDef, {
                instanceName: branchLabel,
                agentType: (forkedDef.agent_type as "host" | "container") || "host",
                environment: forkedDef.agent_type === "container" ? "docker" : "local",
                // #2463 Finding 1 — see handleReattach's comment above.
                // memoryId intentionally unfiltered — same reasoning.
                accountId: realAccountIdOrEmpty(row.identity_id),
                memoryId: row.memory_id,
            });
        } finally {
            setLaunching(null);
        }
    };

    const handleSwitchToExisting = (blockId: string): void => {
        refocusNode(blockId);
    };

    // Extracted from the install branch of handleSelect so the
    // prereq modal's "Launch anyway" path can route to install when
    // needed. Same shape as the existing inline install request.
    const buildInstallRequest = (agent: AgentDefinition) => ({
        kind: "install-agent" as const,
        agent,
        originBlockId: props.model.blockId,
        onInstalled: (continueToLaunch: boolean) => {
            const canonical = getProvider(agent.provider)?.id ?? agent.provider;
            setInstallState((s) => {
                const next = { ...s };
                for (const a of agents()) {
                    if ((getProvider(a.provider)?.id ?? a.provider) === canonical) {
                        next[a.id] = true;
                    }
                }
                return next;
            });
            if (continueToLaunch) {
                modalLayer.replace(buildLaunchRequest(agent));
            } else {
                modalLayer.close();
            }
        },
    });

    // Clicking the card opens either the install or launch modal in
    // the tab-scoped layer, depending on whether the agent's CLI is
    // already installed in the per-version cache. Phase α of
    // SPEC_AGENT_INSTALL_STAGE_2026_05_17.md.
    //
    // Re-entry guard: handleSelect awaits an IPC round-trip when
    // install state is still undefined. Without the in-flight set,
    // a double-click during the await would spawn two install
    // modals — the second mount replaces the first, and its
    // onCleanup fires `install.cancel` on the session the first one
    // had just kicked off.
    // System-tool prereq probe + modal. Phase α of
    // SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md. Caches per-tool
    // probe results for the session (PATH doesn't change mid-session
    // unless the user installs a new tool — `Refresh` clears).
    const prereqCache = new Map<string, boolean>();
    const platformKey = (): "windows" | "macos" | "linux" => {
        // Read from the CEF host's authoritative `getPlatform()` IPC
        // rather than the deprecated `window.navigator.platform` —
        // reagent P2 on PR #908.
        switch (getPlatform()) {
            case "win32": return "windows";
            case "darwin": return "macos";
            default: return "linux";
        }
    };
    const probeMissingPrereqs = async (agent: AgentDefinition) => {
        const prov = getProvider(agent.provider);
        const reqs = prov?.systemPrereqs ?? [];
        if (reqs.length === 0) return [];
        const uncached = reqs.filter((r) => !prereqCache.has(r.tool));
        if (uncached.length > 0) {
            try {
                const r = await RpcApi.ResolvePrereqsCommand(TabRpcClient, {
                    tools: uncached.map((u) => u.tool),
                });
                for (const result of r.results) {
                    prereqCache.set(result.tool, result.found);
                }
            } catch {
                // If the probe fails (e.g. backend not ready), treat
                // all as found — don't block launch on probe failure.
                for (const u of uncached) prereqCache.set(u.tool, true);
            }
        }
        const platform = platformKey();
        return reqs
            .filter((r) => prereqCache.get(r.tool) === false)
            .map((r) => ({
                tool: r.tool,
                label: r.label ?? r.tool,
                installUrl: r.installUrls[platform],
                installLinkText:
                    r.installLinkText?.[platform] ?? `Install ${r.label ?? r.tool}`,
            }));
    };

    // Option E (PR 2 of 2): when an agent has an in-progress session
    // zone (`agent:<defId>:current`), a plain click on the card
    // auto-continues into that session — no launch-modal round-trip.
    // Modifier keys (Shift / Ctrl / Alt / Cmd) are the escape hatch:
    // they force the launch modal even when a session exists, so the
    // user can still pick a different identity / memory / runtime.
    // The "+ New" button on the card archives the current zone and
    // routes through `openLaunchModal` for a fresh start.
    const autoContinue = async (agent: AgentDefinition) => {
        setLaunching(agent.id);
        try {
            // Account / memory are launch-time picks that aren't
            // stored on the AgentDefinition itself — they live on the
            // last NamedAgentRow for this definition. We auto-continue
            // by reusing the most-recent instance's pair so the spawn
            // resolves credentials the same way as the previous run.
            // Empty strings are fine: the backend resolver treats them
            // as "use ambient credentials".
            let accountId = "";
            let memoryId = "";
            try {
                const rows = await RpcApi.ListNamedAgentsCommand(TabRpcClient, {
                    definition_id: agent.id,
                }) ?? [];
                const mostRecent = [...rows].sort(
                    (a, b) => (b.started_at ?? 0) - (a.started_at ?? 0),
                )[0];
                if (mostRecent) {
                    // See identity-carry-over.ts — a UUID-shape check,
                    // not a per-literal blacklist, so legacy sentinels
                    // like "default" (which would otherwise fail
                    // linkagentidentity's FOREIGN KEY constraint) are
                    // filtered along with "blank"/"".
                    accountId = looksLikeRealAccountId(mostRecent.identity_id) ? mostRecent.identity_id : "";
                    memoryId = looksLikeRealAccountId(mostRecent.memory_id) ? mostRecent.memory_id : "";
                }
            } catch {
                // best-effort — fall through with empty bundles.
            }
            await props.model.launchAgentDefinition(agent, {
                instanceName: agent.name,
                agentType: (agent.agent_type as "host" | "container") || "host",
                environment: agent.agent_type === "container" ? "docker" : "local",
                accountId,
                memoryId,
                // No `continueOfInstanceId` — the agent's session
                // zone IS continuous now (E1, PR #1007). The new pane
                // reads from `agent:<defId>:current` on mount.
            });
            if (props.model.nodejsError) {
                setNodejsError(props.model.nodejsError);
                props.model.nodejsError = null;
            }
        } finally {
            setLaunching(null);
        }
    };

    // (`handleNewSession` removed in same cleanup as `handleSelect` —
    // templates can't show "+ New session" pills under Phase 1
    // invariant `hasCurrentSession={false}`, so the callback was
    // unreachable. Re-introduce when a tier renders my-agent rows
    // with the pill.)

    // Phase 1 two-tier picker: clicking a template card opens the
    // "Create new agent from {Template}" modal. The layer chains
    // `agentdefcreatefromtemplate` → `launchAgentDefinition` so both
    // RPCs are covered by a single `submitting()` gate (ESC + backdrop
    // dismiss stay blocked until the new agent is fully launched).
    const openCreateFromTemplateModal = (template: AgentDefinition) => {
        modalLayer.open({
            kind: "create-from-template" as const,
            template,
            originBlockId: props.model.blockId,
            onCreatedAndLaunch: async (newDefId, accountIdSel, memoryIdSel, name, agentType) => {
                // The new definition is user-owned and carries the
                // template's provider + cmd config. Build an
                // AgentDefinition stub good enough for the launch flow
                // — it only reads `id`, `name`, `agent_type`. (The
                // canonical row will be re-fetched by the launch
                // pipeline via SQL; we don't need every column here.)
                setLaunching(newDefId);
                try {
                    // Reagent P1 on #1011 round 2: the previous stub
                    // spread `template` directly, which leaked the
                    // template's `slug` and `working_directory` into
                    // the launch path. Backend `agentdefcreatefromtemplate`
                    // deliberately initialises those fields empty on
                    // the new row so per-agent values are derived
                    // server-side; the stub MUST match that contract,
                    // otherwise the new agent inherits template-scoped
                    // state (e.g. shared `GH_CONFIG_DIR`, cwd path).
                    const stubAgent: AgentDefinition = {
                        ...template,
                        id: newDefId,
                        name,
                        is_seeded: 0,
                        parent_id: template.id,
                        slug: "",
                        working_directory: "",
                        // Reflect the runtime the user picked in the
                        // modal, not the template's — the template is
                        // runtime-agnostic and the backend persisted this
                        // choice on the new row.
                        agent_type: agentType,
                        environment: agentType === "container" ? "docker" : "local",
                    };
                    await props.model.launchAgentDefinition(stubAgent, {
                        instanceName: name,
                        agentType,
                        environment: agentType === "container" ? "docker" : "local",
                        accountId: accountIdSel,
                        memoryId: memoryIdSel,
                        // No `continueOfInstanceId` — the new definition
                        // has no prior session; its agent-anchored zone
                        // is empty and the pane will start fresh.
                    });
                    if (props.model.nodejsError) {
                        setNodejsError(props.model.nodejsError);
                        props.model.nodejsError = null;
                    }
                } finally {
                    setLaunching(null);
                }
            },
        });
    };

    // Cross-tier re-entry guard. Both `handleSelect` (My Agents) and
    // `handleTemplateSelect` (Templates) check + insert into this set
    // so a rapid double-click across tiers can't spawn two parallel
    // install/launch flows for the same definition.
    const pendingSelect = new Set<string>();

    // Phase 1 two-tier picker: template-card click path. Mirrors
    // `handleSelect` for the install + prereq gates (a template's CLI
    // can still need installing) but drops the auto-continue branch —
    // templates carry no session zone post-migration. On success it
    // hands off to `openCreateFromTemplateModal` instead of
    // `openLaunchModal`.
    const handleTemplateSelect = async (
        agent: AgentDefinition,
        _evt?: MouseEvent | KeyboardEvent,
    ) => {
        if (pendingSelect.has(agent.id)) return;
        pendingSelect.add(agent.id);
        try {
            setNodejsError(null);

            let installed = installState()[agent.id];
            if (installed === undefined) {
                const prov = getProvider(agent.provider);
                const isNpmInstallable = !!prov?.npmPackage && prov.npmPackage.length > 0;
                if (isNpmInstallable) {
                    await checkInstalled(agent);
                    installed = installState()[agent.id];
                }
            }

            const missing = await probeMissingPrereqs(agent);
            if (missing.length > 0) {
                const proceedWithFlow = () => {
                    if (installed === false) {
                        // Reagent P1 round 2: do NOT use the generic
                        // `buildInstallRequest` here — its `onInstalled`
                        // routes to `buildLaunchRequest(agent)` which
                        // launches the seeded template directly,
                        // defeating the template-clone migration. Open
                        // the same template-aware install request the
                        // non-prereq branch (line ~545) uses.
                        modalLayer.replace(buildTemplateInstallRequest(agent));
                    } else {
                        openCreateFromTemplateModal(agent);
                    }
                };
                const openPrereqModal = (
                    currentMissing: typeof missing,
                    op: "open" | "replace",
                ): void => {
                    const refresh = async () => {
                        for (const m of currentMissing) prereqCache.delete(m.tool);
                        const fresh = await probeMissingPrereqs(agent);
                        if (fresh.length === 0) {
                            proceedWithFlow();
                        } else {
                            openPrereqModal(fresh, "replace");
                        }
                    };
                    const req = {
                        kind: "agent-prereqs" as const,
                        agent,
                        originBlockId: props.model.blockId,
                        missing: currentMissing,
                        onRefresh: () => void refresh(),
                        onProceed: proceedWithFlow,
                        onCancel: () => {},
                    };
                    if (op === "open") modalLayer.open(req);
                    else modalLayer.replace(req);
                };
                openPrereqModal(missing, "open");
                return;
            }

            if (installed === false) {
                modalLayer.open(buildTemplateInstallRequest(agent));
                return;
            }

            openCreateFromTemplateModal(agent);
        } finally {
            pendingSelect.delete(agent.id);
        }
    };

    // Template-aware install request. The generic `buildInstallRequest`
    // routes `onInstalled → buildLaunchRequest(agent)` which launches
    // the seeded template directly — defeats the template-clone
    // migration. This variant routes the success path into
    // `openCreateFromTemplateModal` so the install→create-clone→launch
    // chain stays correct (reagent P1 on #1011 round 2).
    const buildTemplateInstallRequest = (agent: AgentDefinition) => ({
        kind: "install-agent" as const,
        agent,
        originBlockId: props.model.blockId,
        onInstalled: (continueToLaunch: boolean) => {
            const canonical = getProvider(agent.provider)?.id ?? agent.provider;
            setInstallState((s) => {
                const next = { ...s };
                for (const a of agents()) {
                    if ((getProvider(a.provider)?.id ?? a.provider) === canonical) {
                        next[a.id] = true;
                    }
                }
                return next;
            });
            if (continueToLaunch) {
                openCreateFromTemplateModal(agent);
            } else {
                modalLayer.close();
            }
        },
    });

    // Note: the legacy `handleSelect` (Option E PR-2 default-click
    // handler) was removed in the Phase 1 cleanup. Every rendered
    // card now goes through `handleTemplateSelect`; my-agent rows are
    // handled by `MyAgentsList`'s own `handleReattach` callback. The
    // install/prereq gates that `handleSelect` owned now live inside
    // `handleTemplateSelect` directly. Don't resurrect a single shared
    // `handleSelect` — fan out per tier (reagent P2 on #1011).

    // Phase 2 (Q2 Decision Y — hide templates): right-click on a
    // template card opens a context menu with "Hide template". The
    // card disappears from the picker the next render after the RPC
    // resolves — `listagents` filters by `user_hidden` server-side,
    // and `agents:changed` (broadcast by the backend after hide)
    // refetches the list.
    //
    // Only seeded templates get this context menu — user-owned rows
    // belong to `MyAgentsList`, which has its own affordances and
    // never funnels through this handler.
    const handleTemplateContextMenu = (
        agent: AgentDefinition,
        evt: MouseEvent,
    ) => {
        evt.preventDefault();
        const caption = agent.name || agent.slug || agent.id;
        ContextMenuModel.showContextMenu(
            [
                {
                    label: `Hide template "${caption}"`,
                    click: () => {
                        void (async () => {
                            try {
                                await RpcApi.AgentDefHideCommand(TabRpcClient, {
                                    definition_id: agent.id,
                                });
                            } catch (err) {
                                // Backend rejects only when the row
                                // isn't a template — should be
                                // unreachable from this menu, but log
                                // to muxlog if it ever fires.
                                // eslint-disable-next-line no-console
                                console.warn(
                                    `agentdefhide failed for ${agent.id}:`,
                                    err,
                                );
                            }
                        })();
                    },
                },
            ],
            evt,
        );
    };

    const busy = () => launching() !== null;

    // Phase 1 two-tier picker: partition the agent list into the
    // "+ New from template" tier (seeded templates) and the section
    // implicitly handled by `MyAgentsList` (user-owned, surfaced as
    // recent sessions via `ListRecentSessionsCommand`). The card grid
    // below the My Agents list renders only the templates tier.
    const templates = createMemo(() =>
        agents().filter((a) => a.is_seeded === 1),
    );

    // Refresh install state whenever the agent list changes.
    // Session-zone probes were removed in the Phase 1 cleanup
    // (reagent P2 on #1011) — templates carry `hasCurrentSession={false}`
    // by invariant and my-agent rows source from `MyAgentsList` which
    // doesn't need per-row session probes.
    createEffect(() => {
        for (const agent of agents()) {
            if (!(agent.id in installState())) {
                void checkInstalled(agent);
            }
        }
    });

    // Per-pane zoom — read `term:zoom` from block meta and apply CSS
    // `zoom` on the root, matching AgentPresentationView (chat mode).
    // The universal framework (Ctrl+/-/0 in keymodel, Ctrl+Wheel in
    // app.tsx) writes `term:zoom` for any block whose `viewType ===
    // "agent"`; the picker just needs to read + apply.
    const block = props.model.blockAtom;
    const zoomFactor = createMemo(() => {
        const z = block()?.meta?.["term:zoom"];
        if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
        return Math.max(0.5, Math.min(2.0, z));
    });

    return (
        <>
            <Show
                when={agents().length > 0}
                fallback={
                    <div class="agent-view" style={{ zoom: zoomFactor() }}>
                        <div class="agent-picker-empty">
                            <div class="agent-picker-empty-icon">{"\u2726"}</div>
                            <div class="agent-picker-empty-title">No definitions configured</div>
                            <div class="agent-picker-empty-desc">
                                Use the ⚙ Agent settings to add your first definition.
                            </div>
                        </div>
                        <AgentActionBar />
                    </div>
                }
            >
                <div class="agent-view" style={{ zoom: zoomFactor() }}>
                    <div class="agent-picker">
                        {/* Two-tier picker — Phase 1
                            (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md).
                            Top: user-owned agents from
                            `ListRecentSessionsCommand` with their
                            current Option E session state.
                            Bottom: seeded templates, click = open the
                            create-from-template modal. */}
                        <MyAgentsList
                            onReattach={handleReattach}
                            openDefinitions={openDefinitions}
                            onFork={handleFork}
                            onSwitchToExisting={handleSwitchToExisting}
                        />
                        <div class="agent-picker-templates-header" data-testid="agent-templates-header">
                            <span>+ New from template</span>
                        </div>
                        <div class="agent-picker-list" data-testid="agent-templates-list">
                            <For each={templates()}>
                                {(agent, index) => (
                                    <AgentCard
                                        agent={agent}
                                        launching={launching() === agent.id}
                                        disabled={busy()}
                                        installed={installState()[agent.id]}
                                        onLaunch={handleTemplateSelect}
                                        // Phase 2: right-click → Hide
                                        // template (Q2 Decision Y). Only
                                        // attached for template cards;
                                        // my-agent rows live in
                                        // MyAgentsList and have a
                                        // different action surface.
                                        onContextMenu={handleTemplateContextMenu}
                                        // Templates by invariant have
                                        // no session zone — suppress
                                        // the "+ New" pill (no
                                        // `onNewSession` needed; the
                                        // pill never appears).
                                        hasCurrentSession={false}
                                        defaultFocus={index() === 0}
                                    />
                                )}
                            </For>
                        </div>
                        {/* Phase 2 (Q2 Decision Y): collapsible
                            "Hidden templates" section, lazy-loaded so
                            it doesn't add work when no templates are
                            hidden. Sits under the templates tier so
                            hide + unhide live in the same surface
                            (CLAUDE.md notes that the hamburger
                            Settings menu just opens settings.json,
                            so there's no separate settings panel to
                            host this in). */}
                        <HiddenTemplatesSection />

                        <Show when={nodejsError()}>
                            <div class="agent-nodejs-notice">
                                <div class="nodejs-notice-icon">
                                    <i class="fa-solid fa-circle-exclamation" />
                                </div>
                                <div class="nodejs-notice-content">
                                    <div class="nodejs-notice-title">Node.js Required</div>
                                    <div class="nodejs-notice-text">{nodejsError()}</div>
                                    <div class="nodejs-notice-hint">
                                        After installing, restart AgentMux and try again.
                                    </div>
                                </div>
                            </div>
                        </Show>
                    </div>
                    <AgentActionBar />
                </div>
            </Show>
        </>
    );
};

AgentPicker.displayName = "AgentPicker";
