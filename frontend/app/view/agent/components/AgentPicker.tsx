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
import { CardGrid } from "@/app/element/ui/CardGrid";
import { CopyErrorButton } from "@/app/errors/CopyErrorButton";
import { formatErrorReport } from "@/app/errors/error-report";
import { Menu } from "@/app/element/ui";
import { getApi } from "@/app/store/global";
import { readZoom } from "@/app/store/zoom-factor";
import { MENU_ICON } from "@/app/util/menu-icons";
import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import type { AgentViewModel } from "../agent-model";
import { isAgentInstalling } from "../install-sessions";
import { AgentCard } from "./AgentCard";
import { AgentCardFilters } from "./AgentCardFilters";
import { AgentCreatePanel } from "./AgentCreatePanel";
import { AgentPickerFilterBar, type AgentSortOption } from "./AgentPickerFilterBar";
import { useAgentDefinitions, useInstanceOpenDefinitions, useOpenDefinitionMap } from "./agent-definition-hooks";
import { useInstallStates } from "./agent-picker-install";
import { createPickerLaunch } from "./agent-picker-launch";
import { createPrereqs } from "./agent-picker-prereqs";
import { usePickerReadiness } from "./agent-picker-readiness";
import { loadStoredSort, storeSort } from "./agent-picker-sort";
import { createTemplates } from "./agent-picker-templates";
import { HiddenTemplatesSection } from "./HiddenTemplatesSection";
import { MyAgentsList } from "./MyAgentsList";

export { useAgentDefinitions, useOpenDefinitionMap };

// ── AgentPicker component ───────────────────────────────────────────────────────

interface AgentPickerProps {
    model: AgentViewModel;
}

