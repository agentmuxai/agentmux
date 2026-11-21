// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The markup of Settings → Kanban's stage editor in parts (kanban-section.tsx lays them out): a row per stage, the
// preview strip, the problems list and the footer. Each reads the editor's state (kanban-section-state.ts).

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button, FilterChip, FilterChips, IconButton, MenuTrigger, Notice, Select, TextInput, type MenuEntry } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { t } from "@/app/i18n/i18n";
import { makeIconClass } from "@/util/util";
import { COLUMNS, type ColumnId } from "@/app/view/kanban/kanban-columns";
import { canRemove, canShow } from "@/app/view/kanban/kanban-stages-draft";
import { MAX_STAGES, STAGE_PALETTE, validateStages, type KanbanStage } from "@/app/view/kanban/kanban-stages";

import type { KanbanStagesState } from "./kanban-section-state";
import { createStageReorder, type StageReorder } from "./kanban-stages-reorder";

import "@/app/view/kanban/kanban-stage-colours.scss";
import "./kanban-section.scss";

type Part = { s: KanbanStagesState };

/** The palette as the menu's rows: each swatch is drawn in its own theme token, with its name written beside it. */
const colourItems = (s: KanbanStagesState, stage: KanbanStage): MenuEntry[] =>
    STAGE_PALETTE.map((p) => ({
        label: t(`settings.kanban.colour.${p.id}`),
        type: "radio",
        checked: p.id === stage.colour,
        swatchColor: `var(--kanban-stage-${p.token})`,
        onSelect: () => s.recolour(stage.id, p.id),
    }));

/** The stage's colour: a button showing its swatch and name, which opens the palette. Read-only, it is the button without the menu. */
function ColourPicker(props: Part & { stage: KanbanStage }): JSX.Element {
    const face = (disabled: boolean) => (
        <Button
            density="compact"
            class="kanban-stages__colour"
            disabled={disabled}
            aria-haspopup={disabled ? undefined : "menu"}
            aria-label={t("settings.kanban.colourIs", { name: t(`settings.kanban.colour.${props.stage.colour}`) })}
        >
            <span class="kanban-stages__dot" aria-hidden="true" />
            {t(`settings.kanban.colour.${props.stage.colour}`)}
            <i class={makeIconClass("chevron-down", true)} aria-hidden="true" />
        </Button>
    );
    return (
        <Show when={props.s.editable()} fallback={face(true)}>
            <MenuTrigger items={colourItems(props.s, props.stage)} placement="bottom-start" aria-label={t("settings.kanban.colour")}>
                {face(false)}
            </MenuTrigger>
        </Show>
    );
}

/** The sources a stage may show, as toggle chips: what it shows is what its stage holds. */
function SourceToggles(props: Part & { stage: KanbanStage }): JSX.Element {
    // The chips report the whole set they would leave; the one that differs is the one clicked.
    const onChange = (next: string[]) => {
        const now = props.stage.takes as string[];
        const id = next.find((x) => !now.includes(x)) ?? now.find((x) => !next.includes(x));
        if (id) props.s.toggle(props.stage.id, id as ColumnId, next.includes(id));
    };
    return (
        <FilterChips class="kanban-stages__shows" aria-label={t("settings.kanban.shows")} value={props.stage.takes} onChange={onChange}>
            <For each={COLUMNS.filter((c) => canShow(props.stage, c.id))}>
                {(c) => (
                    <FilterChip id={c.id} disabled={!props.s.editable()}>
                        {t(`settings.kanban.source.${c.id}`)}
                    </FilterChip>
                )}
            </For>
        </FilterChips>
    );
}

/** Taking a stage away while it holds sources asks where they should show: a panel in the row, not a modal. */
function RemovePanel(props: Part & { stage: KanbanStage; onClose: () => void }): JSX.Element {
    const [into, setInto] = createSignal(props.s.removeInto(props.stage.id) ?? "");
    const targets = () => props.s.stages().filter((x) => x.id !== props.stage.id && !x.takes.includes("needs"));
    return (
        <div class="kanban-stages__panel" role="group" aria-label={t("settings.kanban.removeTitle", { name: props.stage.name })}>
            <Show when={props.stage.takes.length > 0 && targets().length > 0}>
                <label class="kanban-stages__panel-label">
                    {t("settings.kanban.removeInto")}
                    <Select
                        density="compact"
                        ariaLabel={t("settings.kanban.removeInto")}
                        value={into()}
                        onChange={setInto}
                        options={targets().map((x) => ({ value: x.id, label: x.name }))}
                    />
                </label>
            </Show>
            <Button
                density="compact"
                tone="danger"
                onClick={() => {
                    props.s.remove(props.stage.id, into() || null);
                    props.onClose();
                }}
            >
                {t("settings.kanban.remove")}
            </Button>
            <Button density="compact" tone="quiet" onClick={props.onClose}>
                {t("settings.kanban.cancel")}
            </Button>
        </div>
    );
}

