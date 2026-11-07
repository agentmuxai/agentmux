// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The panel under a My agents tile: its actions, and the views a click on one of them leads to (rename, fork
// name, Seat, Bundles, Environment, delete). Split out of MyAgentsList.tsx (#13044 S6); the state is
// my-agents-list-panels.ts.

import { Button } from "@/app/element/ui";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { batch, For, Match, Show, Switch, type JSX } from "solid-js";
import { AgentBundlesTab } from "./AgentBundlesTab";
import { AgentEnvPanel } from "./AgentEnvPanel";
import { AgentRowPanel } from "./AgentRowPanel";
import { AgentSeatPanel } from "./AgentSeatPanel";
import type { MyAgentsListProps } from "./MyAgentsList";
import { NamePrompt } from "./NamePrompt";
import type { RowPanels, SubView } from "./my-agents-list-panels";

type PanelView = "actions" | "rename" | "fork-name" | SubView;
const PANEL_SUBTITLE: Record<PanelView, string | undefined> = {
    actions: undefined,
    rename: "Rename",
    "fork-name": "New session",
    seat: "Seat",
    bundles: "Bundles",
    env: "Environment",
    delete: "Delete",
};
// The choices are the answer to a click on the tile, so they have nothing to go back to.
const PANEL_HAS_BACK: ReadonlySet<PanelView> = new Set(["rename", "fork-name", "seat", "bundles", "env", "delete"]);

interface PanelProps {
    row: RecentSessionRow;
    panels: RowPanels;
    openLocations: MyAgentsListProps["openLocations"];
}

type NamingState = Extract<ReturnType<RowPanels["getForkState"]>, { kind: "naming" }>;

/** What the panel's views share: the agent, which view is up and what the panel says about the agent. */
interface PanelState {
    props: PanelProps;
    row: RecentSessionRow;
    id: string;
    name: string;
    view: () => PanelView;
    isActive: () => boolean;
    withoutPane: () => boolean;
    openWhere: () => string;
    /** What the panel says about the agent before its choices. */
    note: () => string | undefined;
    naming: () => NamingState | null;
    toActions: () => void;
}

function createPanelState(props: PanelProps): PanelState {
    const { subView, getRenameState, getForkState, runsWithoutPane, openMap, closePanels, setOpenMenuId } =
        props.panels;
    const row = props.row;
    const id = row.definition_id;
    const isActive = () => openMap().has(id);
    const withoutPane = () => runsWithoutPane(id);
    const openWhere = () => props.openLocations?.().get(id)?.label ?? "open in another pane";
    return {
        props,
        row,
        id,
        name: row.instance_name || row.definition_name,
        view: (): PanelView => {
            const sub = subView();
            if (sub && sub.id === id) return sub.kind;
            if (getRenameState(id)) return "rename";
            if (getForkState(id).kind === "naming") return "fork-name";
            return "actions";
        },
        isActive,
        withoutPane,
        openWhere,
        note: (): string | undefined =>
            isActive() ? `is already ${openWhere()}.` : withoutPane() ? "is running with no pane." : undefined,
        naming: () => {
            const fs = getForkState(id);
            return fs.kind === "naming" ? fs : null;
        },
        toActions: (): void =>
            batch(() => {
                closePanels();
                setOpenMenuId(id);
            }),
    };
}

export const MyAgentsRowPanel = (props: PanelProps): JSX.Element => {
    const s = createPanelState(props);
    const { id, name, view, toActions } = s;
    const { deleteBusy, closePanels, tileEls } = props.panels;
    return (
        <AgentRowPanel
            title={name}
            subtitle={PANEL_SUBTITLE[view()]}
            viewKey={view()}
            tile={() => tileEls.get(id)}
            onClose={() => {
                if (!deleteBusy()) closePanels();
            }}
            onBack={PANEL_HAS_BACK.has(view()) && !deleteBusy() ? toActions : undefined}
        >
            <PanelBody s={s} />
        </AgentRowPanel>
    );
};

function PanelBody({ s }: { s: PanelState }) {
    const { id, name, view, toActions } = s;
    const { seatFormOf, envDraftOf } = s.props.panels;
    return (
        <Switch>
            <Match when={view() === "actions"}>
                <ActionsView s={s} />
            </Match>
            <Match when={view() === "rename"}>
                <RenameView s={s} />
            </Match>
            <Match when={view() === "fork-name"}>
                <ForkNameView s={s} />
            </Match>
            <Match when={view() === "seat"}>
                <div class="agent-row-panel-embed">
                    <AgentSeatPanel agentId={id} agentName={name} form={seatFormOf(id)} onClose={toActions} />
                </div>
            </Match>
            <Match when={view() === "bundles"}>
                <div class="agent-row-panel-embed">
                    <AgentBundlesTab agentId={id} onClose={toActions} />
                </div>
            </Match>
            <Match when={view() === "env"}>
                <div class="agent-row-panel-embed">
                    <AgentEnvPanel agentId={id} agentName={name} draft={envDraftOf(id)} onClose={toActions} />
                </div>
            </Match>
            <Match when={view() === "delete"}>
                <DeleteView s={s} />
            </Match>
        </Switch>
    );
}