export const AgentPicker = (props: AgentPickerProps): JSX.Element => {
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

    // The loading cover, the install checks, the system-tool probe, what a click launches and the New agent half
    // (each is a module of its own: agent-picker-*.ts).
    const readiness = usePickerReadiness(props.model.blockId, agents, definitionsLoading);
    const { pickerReadiness } = readiness;

    // Reactive map of definition_id → blockId for panes currently open.
    const [openDefinitions, refreshOpenDefinitions, openLocations, resolveOpenDefinitions] =
        useInstanceOpenDefinitions();
    const install = useInstallStates(agents);
    const prereqs = createPrereqs();
    const templates = createTemplates({
        agents,
        install,
        clearNodejsError: () => launch.setNodejsError(null),
    });
    const launch = createPickerLaunch(props, { agents, refreshOpenDefinitions, forgetTemplate: templates.forgetTemplate });

    // Per-pane zoom — read `term:zoom` from block meta and apply CSS
    // `zoom` on the root, matching AgentPresentationView (chat mode).
    // The universal framework (Ctrl+/-/0 in keymodel, Ctrl+Wheel in
    // app.tsx) writes `term:zoom` for any block whose `viewType ===
    // "agent"`; the picker just needs to read + apply.
    const block = props.model.blockAtom;
    const zoomFactor = createMemo(() => readZoom(block()?.meta));

    // Shared between both branches below (fallback and real-content) — see
    // agent-picker-readiness.ts for why a single
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
                            onReattach={launch.handleReattach}
                            runtimeOf={(id) => launch.runtimeById().get(id)}
                            onAttach={launch.handleAttach}
                            openDefinitions={openDefinitions}
                            openLocations={openLocations}
                            resolveOpenDefinitions={resolveOpenDefinitions}
                            onFork={launch.handleFork}
                            onSwitchToExisting={launch.handleSwitchToExisting}
                            onFirstLoad={() => readiness.setMyAgentsLoaded(true)}
                            onViewHistory={launch.handleViewHistory}
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
                            <For each={templates.userInstalledProviders()}>
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
                            cards={templates.filterableCards()}
                            selected={templates.cardFilters()}
                            onChange={templates.setCardFilters}
                            shown={templates.shownTemplates().length}
                            installedKnown={templates.installedKnown()}
                            disabled={!!templates.openTemplate()}
                        />
                        <div data-testid="agent-templates-list">
                            {/* Pattern P1 (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md
                                §5.7): a card's panel opens below its row. */}
                            <CardGrid
                                class="agent-picker-list"
                                ariaLabel="New agent"
                                items={templates.shownTemplates()}
                                keyOf={(agent) => agent.id}
                                minCardWidth={240}
                                selectedKey={templates.openTemplate()}
                                onClose={templates.closePanel}
                                renderCard={(agent) => (
                                    <AgentCard
                                        agent={agent}
                                        launching={launch.launching() === agent.id}
                                        disabled={launch.busy()}
                                        installed={install.installState()[agent.id]}
                                        installing={
                                            install.installState()[agent.id] === false &&
                                            isAgentInstalling(install.providerOfAgent.get(agent.id) ?? "")
                                        }
                                        expanded={templates.openTemplate() === agent.id}
                                        drift={install.driftState()[agent.id]}
                                        onLaunch={templates.handleTemplateSelect}
                                        // Right-click → Hide template. Only
                                        // template cards; my-agent rows live in
                                        // MyAgentsList.
                                        onContextMenu={templates.handleTemplateContextMenu}
                                        // Templates have no session zone: no
                                        // "+ New" pill.
                                        hasCurrentSession={false}
                                        defaultFocus={templates.shownTemplates()[0]?.id === agent.id}
                                        blockId={props.model.blockId}
                                    />
                                )}
                                renderDetail={(agent) => (
                                    <AgentCreatePanel
                                        template={agent}
                                        draft={templates.draftFor(agent)}
                                        installed={install.installState()[agent.id]}
                                        probePrereqs={(fresh) => prereqs.prereqsFor(agent, fresh)}
                                        recheckInstalled={() => install.recheckInstalled(agent)}
                                        onCreated={(created) => launch.launchCreated(agent, created)}
                                        onClose={templates.closePanel}
                                        onBusyChange={templates.setPanelBusy}
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

                        <Show when={launch.launchError()}>
                            <div class="agent-nodejs-notice">
                                <div class="nodejs-notice-icon">
                                    <i class="fa-solid fa-circle-exclamation" />
                                </div>
                                <div class="nodejs-notice-content">
                                    <div class="nodejs-notice-title">Launch aborted</div>
                                    <div class="nodejs-notice-text">{launch.launchError()}</div>
                                </div>
                                {/* SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md surface 6 — no existing action row here, so an inline icon. */}
                                <CopyErrorButton
                                    variant="icon"
                                    className="agent-nodejs-notice-copy"
                                    report={() =>
                                        formatErrorReport({ title: "Launch aborted", message: launch.launchError() ?? "" })
                                    }
                                />
                            </div>
                        </Show>
                        <Show when={launch.nodejsError()}>
                            <div class="agent-nodejs-notice">
                                <div class="nodejs-notice-icon">
                                    <i class="fa-solid fa-circle-exclamation" />
                                </div>
                                <div class="nodejs-notice-content">
                                    <div class="nodejs-notice-title">Node.js Required</div>
                                    <div class="nodejs-notice-text">{launch.nodejsError()}</div>
                                    <div class="nodejs-notice-hint">
                                        After installing, restart AgentMux and try again.
                                    </div>
                                </div>
                            </div>
                        </Show>
                    </div>
                    {pickerOverlay()}
                    <Show when={templates.templateMenu()} keyed>
                        {(m) => (
                            <Menu
                                anchor={{ x: m.x, y: m.y }}
                                aria-label="Template actions"
                                items={[
                                    {
                                        label: `Hide template "${m.agent.name || m.agent.slug || m.agent.id}"`,
                                        icon: MENU_ICON.hide,
                                        onSelect: () => templates.hideTemplate(m.agent),
                                    },
                                ]}
                                onClose={() => templates.setTemplateMenu(null)}
                            />
                        )}
                    </Show>
                </div>
            </Show>
        </>
    );
};

AgentPicker.displayName = "AgentPicker";