function StageRow(props: Part & { stage: KanbanStage; index: number; last: boolean; reorder: StageReorder }): JSX.Element {
    const [removing, setRemoving] = createSignal(false);
    const s = props.s;
    const stage = () => props.stage;
    const here = () => validateStages(s.stages()).filter((p) => p.stage === stage().id);
    const needs = () => stage().takes.includes("needs");
    return (
        <li
            class="kanban-stages__row"
            data-stage-colour={stage().colour}
            data-dragging={props.reorder.dragging ? "" : undefined}
            data-drop={props.reorder.drop}
            onDragOver={props.reorder.onDragOver}
            onDrop={props.reorder.onDrop}
        >
            <div class="kanban-stages__line">
                <span
                    class="kanban-stages__grip"
                    draggable={s.editable()}
                    aria-hidden="true"
                    {...tip(t("settings.kanban.dragStage"))}
                    onDragStart={props.reorder.onDragStart}
                    onDragEnd={props.reorder.onDragEnd}
                >
                    <i class={makeIconClass("grip-vertical", true)} />
                </span>
                <span class="kanban-stages__dot" aria-hidden="true" />
                <TextInput
                    density="compact"
                    class="kanban-stages__name"
                    aria-label={t("settings.kanban.stageName")}
                    value={stage().name}
                    invalid={here().length > 0}
                    disabled={!s.editable()}
                    onInput={(e) => s.rename(stage().id, e.currentTarget.value)}
                />
                <ColourPicker s={s} stage={stage()} />
                <span class="kanban-stages__count" {...tip(t("settings.kanban.tasksHere"))}>
                    {s.counts()[stage().id] ?? 0}
                </span>
                <IconButton
                    icon="arrow-up"
                    label={t("settings.kanban.moveUp", { name: stage().name })}
                    disabled={!s.editable() || props.index === 0}
                    onClick={() => s.move(stage().id, -1)}
                />
                <IconButton
                    icon="arrow-down"
                    label={t("settings.kanban.moveDown", { name: stage().name })}
                    disabled={!s.editable() || props.last}
                    onClick={() => s.move(stage().id, 1)}
                />
                <Show
                    when={!needs()}
                    fallback={<IconButton icon="lock" label={t("settings.kanban.cantRemove")} disabled />}
                >
                    <IconButton
                        icon="trash-can"
                        label={t("settings.kanban.removeStage", { name: stage().name })}
                        disabled={!s.editable() || !canRemove(s.stages(), stage().id)}
                        onClick={() => setRemoving(true)}
                    />
                </Show>
            </div>
            <Show when={!needs()} fallback={<p class="kanban-stages__note">{t("settings.kanban.needsNote")}</p>}>
                <SourceToggles s={s} stage={stage()} />
            </Show>
            <Show when={here().length > 0}>
                <p class="kanban-stages__why" role="note">
                    {here().map((p) => p.message).join(". ")}
                </p>
            </Show>
            <Show when={removing()}>
                <RemovePanel s={s} stage={stage()} onClose={() => setRemoving(false)} />
            </Show>
        </li>
    );
}

export function StageList(props: Part): JSX.Element {
    const s = props.s;
    const reorder = createStageReorder(s.moveNear);
    return (
        <ol class="kanban-stages__list" aria-label={t("settings.kanban.stages")}>
            <For each={s.stages().map((x) => x.id)}>
                {(id) => (
                    <Show when={s.stages().find((x) => x.id === id)}>
                        {(stage) => (
                            <StageRow
                                s={s}
                                stage={stage()}
                                index={s.stages().findIndex((x) => x.id === id)}
                                last={s.stages()[s.stages().length - 1]?.id === id}
                                reorder={reorder(id)}
                            />
                        )}
                    </Show>
                )}
            </For>
        </ol>
    );
}

/** The stage headers as the board will draw them, with today's counts: the way to see a merge before applying it. */
export function Preview(props: Part): JSX.Element {
    const s = props.s;
    return (
        <div class="kanban-stages__preview" aria-label={t("settings.kanban.preview")}>
            <For each={s.stages()}>
                {(st) => (
                    <span class="kanban-stages__chip" data-stage-colour={st.colour}>
                        <span class="kanban-stages__dot" aria-hidden="true" />
                        {st.name}
                        <span class="kanban-stages__chip-count">{s.counts()[st.id] ?? 0}</span>
                    </span>
                )}
            </For>
        </div>
    );
}

/** Everything wrong with the draft, once, as one live message. */
export function Problems(props: Part): JSX.Element {
    return (
        <Show when={props.s.dirty() && props.s.problems().length > 0}>
            <p class="kanban-stages__problems" role="status" aria-live="polite">
                {[...new Set(props.s.problems().map((p) => p.message))].join(". ")}
            </p>
        </Show>
    );
}