function ActionsView({ s }: { s: PanelState }) {
    const { name, row, note, isActive, withoutPane } = s;
    const { rowActions } = s.props.panels;
    return (
        <div class="agent-row-panel-actions-view" data-testid="agent-row-actions">
            <Show when={note()}>
                {(text) => (
                    <p class="agent-row-panel-note">
                        <strong>{name}</strong> {text()}
                    </p>
                )}
            </Show>
            <div class="agent-row-panel-actions" data-testid="agent-row-open">
                <OpenButtons s={s} />
            </div>
            <div class="agent-row-panel-actions" data-testid="agent-row-more">
                <For each={rowActions(row, !isActive() && !withoutPane())}>
                    {(action) => (
                        <Button
                            icon={action.icon}
                            tone={action.tone}
                            onClick={action.onSelect}
                            data-testid={action.testId}
                        >
                            {action.label}
                        </Button>
                    )}
                </For>
            </div>
        </div>
    );
}

function OpenButtons({ s }: { s: PanelState }) {
    const { handleOpen, handleAttachNoPane, handleOpenNewSession, handleSwitchToExisting } = s.props.panels;
    const { row, isActive, withoutPane } = s;
    return (
        <Switch>
            <Match when={isActive()}>
                <Button
                    tone="accent"
                    icon="right-left"
                    data-testid="agent-fork-switch"
                    onClick={() => void handleSwitchToExisting(row)}
                >
                    Switch to existing
                </Button>
                <Button
                    icon="code-fork"
                    data-testid="agent-fork-open-new"
                    onClick={() => void handleOpenNewSession(row)}
                >
                    Fork…
                </Button>
            </Match>
            <Match when={withoutPane()}>
                <Button
                    tone="accent"
                    icon="arrow-right-to-bracket"
                    data-testid="agent-fork-attach"
                    onClick={() => handleAttachNoPane(row)}
                >
                    Open it
                </Button>
                <Button icon="code-fork" data-testid="agent-fork-fork" onClick={() => void handleOpenNewSession(row)}>
                    Fork…
                </Button>
            </Match>
            <Match when={true}>
                <Button
                    tone="accent"
                    icon="arrow-right-to-bracket"
                    data-testid="agent-my-agents-open"
                    onClick={() => void handleOpen(row)}
                >
                    Open
                </Button>
            </Match>
        </Switch>
    );
}

function RenameView({ s }: { s: PanelState }) {
    const { id, row } = s;
    const { getRenameState, setRenameState, handleRenameSubmit, handleRenameCancel } = s.props.panels;
    return (
        <Show when={getRenameState(id)}>
            {(rs) => (
                <div data-testid="agent-rename-prompt">
                    <NamePrompt
                        label="Rename to:"
                        placeholder="Agent name"
                        value={rs().label}
                        loading={rs().loading}
                        error={rs().error}
                        submitLabel="Save"
                        inputTestid="agent-rename-input"
                        submitTestid="agent-rename-save"
                        onInput={(label) => setRenameState(id, { ...rs(), label })}
                        onSubmit={() => void handleRenameSubmit(row)}
                        onCancel={() => handleRenameCancel(id)}
                    />
                </div>
            )}
        </Show>
    );
}

function ForkNameView({ s }: { s: PanelState }) {
    const { id, name, row, naming, isActive, openWhere } = s;
    const { setForkState, handleForkStart, handleForkCancel, setOpenMenuId } = s.props.panels;
    return (
        <Show when={naming()}>
            {(ns) => (
                <div data-testid="agent-fork-prompt">
                    <Show when={isActive()}>
                        <p class="agent-row-panel-note">
                            <strong>{name}</strong> is already {openWhere()}.
                        </p>
                    </Show>
                    <NamePrompt
                        label="Name for new session:"
                        placeholder="Session name"
                        value={ns().label}
                        loading={ns().loading}
                        error={ns().error}
                        submitLabel="Start"
                        inputTestid="agent-fork-name-input"
                        submitTestid="agent-fork-start"
                        onInput={(label) =>
                            setForkState(id, {
                                kind: "naming",
                                label,
                                loading: false,
                                error: null,
                                edited: true,
                            })
                        }
                        onSubmit={() => void handleForkStart(row)}
                        onCancel={() =>
                            batch(() => {
                                handleForkCancel(id);
                                setOpenMenuId(id);
                            })
                        }
                    />
                </div>
            )}
        </Show>
    );
}

function DeleteView({ s }: { s: PanelState }) {
    const { name, row, toActions } = s;
    const { deleteBusy, confirmDelete } = s.props.panels;
    return (
        <div data-testid="agent-delete-confirm">
            <p class="agent-row-panel-note">
                This permanently deletes the agent and its credentials, skills, and activity history. Its bundle and any
                manually-saved transcripts are not affected. This cannot be undone.
            </p>
            <div class="agent-row-panel-actions">
                <Button
                    tone="danger"
                    icon="trash-can"
                    data-testid="agent-delete-confirm-yes"
                    busy={deleteBusy()}
                    onClick={() => void confirmDelete(row)}
                >
                    Delete {name}
                </Button>
                <Button disabled={deleteBusy()} onClick={toActions}>
                    Cancel
                </Button>
            </div>
        </div>
    );
}
