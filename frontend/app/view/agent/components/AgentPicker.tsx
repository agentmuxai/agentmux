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
 *    (`is_seeded = 1`). Click = open its panel below the card's row
 *    (`AgentCreatePanel`), which clones the template into a new user
 *    agent and launches it (SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md).
 *
 * Templates by Phase 1 invariant carry NO session zone (the startup
 * migration `migrate_promote_template_sessions_v1` evicts any
 * pre-existing template-session into a user agent). The template
 * cards therefore never show the "+ New" pill or auto-continue.
 *
 * Pre-existing flows kept intact:
 *  - Modifier-key force-modal on a My Agents row (Shift / Ctrl / Alt /
 *    Cmd) bypasses auto-continue.
 *  - The tools a harness needs and its CLI's install come first, as
 *    steps of the same panel, when they apply.
 *
 * See docs/specs/SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md (legacy)
 * and docs/specs/SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md (current).
 */

import { PaneLoadingCover } from "@/app/element/PaneLoadingCover";
import { CopyErrorButton } from "@/app/errors/CopyErrorButton";
import { formatErrorReport } from "@/app/errors/error-report";
import { noteOpenedByPerson } from "@/app/store/agent-open-note";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { trackPaneContent } from "@/app/store/pane-content-holds";
import { createPaneReadiness } from "@/app/store/pane-readiness";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { readZoom } from "@/app/store/zoom-factor";
import { revealBlock } from "@/app/util/reveal-block";
import { refreshAccountCache } from "@/app/view/identity/identity-model";
import { useWindowTabDisplayed } from "@/app/workspace/window-tab-visibility";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { getPlatform } from "@/util/platformutil";
import { createEffect, createMemo, createSignal, For, onCleanup, Show, untrack, type JSX } from "solid-js";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
import { copyForkHistory } from "../quick-fork";
import type { AgentViewModel } from "../agent-model";
import { realAccountIdOrEmpty } from "../identity-carry-over";
import { checkProviderInstalled, invalidateInstallCheck, onInstallCheckInvalidated } from "../install-check-cache";
import { isAgentInstalling } from "../install-sessions";
import { AgentCreatePanel } from "./AgentCreatePanel";
import { AgentCardFilters } from "./AgentCardFilters";
import { matchesFilters, type CardFilter, type FilterableCard } from "../defaults/provider-access-model";
import { createCreateDraft, type CreateDraft, type CreatedAgent } from "./AgentCreateForm";
import { CardGrid } from "@/app/element/ui/CardGrid";
import { openOrFocusHistoryTab } from "../open-history-tab";
import { beginAgentOpen, finishAgentOpen } from "../open-trace";
import { getProvider, getProviderList } from "../providers";
import { openAgentInActiveTab, sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { getApi, pushNotification } from "@/app/store/global";
import { needsProbe, prereqShortfall, shortfallLabel, type PrereqProbe } from "../providers/prereq-check";
import { resolveDrift, type PinDrift } from "../providers/version-drift";
import { AgentCard } from "./AgentCard";
import { AgentPickerFilterBar, DEFAULT_AGENT_SORT, type AgentSortOption } from "./AgentPickerFilterBar";
import { HiddenTemplatesSection } from "./HiddenTemplatesSection";
import { MyAgentsList } from "./MyAgentsList";

import { MENU_ICON } from "@/app/util/menu-icons";
/** This-machine-only preference — no cross-device sync, no existing
 *  localStorage precedent in this component tree to extend (a new small
 *  pattern, not a reuse). Wrapped defensively: localStorage can throw in
 *  some embedding contexts (private browsing, storage quota), and a sort
 *  preference is never worth a hard failure over. */
const AGENT_SORT_STORAGE_KEY = "agentPicker:sortBy";

function loadStoredSort(): AgentSortOption {
    try {
        const raw = localStorage.getItem(AGENT_SORT_STORAGE_KEY);
        if (raw === "recent" || raw === "name" || raw === "type") return raw;
    } catch {
        // ignore — fall through to the default
    }
    return DEFAULT_AGENT_SORT;
}

function storeSort(sort: AgentSortOption): void {
    try {
        localStorage.setItem(AGENT_SORT_STORAGE_KEY, sort);
    } catch {
        // best-effort only — a failed write just means the preference
        // doesn't survive to the next picker open, not a functional break
    }
}

import { useAgentDefinitions, useInstanceOpenDefinitions, useOpenDefinitionMap } from "./agent-definition-hooks";
export { useAgentDefinitions, useOpenDefinitionMap };

// ── AgentPicker component ───────────────────────────────────────────────────────

interface AgentPickerProps {
    model: AgentViewModel;
}

export const AgentPicker = (props: AgentPickerProps): JSX.Element => {
    const [launching, setLaunching] = createSignal<string | null>(null);
    const [nodejsError, setNodejsError] = createSignal<string | null>(null);
    // Identity M4b-3: a launch that aborted (its row could not be recorded).
    const [launchError, setLaunchError] = createSignal<string | null>(null);
    // Taken after every launch attempt, so a later attempt that did not
    // abort clears the notice instead of leaving it up (review of #3583).
    const takeLaunchError = () => {
        setLaunchError(props.model.launchError);
        props.model.launchError = null;
    };
    // Filter-bar query (SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md) —
    // narrows MyAgentsList only, not the template grid below it (Q1,
    // confirmed by the human operator): the ask was specifically to find
    // an *existing agent*, and a shared query risks the template grid
    // silently shrinking for an unrelated reason right next to it.
    const [filterQuery, setFilterQuery] = createSignal("");
    // Sort control, far right of the filter bar (SPEC follow-up — see
    // docs/reports/REPORT_AGENT_PICKER_FIELD_ORDER_SORT_AND_DATA_GAPS_AUDIT_2026_08_24.md §3).
    // Same narrowing-scope decision as filterQuery above: only MyAgentsList,
    // not the template grid.
    const [sortBy, setSortByState] = createSignal<AgentSortOption>(loadStoredSort());
    const setSortBy = (sort: AgentSortOption) => {
        setSortByState(sort);
        storeSort(sort);
    };
    const [agents, definitionsLoading] = useAgentDefinitions();


    // Combined "picker content is ready to reveal" gate — true once BOTH
    // independent async sources feeding this view have resolved their
    // FIRST load: the definitions list (definitionsLoading above) and
    // MyAgentsList's own ListRecentSessionsCommand resource (reported back
    // via onFirstLoad below, since MyAgentsList owns that resource
    // internally). Neither alone was sufficient — gating only on
    // definitionsLoading still let MyAgentsList's own empty `<ul>` flash
    // before its rows arrived; gating only on MyAgentsList's load left the
    // OUTER `<Show when={agents().length > 0}>` free to flash the "No
    // definitions configured" empty state first for users who actually
    // have agents, simply because `agents()` starts at `[]` and is
    // indistinguishable from "genuinely zero" without definitionsLoading.
    // One overlay, held until both are done, replaces two independent
    // partial-content flashes with a single reveal — same principle as
    // agent-view.tsx's pane-level loading overlay (one gate, one reveal),
    // applied here across two sibling data sources instead of one.
    const [myAgentsLoaded, setMyAgentsLoaded] = createSignal(false);
    // A genuinely-empty definitions list never mounts MyAgentsList at all (it
    // lives inside the `agents().length > 0` branch below) — so `loaded`
    // must not wait on `myAgentsLoaded` in that case, or the overlay would
    // spin forever for a user with zero agent definitions.
    const pickerReady = createMemo(() => {
        if (definitionsLoading()) return false;
        if (agents().length === 0) return true;
        return myAgentsLoaded();
    });

    // Hold-then-fade, now expressed through the shared readiness controller
    // (SPEC_PANE_LOADING_CONSOLIDATION_2026_09_20.md phase 2). Mechanics are
    // unchanged — one gate, released when `pickerReady()` flips, then one CSS fade
    // duration before unmount so the overlay's removal is never a pop.
    //
    // What changes is ownership: this component used to render
    // `.agent-pane-loading-overlay` itself, with its own ready signal and its own
    // unmount timer, while agent-view.tsx rendered the SAME class with its own —
    // two covers for one pane, whichever unmounted last winning. Both now go
    // through <PaneLoadingCover>, so the class has a single owner.
    const windowTabDisplayed = useWindowTabDisplayed();
    // The picker's loading is all behind this cover (pane-content-holds.ts).
    onCleanup(trackPaneContent(props.model.blockId));
    const pickerReadiness = createPaneReadiness({
        label: "agent-picker",
        holdFor: props.model.blockId,
        hidden: () => !windowTabDisplayed(),
    });
    const releasePickerGate = pickerReadiness.gate("picker-content");
    let pickerOverlayFadeTimeout: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(pickerOverlayFadeTimeout));
    createEffect(() => {
        if (pickerReady()) releasePickerGate();
    });
    createEffect(() => {
        if (pickerReadiness.phase() === "revealing") {
            pickerOverlayFadeTimeout = setTimeout(() => pickerReadiness.revealComplete(), 220);
        }
    });

    // Reactive map of definition_id → blockId for panes currently open.
    const [openDefinitions, refreshOpenDefinitions, openLocations, resolveOpenDefinitions] =
        useInstanceOpenDefinitions();

    // Per-agent install state, keyed by agent.id: the same answer the
    // Toolchain pane shows (one `install.check`, one backend lookup).
    //   undefined = not yet checked
    //   true      = found: AgentMux's install, or the user's own copy of a
    //               CLI AgentMux doesn't install
    //   false     = not found — card shows the bottom-right ribbon
    const [installState, setInstallState] = createSignal<Record<string, boolean | undefined>>({});
    /** Per-definition CLI drift, resolved alongside the install check (same
     *  round-trip). Absent = not checked yet, or a provider with no pin. */
    const [driftState, setDriftState] = createSignal<Record<string, PinDrift | undefined>>({});

    // Phase 1 two-tier picker (reagent P2 on #1011): the auto-continue
    // session-state cache + probe lived here for the Option E PR-2
    // default-click path. After the two-tier rewrite, templates carry
    // `hasCurrentSession={false}` by invariant and `handleSelect` is
    // unreachable from any card render, so the cache + probe + the
    // entire session-state branch became dead. Deleted. If a future
    // tier needs per-agent session probing, re-introduce the cache
    // there scoped to that tier's data source — not here.

    /** Each checked agent's resolved provider, to re-check only the cards an
     *  invalidation concerns. */
    const providerOfAgent = new Map<string, string>();
    const checkInstalled = async (agent: AgentDefinition) => {
        // Same resolver as the launch (agent-model.ts), so the CLI checked
        // here is the one that will run (#2594).
        const providerId = await resolveEffectiveLaunchProvider(agent);
        const prov = getProvider(providerId);
        if (prov) providerOfAgent.set(agent.id, prov.id);
        if (!prov) {
            setInstallState((s) => ({ ...s, [agent.id]: undefined }));
            return;
        }
        try {
            // One check per provider CLI, shared across agents, pickers and
            // the Toolchain pane. A CLI AgentMux doesn't install is checked
            // too: found on PATH or in its installer's folder, or the ribbon.
            const r = await checkProviderInstalled({
                providerId: prov.id,
                cliCommand: prov.cliCommand,
                // Lets the same round-trip answer "which version", so the card
                // can show an upgrade hint without a second probe per card.
                npmPackage: prov.npmPackage || undefined,
            });
            setInstallState((s) => ({ ...s, [agent.id]: r.installed }));
            // Same predicate the Toolchain pane renders — the card must not
            // develop its own opinion about what "behind" means.
            setDriftState((s) => ({
                ...s,
                [agent.id]: resolveDrift({
                    installed: r.version ?? undefined,
                    pinned: prov.pinnedVersion,
                    found: r.installed,
                }).drift,
            }));
        } catch {
            // Treat as needs-install — better to over-prompt than miss.
            setInstallState((s) => ({ ...s, [agent.id]: false }));
        }
    };

    // Cascade follow-up (2026-05-23) — reattach via Recent Sessions.
    // No form: the row carries the account and bundle, so this calls
    // launchAgentDefinition directly with `continueOfInstanceId +
    // workDirOverride`. That keeps the continuation path single-sourced:
    // same backend RPCs, same CreateAgentInstance lineage, same env
    // injection. If the row's definition no longer exists (rare, but
    // possible if the user deleted it), it logs and does nothing.
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
            console.warn(`recent-session reattach: definition ${row.definition_id} not found`);
            return;
        }
        setLaunching(def.id);
        const openBlockId = props.model.blockId;
        beginAgentOpen(openBlockId, def.name, "my-agents");
        // The person opened it from My Agents: the resumed agent opens with its update.
        noteOpenedByPerson(def.id);
        try {
            const launched = await props.model.launchAgentDefinition(def, {
                instanceName: row.instance_name,
                agentType: (def.agent_type as "host" | "container") || "host",
                environment: def.agent_type === "container" ? "docker" : "local",
                // #2463 Finding 1: a legacy row's identity_id can carry a
                // stale/legacy value from before the account-linking system
                // — forwarding it unfiltered as accountId crashes the
                // write-through's FOREIGN KEY insert (see
                // identity-carry-over.ts's realAccountIdOrEmpty).
                // refreshAccountCache() (not the synchronous loadAccounts()
                // cache, which can still be mid-priming — reagentx P2 on
                // #2464) guarantees a fresh list to check against.
                // memory_id does NOT need this: unlike account_id it has no
                // FK constraint, and legitimate bundle ids are routinely
                // non-UUID ("blank", "seed-*" — storage/bundles.rs /
                // app_api/bundle/mod.rs) rather than legacy garbage, so
                // filtering it would silently drop a real carry-over.
                accountId: realAccountIdOrEmpty(
                    row.identity_id,
                    (await refreshAccountCache()).map((a) => a.id)
                ),
                bundleId: row.memory_id,
                continueOfInstanceId: row.instance_id,
                workDirOverride: row.working_directory,
                // Carry the CLI-emitted session id forward so the new
                // block's spawn sees `--resume <sid>` on the FIRST
                // turn — otherwise the CLI starts a fresh session
                // and re-injects the startup context (the 2026-05-24
                // "click Maks → startup context replayed" report).
                continueSessionId: row.session_id ?? "",
            });
            if (!launched) finishAgentOpen(openBlockId, "failed");
            takeLaunchError();
        } catch (e) {
            finishAgentOpen(openBlockId, "failed");
            throw e;
        } finally {
            setLaunching(null);
        }
    };

    // Each agent's runtime state, shared with the status bar and the Swarm.
    const runtimeRows = sharedAgentRuntime().rows;
    const runtimeById = createMemo(() => new Map(runtimeRows().map((r) => [r.agent_id, r] as const)));
    // An agent that runs with no pane: attach to it rather than start another process.
    const handleAttach = async (row: RecentSessionRow): Promise<void> => {
        const why = await openAgentInActiveTab({
            agent_id: row.definition_id,
            name: row.instance_name || row.definition_name,
        });
        if (why) {
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: `Couldn't open ${row.instance_name || row.definition_name}`,
                message: why,
                timestamp: new Date().toISOString(),
                type: "error",
                expiration: Date.now() + 8000,
            });
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
            // A fork is an exact copy: its pane opens with the parent's
            // history, when it continues the parent's conversation.
            if (row.session_id) {
                const providerId = getProvider(await resolveEffectiveLaunchProvider(forkedDef))?.id ?? "";
                await copyForkHistory(forkedDef.id, row.definition_id, row.session_id, providerId);
            }
            await props.model.launchAgentDefinition(forkedDef, {
                instanceName: branchLabel,
                agentType: (forkedDef.agent_type as "host" | "container") || "host",
                environment: forkedDef.agent_type === "container" ? "docker" : "local",
                // #2463 Finding 1 — see handleReattach's comment above.
                // bundleId intentionally unfiltered — same reasoning.
                accountId: realAccountIdOrEmpty(
                    row.identity_id,
                    (await refreshAccountCache()).map((a) => a.id)
                ),
                bundleId: row.memory_id,
                // Carry the parent conversation's history forward — without
                // these two, forking only clones the agent *definition*
                // (config/instructions/skills) and starts a brand new
                // conversation, silently dropping the whole point of a
                // "fork" (SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md §2).
                // `forkSession: true` is what makes launchAgentDefinition
                // append `--fork-session` (Claude only; every other
                // provider's launchAgentDefinition ignores both fields and
                // falls back to a true fresh start — reagent + Codex's
                // review of PR #2725 found that used to not hold: passing
                // `forkSession` with an empty session id pushed a bare
                // `--fork-session` with nothing to resume from, and for a
                // non-Claude provider `continueSessionId` alone (with
                // `forkSession` silently ignored) resumed the SAME live
                // session as the parent instead of falling back — both
                // fixed at the source in launchAgentDefinition, and guarded
                // here too: only request a fork when there's an actual
                // session to fork from.
                ...(row.session_id ? { continueSessionId: row.session_id, forkSession: true } : {}),
            });
        } finally {
            setLaunching(null);
        }
    };

    // The agent may be in another window tab, or a background tab of a
    // multi-tab pane: revealBlock switches to it (SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md).
    const handleSwitchToExisting = (blockId: string): void => {
        void revealBlock(blockId, { source: "picker" });
    };

    // My Agents row menu → View History (docs/specs/SPEC_AGENT_DELETE_2026_09_16.md
    // §4.3). openOrFocusHistoryTab needs a "current pane" to push the
    // history tab onto as a sibling tab — this picker's OWN blockId (it's
    // the blank/picker tab itself, pre-launch) is exactly that pane.
    const handleViewHistory = (row: RecentSessionRow): void => {
        void openOrFocusHistoryTab({
            currentBlockId: props.model.blockId,
            agentId: row.definition_id,
            provider: row.provider,
            agentName: row.definition_name,
        });
    };

    // System-tool prereq probe. Phase α of
    // SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md. Caches per-tool
    // probe results for the session (PATH doesn't change mid-session
    // unless the user installs a new tool; the panel's Check again clears).
    const prereqCache = new Map<string, PrereqProbe>();
    const platformKey = (): "windows" | "macos" | "linux" => {
        // Read from the CEF host's authoritative `getPlatform()` IPC
        // rather than the deprecated `window.navigator.platform` —
        // reagent P2 on PR #908.
        switch (getPlatform()) {
            case "win32":
                return "windows";
            case "darwin":
                return "macos";
            default:
                return "linux";
        }
    };
    const probeMissingPrereqs = async (agent: AgentDefinition) => {
        // Same resolver as the launch (#2594), so the prereqs probed are
        // those of the provider that will run.
        const providerId = await resolveEffectiveLaunchProvider(agent);
        const prov = getProvider(providerId);
        const reqs = prov?.systemPrereqs ?? [];
        if (reqs.length === 0) return [];
        const uncached = reqs.filter((r) => needsProbe(r, prereqCache.get(r.tool)));
        if (uncached.length > 0) {
            try {
                const versions = uncached.filter((u) => u.minVersion).map((u) => u.tool);
                const r = await RpcApi.ResolvePrereqsCommand(TabRpcClient, {
                    tools: uncached.map((u) => u.tool),
                    ...(versions.length > 0 ? { versions } : {}),
                });
                for (const result of r.results) {
                    const wanted = uncached.some((u) => u.tool === result.tool && u.minVersion);
                    prereqCache.set(result.tool, {
                        found: result.found,
                        version: wanted ? (result.version ?? null) : undefined,
                    });
                }
            } catch {
                // If the probe fails (e.g. backend not ready), treat
                // all as found — don't block launch on probe failure.
                for (const u of uncached) prereqCache.set(u.tool, { found: true, version: null });
            }
        }
        const platform = platformKey();
        return reqs.flatMap((r) => {
            const shortfall = prereqShortfall(r, prereqCache.get(r.tool));
            if (!shortfall) return [];
            const label = r.label ?? r.tool;
            return [
                {
                    tool: r.tool,
                    label: shortfallLabel(r, shortfall),
                    installUrl: r.installUrls[platform],
                    installLinkText:
                        shortfall.kind === "too-old"
                            ? `Update ${label}`
                            : (r.installLinkText?.[platform] ?? `Install ${label}`),
                },
            ];
        });
    };

    // (`handleNewSession` removed in same cleanup as `handleSelect` —
    // templates can't show "+ New session" pills under Phase 1
    // invariant `hasCurrentSession={false}`, so the callback was
    // unreachable. Re-introduce when a tier renders my-agent rows
    // with the pill.)

    // ── New agent, inline ─────────────────────────────────────────────
    // A harness card opens its panel below its row, pushing the cards below
    // it down; one panel at a time. It walks through the tools the harness
    // needs, its CLI's install and the new agent's settings, in place of the
    // three modals it used to take (SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md
    // §2). What the user typed is kept per harness while the picker lives.
    const [openTemplate, setOpenTemplate] = createSignal<string | null>(null);
    /** The harnesses whose CLI AgentMux doesn't install (no npm package). */
    const userInstalledProviders = () => getProviderList().filter((p) => !p.npmPackage && p.docsUrl);
    // The open panel is creating an agent: it can't close or move meanwhile.
    const [panelBusy, setPanelBusy] = createSignal(false);
    const drafts = new Map<string, CreateDraft>();
    const draftFor = (template: AgentDefinition): CreateDraft => {
        let draft = drafts.get(template.id);
        if (!draft) {
            draft = createCreateDraft(template);
            drafts.set(template.id, draft);
        }
        return draft;
    };

    const handleTemplateSelect = (agent: AgentDefinition, _evt?: MouseEvent | KeyboardEvent) => {
        if (panelBusy()) return;
        setNodejsError(null);
        if (openTemplate() === agent.id) {
            setOpenTemplate(null);
            return;
        }
        // `checkInstalled` resolves through the bundle (#2594); the mount-time
        // check has normally answered already.
        if (installState()[agent.id] === undefined) void checkInstalled(agent);
        setOpenTemplate(agent.id);
    };

    /** Escape or Cancel in the open panel; refused while it creates an agent. */
    const closePanel = (): boolean => {
        if (panelBusy()) return false;
        setOpenTemplate(null);
        return true;
    };

    /** The system tools `agent`'s harness needs that aren't on PATH; `fresh` asks again. */
    const prereqsFor = async (agent: AgentDefinition, fresh: boolean) => {
        if (fresh) prereqCache.clear();
        return probeMissingPrereqs(agent);
    };

    /** Check `agent`'s CLI again, for every card and the Toolchain pane. */
    const recheckInstalled = async (agent: AgentDefinition) => {
        const providerId =
            providerOfAgent.get(agent.id) ?? getProvider(await resolveEffectiveLaunchProvider(agent))?.id;
        if (providerId) invalidateInstallCheck(providerId);
        await checkInstalled(agent);
    };

    /** Launch the agent the panel's form created from `template`. */
    const launchCreated = async (template: AgentDefinition, created: CreatedAgent) => {
        setLaunching(created.definitionId);
        try {
            // A stub good enough for the launch flow (it reads `id`, `name`,
            // `agent_type`; the pipeline re-fetches the row). It must not
            // spread the template's `slug` or `working_directory`: the backend
            // leaves them empty on the new row so per-agent values are derived
            // server-side (#1011). The slug is the one the backend gave the new
            // row, not one derived from the name (#3573).
            const stubAgent: AgentDefinition = {
                ...template,
                id: created.definitionId,
                name: created.name,
                is_seeded: 0,
                parent_id: template.id,
                slug: created.slug,
                working_directory: "",
                // The runtime the user picked; the template is runtime-agnostic
                // and the backend persisted the choice on the new row.
                agent_type: created.agentType,
                environment: created.agentType === "container" ? "docker" : "local",
            };
            await props.model.launchAgentDefinition(stubAgent, {
                instanceName: created.name,
                agentType: created.agentType,
                environment: created.agentType === "container" ? "docker" : "local",
                accountId: created.identityId,
                bundleId: created.memoryId,
                // No `continueOfInstanceId`: the new definition has no session.
                // The model the user picked (empty when the harness has no
                // list) seeds the first launch's agent:runtime block meta.
                model: created.model || undefined,
            });
            if (props.model.nodejsError) {
                setNodejsError(props.model.nodejsError);
                props.model.nodejsError = null;
            }
            takeLaunchError();
        } finally {
            setLaunching(null);
            // The agent exists now, whether or not it started (a launch can
            // stop short, e.g. for a missing Node.js, without throwing): close
            // the panel and forget what was typed, so a second Create can't
            // collide with it. It is in My Agents, and the notice above says
            // why it didn't start.
            drafts.delete(template.id);
            if (openTemplate() === template.id) setOpenTemplate(null);
        }
    };

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
    const handleTemplateContextMenu = (agent: AgentDefinition, evt: MouseEvent) => {
        evt.preventDefault();
        const caption = agent.name || agent.slug || agent.id;
        ContextMenuModel.showContextMenu(
            [
                {
                    label: `Hide template "${caption}"`,
                    icon: MENU_ICON.hide,
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
                                console.warn(`agentdefhide failed for ${agent.id}:`, err);
                            }
                        })();
                    },
                },
            ],
            evt
        );
    };

    const busy = () => launching() !== null;

    // Phase 1 two-tier picker: partition the agent list into the
    // "+ New from template" tier (seeded templates) and the section
    // implicitly handled by `MyAgentsList` (user-owned, surfaced as
    // recent sessions via `ListRecentSessionsCommand`). The card grid
    // below the My Agents list renders only the templates tier.
    const templates = createMemo(() => agents().filter((a) => a.is_seeded === 1));
    // The harnesses whose CLI is installed come first; each group keeps its
    // own order (the sort is stable). A card whose check is still out counts
    // as not installed, and moves up when its answer comes.
    const orderedTemplates = createMemo(() => {
        const installed = installState();
        return [...templates()].sort((a, b) => Number(installed[b.id] === true) - Number(installed[a.id] === true));
    });
    // The Free, Local and Installed chips above the cards narrow the grid (not remembered: a
    // filter that hides cards on its own next visit looks like cards disappearing).
    // SPEC_PROVIDER_CARD_TAGS_FREE_LOCAL_2026_10_29.md §6.
    const [cardFilters, setCardFilters] = createSignal<CardFilter[]>([]);
    const filterableCards = createMemo<FilterableCard[]>(() =>
        orderedTemplates().map((a) => ({ provider: a.provider, installed: installState()[a.id] }))
    );
    const installedKnown = createMemo(() => orderedTemplates().every((a) => installState()[a.id] !== undefined));
    // The card whose panel is open stays, so choosing a filter never closes what you are filling in.
    const shownTemplates = createMemo(() => {
        const selected = new Set(cardFilters());
        if (selected.size === 0) return orderedTemplates();
        const now = new Date();
        return orderedTemplates().filter(
            (a) => a.id === openTemplate() || matchesFilters({ provider: a.provider, installed: installState()[a.id] }, selected, now)
        );
    });

    // Refresh install state whenever the agent list changes.
    // Session-zone probes were removed in the Phase 1 cleanup
    // (reagent P2 on #1011) — templates carry `hasCurrentSession={false}`
    // by invariant and my-agent rows source from `MyAgentsList` which
    // doesn't need per-row session probes.
    //
    // Tracks `agents()` only. It used to track `installState()` as well,
    // and an agent's entry is only written once its check returns, so
    // every returning check re-ran this and re-issued the check for every
    // agent still pending: ~n²/2 calls, ~700 with ~37 agents
    // (docs/analysis/ANALYSIS_NEW_WINDOW_TAB_LATENCY_2026_09_30.md).
    const installCheckRequested = new Set<string>();
    createEffect(() => {
        for (const agent of agents()) {
            if (installCheckRequested.has(agent.id)) continue;
            installCheckRequested.add(agent.id);
            if (!(agent.id in untrack(installState))) void checkInstalled(agent);
        }
    });
    // Another picker or the Toolchain pane forgot an answer (an install
    // finished, a refresh found a CLI): ask again for the cards it concerns,
    // so a card never keeps a stale ribbon. One shared check per provider.
    onCleanup(
        onInstallCheckInvalidated((providerId) => {
            for (const agent of untrack(agents)) {
                const known = providerOfAgent.get(agent.id);
                if (providerId == null || known === undefined || known === providerId) void checkInstalled(agent);
            }
        }),
    );

    // Per-pane zoom — read `term:zoom` from block meta and apply CSS
    // `zoom` on the root, matching AgentPresentationView (chat mode).
    // The universal framework (Ctrl+/-/0 in keymodel, Ctrl+Wheel in
    // app.tsx) writes `term:zoom` for any block whose `viewType ===
    // "agent"`; the picker just needs to read + apply.
    const block = props.model.blockAtom;
    const zoomFactor = createMemo(() => readZoom(block()?.meta));

    // Shared between both branches below (fallback and real-content) — see
    // pickerReady/showPickerOverlay's own comments above for why a single
    // combined overlay replaced each branch's previous independent partial
    // reflow. Not a separate component (no reactive scope of its own
    // needed) — just a render-time helper so the markup isn't duplicated.
    const pickerOverlay = () => <PaneLoadingCover phase={pickerReadiness.phase} />;

    return (
        <>
            <Show
                when={agents().length > 0}
                fallback={
                    <div class="agent-view" style={{ zoom: zoomFactor() }}>
                        <Show when={!definitionsLoading()}>
                            <div class="agent-picker-empty">
                                <div class="agent-picker-empty-icon">{"\u2726"}</div>
                                <div class="agent-picker-empty-title">No definitions configured</div>
                                <div class="agent-picker-empty-desc">
                                    Use the ⚙ Agent settings to add your first definition.
                                </div>
                            </div>
                        </Show>
                        {pickerOverlay()}
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
                            Bottom: seeded templates, click = open its
                            panel below the card's row. */}
                        <AgentPickerFilterBar
                            value={filterQuery}
                            onInput={setFilterQuery}
                            onClear={() => setFilterQuery("")}
                            sortBy={sortBy}
                            onSortChange={setSortBy}
                        />
                        <MyAgentsList
                            nameFilter={filterQuery}
                            sortBy={sortBy}
                            onReattach={handleReattach}
                            runtimeOf={(id) => runtimeById().get(id)}
                            onAttach={handleAttach}
                            openDefinitions={openDefinitions}
                            openLocations={openLocations}
                            resolveOpenDefinitions={resolveOpenDefinitions}
                            onFork={handleFork}
                            onSwitchToExisting={handleSwitchToExisting}
                            onFirstLoad={() => setMyAgentsLoaded(true)}
                            onViewHistory={handleViewHistory}
                        />
                        <div class="agent-picker-templates-header" data-testid="agent-templates-header">
                            <span>New Agent</span>
                        </div>
                        <p class="agent-picker-templates-hint" data-testid="agent-templates-hint">
                            Each card is a <strong>harness</strong> (the CLI that runs the agent, e.g. Claude Code,
                            Codex) — you'll pick which <strong>model</strong> it uses next.
                        </p>
                        {/* SPEC_AGENT_CLI_ISOLATION_NOTE_2026_10_18.md §4.1. The exceptions are
                            the catalog's providers with no npm package, so the note can't
                            drift from what AgentMux actually installs. */}
                        <p class="agent-picker-templates-hint" data-testid="agent-templates-managed-note">
                            AgentMux installs and runs its own copy of each agent CLI, kept apart from any you installed
                            yourself, and keeps each agent's login and settings in its own folders. The exceptions use
                            the copy on your PATH, so install them yourself first:{" "}
                            <For each={userInstalledProviders()}>
                                {(p, i) => (
                                    <>
                                        {i() > 0 ? ", " : ""}
                                        <a
                                            href={p.docsUrl}
                                            onClick={(e) => {
                                                e.preventDefault();
                                                getApi().openExternal(p.docsUrl);
                                            }}
                                        >
                                            {p.displayName}
                                        </a>
                                    </>
                                )}
                            </For>
                            . This isolates the tools, not what an agent can do on your computer.
                        </p>
                        <AgentCardFilters
                            cards={filterableCards()}
                            selected={cardFilters()}
                            onChange={setCardFilters}
                            shown={shownTemplates().length}
                            installedKnown={installedKnown()}
                            disabled={!!openTemplate()}
                        />
                        <div data-testid="agent-templates-list">
                            {/* Pattern P1 (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md
                                §5.7): a card's panel opens below its row. */}
                            <CardGrid
                                class="agent-picker-list"
                                ariaLabel="New agent"
                                items={shownTemplates()}
                                keyOf={(agent) => agent.id}
                                minCardWidth={240}
                                selectedKey={openTemplate()}
                                onClose={closePanel}
                                renderCard={(agent) => (
                                    <AgentCard
                                        agent={agent}
                                        launching={launching() === agent.id}
                                        disabled={busy()}
                                        installed={installState()[agent.id]}
                                        installing={
                                            installState()[agent.id] === false &&
                                            isAgentInstalling(providerOfAgent.get(agent.id) ?? "")
                                        }
                                        expanded={openTemplate() === agent.id}
                                        drift={driftState()[agent.id]}
                                        onLaunch={handleTemplateSelect}
                                        // Right-click → Hide template. Only
                                        // template cards; my-agent rows live in
                                        // MyAgentsList.
                                        onContextMenu={handleTemplateContextMenu}
                                        // Templates have no session zone: no
                                        // "+ New" pill.
                                        hasCurrentSession={false}
                                        defaultFocus={shownTemplates()[0]?.id === agent.id}
                                        blockId={props.model.blockId}
                                    />
                                )}
                                renderDetail={(agent) => (
                                    <AgentCreatePanel
                                        template={agent}
                                        draft={draftFor(agent)}
                                        installed={installState()[agent.id]}
                                        probePrereqs={(fresh) => prereqsFor(agent, fresh)}
                                        recheckInstalled={() => recheckInstalled(agent)}
                                        onCreated={(created) => launchCreated(agent, created)}
                                        onClose={closePanel}
                                        onBusyChange={setPanelBusy}
                                    />
                                )}
                            />
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

                        <Show when={launchError()}>
                            <div class="agent-nodejs-notice">
                                <div class="nodejs-notice-icon">
                                    <i class="fa-solid fa-circle-exclamation" />
                                </div>
                                <div class="nodejs-notice-content">
                                    <div class="nodejs-notice-title">Launch aborted</div>
                                    <div class="nodejs-notice-text">{launchError()}</div>
                                </div>
                                {/* SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md surface 6 — no existing action row here, so an inline icon. */}
                                <CopyErrorButton
                                    variant="icon"
                                    className="agent-nodejs-notice-copy"
                                    report={() =>
                                        formatErrorReport({ title: "Launch aborted", message: launchError() ?? "" })
                                    }
                                />
                            </div>
                        </Show>
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
                    {pickerOverlay()}
                </div>
            </Show>
        </>
    );
};

AgentPicker.displayName = "AgentPicker";
