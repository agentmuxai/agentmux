// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What a click in the agent picker starts: reattaching a My agents row, attaching to one that runs with no
// pane, forking one, opening a history tab, and launching the agent a template's form created. Split out of
// AgentPicker.tsx (#13044 S6).

import { noteOpenedByPerson } from "@/app/store/agent-open-note";
import { openAgentInActiveTab, sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { pushNotification } from "@/app/store/global";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { revealBlock } from "@/app/util/reveal-block";
import { refreshAccountCache } from "@/app/view/identity/identity-model";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { createMemo, createSignal, type Accessor } from "solid-js";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
import type { AgentViewModel } from "../agent-model";
import { realAccountIdOrEmpty } from "../identity-carry-over";
import { openOrFocusHistoryTab } from "../open-history-tab";
import { beginAgentOpen, finishAgentOpen } from "../open-trace";
import { getProvider } from "../providers";
import { copyForkHistory } from "../quick-fork";
import type { CreatedAgent } from "./AgentCreateForm";

export function createPickerLaunch(
    props: { model: AgentViewModel },
    d: {
        agents: Accessor<AgentDefinition[]>;
        refreshOpenDefinitions: () => void;
        forgetTemplate: (templateId: string) => void;
    }
) {
    const { agents, refreshOpenDefinitions } = d;
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
            d.forgetTemplate(template.id);
        }
    };

    const busy = () => launching() !== null;

    return {
        launching,
        nodejsError,
        setNodejsError,
        launchError,
        busy,
        runtimeById,
        handleReattach,
        handleAttach,
        handleFork,
        handleSwitchToExisting,
        handleViewHistory,
        launchCreated,
    };
}

export type PickerLaunch = ReturnType<typeof createPickerLaunch>;
