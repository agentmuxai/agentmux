// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The parts of the agent picker (AgentPicker.tsx lays them out): the empty state, the My Agents tier, the New Agent
// templates tier, the launch notices, the template menu, and the loading cover shared by both branches.

import { PaneLoadingCover } from "@/app/element/PaneLoadingCover";
import { CardGrid } from "@/app/element/ui/CardGrid";
import { CopyErrorButton } from "@/app/errors/CopyErrorButton";
import { formatErrorReport } from "@/app/errors/error-report";
import { Menu } from "@/app/element/ui";
import { MENU_ICON } from "@/app/util/menu-icons";
import { Show, type JSX } from "solid-js";
import { isAgentInstalling } from "../install-sessions";
import { AgentCard } from "./AgentCard";
import { AgentCardFilters } from "./AgentCardFilters";
import { AgentCreatePanel } from "./AgentCreatePanel";
import { AgentPickerFilterBar } from "./AgentPickerFilterBar";
import { HiddenTemplatesSection } from "./HiddenTemplatesSection";
import { MyAgentsList } from "./MyAgentsList";
import type { AgentPickerProps, AgentPickerState } from "./agent-picker-state";

/** The note under the harness list (SPEC_AGENT_CLI_ISOLATION_NOTE_2026_10_18.md §4.1): AgentMux
 * installs every agent CLI itself, so it has no exceptions to list
 * (SPEC_ISOLATED_INSTALL_FOR_NON_NPM_CLIS_2026_11_05.md §4). */
const ManagedCliNote = (): JSX.Element => (
    <p class="agent-picker-templates-hint" data-testid="agent-templates-managed-note">
        AgentMux installs and runs its own copy of each agent CLI, kept apart from any you installed
        yourself (to run yours instead, switch on "Use the CLI on my PATH" for it in Toolchain), and
        keeps each agent's login and settings in its own folders. This isolates the tools, not what an
        agent can do on your computer.
    </p>
);

/** Shared between both branches (fallback and real content) — see agent-picker-readiness.ts for why a single
 *  combined overlay replaced each branch's previous independent partial reflow. */
export function PickerOverlay(part: { s: AgentPickerState }): JSX.Element {
    return <PaneLoadingCover phase={part.s.pickerReadiness.phase} />;
}

/** No agent definitions: the empty state once they have loaded, and the cover. */
export function EmptyPicker(part: { s: AgentPickerState }): JSX.Element {
    const { s } = part;
    const { zoomFactor, definitionsLoading } = s;
    return (
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
            <PickerOverlay s={s} />
        </div>
    );
}

/** The user's own agents: the filter bar and the list. */
function MyAgentsTier(part: { s: AgentPickerState; props: AgentPickerProps }): JSX.Element {
    const { s, props } = part;
    const { launch, filterQuery, setFilterQuery, sortBy, setSortBy, openDefinitions, openLocations, resolveOpenDefinitions, readiness } = s;
    return (
        <>
            {/* Two-tier picker — Phase 1 (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md).
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
                paneBlockId={props.model.blockId}
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
        </>
    );
}

/** The New Agent tier: the harness cards, with a card's panel opening below its row. */
function TemplatesTier(part: { s: AgentPickerState; props: AgentPickerProps }): JSX.Element {
    const { s, props } = part;
    const { templates, launch, install, prereqs } = s;
    return (
        <>
            <div class="agent-picker-templates-header" data-testid="agent-templates-header">
                <span>New Agent</span>
            </div>
            <p class="agent-picker-templates-hint" data-testid="agent-templates-hint">
                Each card is a <strong>harness</strong> (the CLI that runs the agent, e.g. Claude Code,
                Codex) — you'll pick which <strong>model</strong> it uses next.
            </p>
            <ManagedCliNote />
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
                (CLAUDE.md notes that the AgentMux logo menu's
                Settings entry just opens settings.json,
                so there's no separate settings panel to
                host this in). */}
        </>
    );
}

/** A launch that aborted, and the Node.js notice. */
function LaunchNotices(part: { s: AgentPickerState }): JSX.Element {
    const { launch } = part.s;
    return (
        <>
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
        </>
    );
}

/** Right-click on a template card: Hide template. */
function TemplateMenu(part: { s: AgentPickerState }): JSX.Element {
    const { templates } = part.s;
    return (
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
    );
}

/** The picker with agents: the two tiers, the hidden templates, the notices, the cover and the menu. */
export function PickerContent(part: { s: AgentPickerState; props: AgentPickerProps }): JSX.Element {
    const { s } = part;
    return (
        <div class="agent-view" style={{ zoom: s.zoomFactor() }}>
            <div class="agent-picker">
                <MyAgentsTier s={s} props={part.props} />
                <TemplatesTier s={s} props={part.props} />
                {/* Phase 2 (Q2 Decision Y): collapsible
                    "Hidden templates" section, lazy-loaded so
                    it doesn't add work when no templates are
                    hidden. Sits under the templates tier so
                    hide + unhide live in the same surface
                    (CLAUDE.md notes that the AgentMux logo menu's
                    Settings entry just opens settings.json,
                    so there's no separate settings panel to
                    host this in). */}
                <HiddenTemplatesSection />

                <LaunchNotices s={s} />
            </div>
            <PickerOverlay s={s} />
            <TemplateMenu s={s} />
        </div>
    );
}
