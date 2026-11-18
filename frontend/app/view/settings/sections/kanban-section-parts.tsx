// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The markup of Settings → Kanban's stage editor in parts (kanban-section.tsx lays them out): a row per stage, the
// preview strip, the problems list and the footer. Each reads the editor's state (kanban-section-state.ts).

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button, IconButton, Notice, Select, TextInput } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { t } from "@/app/i18n/i18n";
import { COLUMNS, type ColumnId } from "@/app/view/kanban/kanban-columns";
import { canRemove, canShow } from "@/app/view/kanban/kanban-stages-draft";
import { MAX_STAGES, STAGE_PALETTE, validateStages, type KanbanStage, type StageColour } from "@/app/view/kanban/kanban-stages";

import type { KanbanStagesState } from "./kanban-section-state";

import "@/app/view/kanban/kanban-stage-colours.scss";
import "./kanban-section.scss";

type Part = { s: KanbanStagesState };

const colourOptions = () => STAGE_PALETTE.map((p) => ({ value: p.id, label: t(`settings.kanban.colour.${p.id}`) }));

/** The sources a stage may show, as toggles: what it shows is what its stage holds. */
function SourceToggles(props: Part & { stage: KanbanStage }): JSX.Element {
    return (
        <div class="kanban-stages__shows" role="group" aria-label={t("settings.kanban.shows")}>
            <For each={COLUMNS.filter((c) => canShow(props.stage, c.id))}>
                {(c) => (
                    <Button
                        density="compact"
                        tone="quiet"
                        pressed={props.stage.takes.includes(c.id)}
                        disabled={!props.s.editable()}
                        onClick={() => props.s.toggle(props.stage.id, c.id as ColumnId, !props.stage.takes.includes(c.id))}
                    >
                        {t(`settings.kanban.source.${c.id}`)}
                    </Button>
                )}
            </For>
        </div>
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

function StageRow(props: Part & { stage: KanbanStage; index: number; last: boolean }): JSX.Element {
    const [removing, setRemoving] = createSignal(false);
    const s = props.s;
    const stage = () => props.stage;
    const here = () => validateStages(s.stages()).filter((p) => p.stage === stage().id);
    const needs = () => stage().takes.includes("needs");
    return (
        <li class="kanban-stages__row" data-stage-colour={stage().colour}>
            <div class="kanban-stages__line">
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
                <Select
                    density="compact"
                    ariaLabel={t("settings.kanban.colour")}
                    value={stage().colour}
                    disabled={!s.editable()}
                    onChange={(v) => s.recolour(stage().id, v as StageColour)}
                    options={colourOptions()}
                />
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
                    fallback={<span class="kanban-stages__required">{t("settings.kanban.required")}</span>}
                >
                    <IconButton
                        icon="trash-can"
                        label={t("settings.kanban.removeStage", { name: stage().name })}
                        disabled={!s.editable() || !canRemove(s.stages(), stage().id)}
                        onClick={() => setRemoving(true)}
                    />
                </Show>
            </div>
            <Show when={!needs()}>
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
                {props.s.problems().map((p) => p.message).join(". ")}
            </p>
        </Show>
    );
}

export function Notices(props: Part): JSX.Element {
    const s = props.s;
    return (
        <>
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
