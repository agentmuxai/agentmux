// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The panel under a My agents tile: its actions, and the views a click on one of them leads to (rename, fork
// name, Seat, Bundles, Environment, delete). Split out of MyAgentsList.tsx (#13044 S6); the state is
// my-agents-list-panels.ts.

import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";
import { batch, For, Match, Show, Switch, type JSX } from "solid-js";
import { AgentBundlesTab } from "./AgentBundlesTab";
import { AgentEnvPanel } from "./AgentEnvPanel";
import { AgentRowPanel } from "./AgentRowPanel";
import { AgentSeatPanel } from "./AgentSeatPanel";
import { AgentSpendPanel } from "./AgentSpendPanel";
import type { MyAgentsListProps } from "./MyAgentsList";
import { NamePrompt } from "./NamePrompt";
import type { RowPanels, SubView } from "./my-agents-list-panels";

type PanelView = "actions" | "rename" | "fork-name" | SubView;
/** The panel's subtitle for a view; read where it is drawn, so it follows the language. */
function panelSubtitle(view: PanelView): string | undefined {
    switch (view) {
        case "rename":
            return t("myAgentsRow.subtitleRename");
        case "fork-name":
            return t("myAgentsRow.subtitleNewSession");
        case "seat":
            return t("myAgentsRow.subtitleSeat");
        case "bundles":
            return t("myAgentsRow.subtitleBundles");
        case "env":
            return t("myAgentsRow.subtitleEnv");
        case "spend":
            return t("agentSpend.subtitle");
        case "delete":
            return t("myAgentsRow.subtitleDelete");
        default:
            return undefined;
    }
}
// The choices are the answer to a click on the tile, so they have nothing to go back to.
const PANEL_HAS_BACK: ReadonlySet<PanelView> = new Set(["rename", "fork-name", "seat", "bundles", "env", "spend", "delete"]);

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
    /** What the panel says about the agent before its choices: it is open in a pane, or runs with none. */
    note: () => "active" | "nopane" | undefined;
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
    const openWhere = () => props.openLocations?.().get(id)?.label ?? t("myAgentsRow.openElsewhere");
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
        note: (): "active" | "nopane" | undefined => (isActive() ? "active" : withoutPane() ? "nopane" : undefined),
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
            subtitle={panelSubtitle(view())}
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
            <Match when={view() === "spend"}>
                <div class="agent-row-panel-embed">
                    <AgentSpendPanel agentId={id} agentName={name} onClose={toActions} />
                </div>
            </Match>
            <Match when={view() === "delete"}>
                <DeleteView s={s} />
            </Match>
        </Switch>
    );
}

function ActionsView({ s }: { s: PanelState }) {
    const { name, row, note, isActive, withoutPane, openWhere } = s;
    const { rowActions } = s.props.panels;
    return (
        <div class="agent-row-panel-actions-view" data-testid="agent-row-actions">
            <Show when={note()}>
                {(kind) => (
                    <p class="agent-row-panel-note">
                        <Rich k={kind() === "active" ? "myAgentsRow.noteActive" : "myAgentsRow.noteNoPane"} params={{ name, where: openWhere() }} />
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
    const { handleOpen, handleAttachNoPane, handleOpenNewSession, handleSwitchToExisting, handleOpenHere, canOpenHere } =
        s.props.panels;
    const { row, isActive, withoutPane } = s;
    return (
        <Switch>
            <Match when={isActive()}>
                <Button
                    tone="accent"
                    icon="right-left"
                    data-testid="agent-fork-switch"
                    onClick={() => handleSwitchToExisting(row)}
                >
                    {t("myAgentsRow.switchExisting")}
                </Button>
                <Show when={canOpenHere(row.definition_id)}>
                    <Button
                        icon="arrow-right-to-bracket"
                        title={t("agentPicker.openHereTip", { name: s.name })}
                        data-testid="agent-open-here"
                        onClick={() => handleOpenHere(row)}
                    >
                        {t("agentPicker.openHere")}
                    </Button>
                </Show>
                <Button
                    icon="code-fork"
                    data-testid="agent-fork-open-new"
                    onClick={() => handleOpenNewSession(row)}
                >
                    {t("myAgentsRow.fork")}
                </Button>
            </Match>
            <Match when={withoutPane()}>
                <Button
                    tone="accent"
                    icon="arrow-right-to-bracket"
                    data-testid="agent-fork-attach"
                    onClick={() => handleAttachNoPane(row)}
                >
                    {t("myAgentsRow.openIt")}
                </Button>
                <Button icon="code-fork" data-testid="agent-fork-fork" onClick={() => handleOpenNewSession(row)}>
                    {t("myAgentsRow.fork")}
                </Button>
            </Match>
            <Match when={true}>
                <Button
                    tone="accent"
                    icon="arrow-right-to-bracket"
                    data-testid="agent-my-agents-open"
                    onClick={() => handleOpen(row)}
                >
                    {t("myAgentsRow.open")}
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
                        label={t("myAgentsRow.renameTo")}
                        placeholder={t("myAgentsRow.agentName")}
                        value={rs().label}
                        loading={rs().loading}
                        error={rs().error}
                        submitLabel={t("myAgentsRow.save")}
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
                            <Rich k="myAgentsRow.noteActive" params={{ name, where: openWhere() }} />
                        </p>
                    </Show>
                    <NamePrompt
                        label={t("myAgentsRow.nameForSession")}
                        placeholder={t("myAgentsRow.sessionName")}
                        value={ns().label}
                        loading={ns().loading}
                        error={ns().error}
                        submitLabel={t("myAgentsRow.start")}
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
                {t("myAgentsRow.deleteWarning")}
            </p>
            <div class="agent-row-panel-actions">
                <Button
                    tone="danger"
                    icon="trash-can"
                    data-testid="agent-delete-confirm-yes"
                    busy={deleteBusy()}
                    onClick={() => confirmDelete(row)}
                >
                    {t("myAgentsRow.delete", { name })}
                </Button>
                <Button disabled={deleteBusy()} onClick={toActions}>
                    {t("myAgentsRow.cancel")}
                </Button>
            </div>
        </div>
    );
}