export function Notices(props: Part): JSX.Element {
    const s = props.s;
    return (
        <>
            <Show when={s.refused() ?? s.model.error()}>
                {(why) => (
                    <Notice tone="danger" onDismiss={s.dismissRefused}>
                        {why().charAt(0).toUpperCase() + why().slice(1)}
                    </Notice>
                )}
            </Show>
            <Show when={!s.supported()}>
                <Notice tone="warning">{t("settings.kanban.unsupported")}</Notice>
            </Show>
            <Show when={s.archived()}>
                <Notice>{t("settings.kanban.archived")}</Notice>
            </Show>
            <Show when={s.info()?.stages_error}>
                {(why) => (
                    <Notice tone="warning" actions={<Button density="compact" onClick={s.reset}>{t("settings.kanban.reset")}</Button>}>
                        {t("settings.kanban.brokenStored", { why: why() })}
                    </Notice>
                )}
            </Show>
            <Show when={s.changedMeanwhile()}>
                <Notice
                    tone="warning"
                    actions={
                        <>
                            <Button density="compact" onClick={s.discard}>{t("settings.kanban.loadTheirs")}</Button>
                            <Button density="compact" onClick={() => void s.applyMine()}>{t("settings.kanban.applyMine")}</Button>
                        </>
                    }
                >
                    {t("settings.kanban.changedMeanwhile")}
                </Notice>
            </Show>
        </>
    );
}

/** "Use for all my boards": asks first, in the editor, and says what it did. */
function CopyToAll(props: Part): JSX.Element {
    const s = props.s;
    const [asking, setAsking] = createSignal(false);
    return (
        <Show when={s.otherLiveBoards() > 0}>
            <Show
                when={asking()}
                fallback={
                    <Button density="compact" disabled={!s.editable() || s.problems().length > 0} onClick={() => setAsking(true)}>
                        {t("settings.kanban.useForAll")}
                    </Button>
                }
            >
                <span class="kanban-stages__ask" role="group" aria-label={t("settings.kanban.useForAll")}>
                    {t("settings.kanban.useForAllAsk", { count: s.otherLiveBoards() })}
                    <Button
                        density="compact"
                        tone="accent"
                        onClick={() => {
                            setAsking(false);
                            void s.copyToAll();
                        }}
                    >
                        {t("settings.kanban.copy")}
                    </Button>
                    <Button density="compact" tone="quiet" onClick={() => setAsking(false)}>
                        {t("settings.kanban.cancel")}
                    </Button>
                </span>
            </Show>
        </Show>
    );
}

/** The board's latest changes to its stages, newest first: who changed them, how, and when. */
export function History(props: Part): JSX.Element {
    const lines = () => props.s.info()?.history ?? [];
    return (
        <Show when={lines().length > 0}>
            <section class="kanban-stages__history" aria-label={t("settings.kanban.history")}>
                <h4 class="kanban-stages__history-title">{t("settings.kanban.history")}</h4>
                <ul class="kanban-stages__history-list">
                    <For each={lines()}>
                        {(ev) => (
                            <li>
                                {t("settings.kanban.historyLine", {
                                    who: ev.actor_kind === "agent" ? ev.actor : t("settings.kanban.you"),
                                    what: ev.body,
                                    when: new Date(ev.at).toLocaleString(),
                                })}
                            </li>
                        )}
                    </For>
                </ul>
            </section>
        </Show>
    );
}

export function Footer(props: Part): JSX.Element {
    const s = props.s;
    return (
        <div class="kanban-stages__footer">
            <CopyToAll s={s} />
            <Show when={s.copy()}>
                {(r) => (
                    <span class="kanban-stages__done" role="status">
                        {t("settings.kanban.copied", { count: r().copied })}
                        <Show when={r().skipped.length > 0}> {t("settings.kanban.skipped", { names: r().skipped.join(", ") })}</Show>
                    </span>
                )}
            </Show>
            <span class="kanban-stages__spacer" />
            <Button density="compact" disabled={!s.dirty()} onClick={s.discard}>
                {t("settings.kanban.discard")}
            </Button>
            <Button
                density="compact"
                tone="accent"
                busy={s.busy()}
                disabled={!s.editable() || !s.dirty() || s.problems().length > 0}
                onClick={() => void s.apply()}
            >
                {t("settings.kanban.apply")}
            </Button>
        </div>
    );
}

export function StageActions(props: Part): JSX.Element {
    const s = props.s;
    return (
        <div class="kanban-stages__actions">
            <span class="kanban-stages__total">{t("settings.kanban.total", { count: s.stages().length })}</span>
            <span class="kanban-stages__spacer" />
            <Button density="compact" tone="quiet" disabled={!s.editable()} onClick={s.reset}>
                {t("settings.kanban.reset")}
            </Button>
            <span class="ui-tooltip-anchor" {...tip(s.canAdd() ? undefined : t("settings.kanban.atMost", { max: MAX_STAGES }))}>
                <Button
                    density="compact"
                    icon="plus"
                    disabled={!s.editable() || !s.canAdd()}
                    onClick={() => s.add(t("settings.kanban.newStageName", { n: s.stages().length + 1 }))}
                >
                    {t("settings.kanban.addStage")}
                </Button>
            </span>
        </div>
    );
}
