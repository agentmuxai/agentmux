// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The "New agent" half of the picker: which card's panel is open, what was typed per harness, the Free / Local /
// Installed chips and the order of the cards, and the right-click Hide menu. Split out of AgentPicker.tsx
// (#13044 S6).

import type { AgentDefinition } from "@/app/store/rpc-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createMemo, createSignal, type Accessor } from "solid-js";
import { matchesFilters, type CardFilter, type FilterableCard } from "../defaults/provider-access-model";
import { getProviderList } from "../providers";
import { createCreateDraft, type CreateDraft } from "./AgentCreateForm";
import type { InstallStates } from "./agent-picker-install";

export function createTemplates(d: {
    agents: Accessor<AgentDefinition[]>;
    install: Pick<InstallStates, "installState" | "checkInstalled">;
    clearNodejsError: () => void;
}) {
    const { agents } = d;
    const { installState, checkInstalled } = d.install;
    const setNodejsError = (_: null): void => d.clearNodejsError();
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

    /** The agent the panel's form created is launched (or stopped short): close its panel and forget what was typed. */
    const forgetTemplate = (templateId: string): void => {
        drafts.delete(templateId);
        if (openTemplate() === templateId) setOpenTemplate(null);
    };

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
    // The Hide menu is the kit Menu, opened at the pointer.
    const [templateMenu, setTemplateMenu] = createSignal<{ agent: AgentDefinition; x: number; y: number } | null>(null);
    const handleTemplateContextMenu = (agent: AgentDefinition, evt: MouseEvent) => {
        evt.preventDefault();
        // The pane's own right-click menu is not wanted under this one.
        evt.stopPropagation();
        setTemplateMenu({ agent, x: evt.clientX, y: evt.clientY });
    };
    const hideTemplate = (agent: AgentDefinition) => {
        void (async () => {
            try {
                await RpcApi.AgentDefHideCommand(TabRpcClient, { definition_id: agent.id });
            } catch (err) {
                // Rejected only for a row that is not a template, which this menu cannot reach; leave a trace.
                console.warn(`agentdefhide failed for ${agent.id}:`, err);
            }
        })();
    };

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
            (a) =>
                a.id === openTemplate() ||
                matchesFilters({ provider: a.provider, installed: installState()[a.id] }, selected, now)
        );
    });

    return {
        openTemplate,
        setPanelBusy,
        draftFor,
        handleTemplateSelect,
        closePanel,
        forgetTemplate,
        templateMenu,
        setTemplateMenu,
        handleTemplateContextMenu,
        hideTemplate,
        cardFilters,
        setCardFilters,
        filterableCards,
        installedKnown,
        shownTemplates,
        userInstalledProviders,
    };
}

export type Templates = ReturnType<typeof createTemplates>;
