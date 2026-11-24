// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — Jobs section: everything that runs by itself, one row per job, with its trigger and next
// run, its last run's outcome, its recent runs, its spend and its state; a job's runs on click; and
// the operator's Pause all (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.3).

import { createEffect, createMemo, createSignal, For, onCleanup, Show, type JSX } from "solid-js";

import { Button, Notice, Pill } from "@/app/element/ui";
import type { JobView } from "@/types/rpc/JobView";
import { formatTimeAgo } from "@/util/format-time";

import { CloudReporters } from "./cloud-reporters";
import { costText, describeTrigger, droppedText, failingCount, inWords, runBadge, stateText } from "./jobs-format";
import { JobEditor, type EditorStart } from "./job-editor";
import { JobUpdateDiff } from "./job-update-diff";
import { JobHistory } from "./jobs-history";
import { JobsModel } from "./jobs-model";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./warden-jobs.scss";
import { tip } from "@/app/element/tip";
import { t } from "@/app/i18n/i18n";

function Strip(props: { job: JobView }): JSX.Element {
    // Oldest on the left, as time reads.
    const marks = () => [...props.job.recent].reverse();
    return (
        <span class="warden-jobs-strip" aria-label={t("jobs.manager.lastRuns", { count: props.job.recent.length })}>
            <For each={marks()}>
                {(run) => {
                    const b = runBadge(run);
                    return <span class="warden-jobs-mark" data-tone={b.tone} data-hollow={b.hollow ? "true" : undefined} {...tip(b.detail, { label: true })} />;
                }}
            </For>
        </span>
    );
}

function JobRow(props: { job: JobView; now: number; selected: boolean; onOpen: () => void }): JSX.Element {
    const last = () => props.job.recent[0];
    const state = () => stateText(props.job.state);
    const trigger = () => describeTrigger(props.job);
    return (
        <tr
            class="warden-jobs-row"
            data-selected={props.selected ? "true" : undefined}
            data-failing={props.job.failing ? "true" : undefined}
            tabIndex={0}
            onClick={() => props.onOpen()}
            onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    props.onOpen();
                }
            }}
        >
            <td>
                <div class="warden-jobs-name">
                    {props.job.name}
                    <Show when={props.job.source === "drone"}>
                        <Pill>{t("jobs.manager.drone")}</Pill>
                    </Show>
                    <Show when={props.job.cloud}>
                        {(c) => (
                            <>
                                <Pill icon="cloud">{t("jobs.manager.cloud")}</Pill>
                                <Show when={!c().reporter_pinned}>
                                    <Pill tone="warn" title={t("jobs.manager.notPinnedTip", { reporter: c().reporter })}>
                                        {t("jobs.manager.notPinned")}
                                    </Pill>
                                </Show>
                            </>
                        )}
                    </Show>
                    <Show when={props.job.template?.update}>
                        {(u) => (
                            <Pill
                                tone="accent"
                                icon="circle-up"
                                title={t(u().approved ? "jobs.manager.installed" : "jobs.manager.installedWaiting", { id: props.job.template?.id, version: u().version })}
                            >
                                {u().approved ? t("jobs.manager.updateAvailable") : t("jobs.manager.updateToApprove")}
                            </Pill>
                        )}
                    </Show>
                </div>
                <div class="warden-manager-dim">
                    {props.job.owner ? t("jobs.manager.by", { owner: props.job.owner }) : t("jobs.manager.yours")}
                    <Show when={props.job.cloud}>{(c) => ` · ${t("jobs.manager.reportsAs", { reporter: c().reporter })}`}</Show>
                </div>
            </td>
            <td {...tip(props.job.expression || undefined)}>
                <div>{trigger()}</div>
                <Show when={props.job.next_run_ms}>{(next) => <div class="warden-manager-dim">{inWords(next(), props.now)}</div>}</Show>
            </td>
            <td>
                <Show when={last()} fallback={<span class="warden-manager-dim">{t("jobs.manager.neverRun")}</span>}>
                    {(run) => {
                        const b = () => runBadge(run());
                        return (
                            <>
                                <Pill tone={b().tone} title={b().detail}>
                                    {b().label}
                                </Pill>
                                <div class="warden-manager-dim">{formatTimeAgo(run().scheduled_for_ms ?? run().started_at_ms, props.now)}</div>
                            </>
                        );
                    }}
                </Show>
            </td>
            <td>
                <Strip job={props.job} />
            </td>
            <td class="warden-jobs-num" {...tip("Today, and the last 7 days")}>
                {costText(props.job.cost_today_usd, props.job.cost_known)}
                <span class="warden-manager-dim"> / {costText(props.job.cost_week_usd, props.job.cost_known)}</span>
            </td>
            <td>
                <Pill tone={state().tone}>{state().label}</Pill>
            </td>
        </tr>
    );
}

function Toolbar(props: {
    paused: boolean;
    busy: boolean;
    onNew: () => void;
    onReporters: () => void;
    onTogglePause: () => void;
}): JSX.Element {
    return (
        <div class="warden-jobs-toolbar">
            <p class="warden-manager-summary">
                {t("jobs.manager.about")}
            </p>
            <div class="warden-jobs-actions">
                <Button density="compact" tone="accent" icon="plus" onClick={() => props.onNew()}>
                    {t("jobs.manager.newJob")}
                </Button>
                <Button density="compact" icon="cloud" onClick={() => props.onReporters()}>
                    {t("jobs.manager.reporters")}
                </Button>
                <Button
                    density="compact"
                    tone={props.paused ? "accent" : "danger"}
                    icon={props.paused ? "play" : "pause"}
                    busy={props.busy}
                    onClick={() => props.onTogglePause()}
                >
                    {props.paused ? t("jobs.manager.resumeAll") : t("jobs.manager.pauseAll")}
                </Button>
            </div>
        </div>
    );
}

export const WardenJobsManager = (props: {
    visible?: boolean;
    /** How many jobs are failing, for the pane's title. */
    onFailingCount?: (n: number) => void;
}): JSX.Element => {
    const model = new JobsModel();
    onCleanup(() => model.stop());
    createEffect(() => model.setVisible(props.visible ?? true));

    const [selected, setSelected] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    const [note, setNote] = createSignal<string | null>(null);
    const [editor, setEditor] = createSignal<EditorStart | null>(null);
    const [updating, setUpdating] = createSignal<string | null>(null);
    const [reportersOpen, setReportersOpen] = createSignal(false);

    const list = model.list;
    const jobs = createMemo(() => list()?.jobs ?? []);
    const now = () => list()?.now_ms ?? Date.now();
    const pausedAll = () => list()?.paused_all ?? null;
    const selectedJob = createMemo(() => jobs().find((j) => j.id === selected()) ?? null);
    const openEditor = (start: EditorStart) => {
        setUpdating(null);
        setEditor(start);
    };
    createEffect(() => props.onFailingCount?.(failingCount(jobs())));

    const togglePauseAll = async () => {
        setBusy(true);
        setNote(null);
        try {
            await model.act({ action: pausedAll() ? "resume_all" : "pause_all" });
        } catch (e) {
            setNote(String(e));
        } finally {
            setBusy(false);
        }
    };

    return (
        <div class="warden-manager-body warden-jobs">
            <Toolbar
                paused={pausedAll() != null}
                busy={busy()}
                onNew={() => openEditor({ mode: "new" })}
                onReporters={() => setReportersOpen(!reportersOpen())}
                onTogglePause={() => void togglePauseAll()}
            />
            {/* keyed: opening Edit on another job starts a fresh editor. */}
            <Show when={editor()} keyed>
                {(start) => <JobEditor start={start} model={model} defaults={list()?.defaults} onClose={() => setEditor(null)} />}
            </Show>
            <Show when={updating()} keyed>
                {(id) => (
                    <Show when={jobs().find((j) => j.id === id)}>
                        {(job) => <JobUpdateDiff job={job()} model={model} onClose={() => setUpdating(null)} />}
                    </Show>
                )}
            </Show>
            <Show when={pausedAll()}>
                {(p) => (
                    <Notice tone="warning">
                        {t("jobs.manager.allPaused", { when: formatTimeAgo(p().set_at_ms, now()) })}
                    </Notice>
                )}
            </Show>
            <Show when={reportersOpen()}>
                <CloudReporters model={model} polled={model.polled()} onClose={() => setReportersOpen(false)} />
            </Show>
            <Show when={droppedText(list()?.dropped)}>
                {(text) => (
                    <Notice tone="warning">{text()}</Notice>
                )}
            </Show>
            <Show when={note() ?? model.error()}>
                <div class="warden-manager-error">⚠ {note() ?? model.error()}</div>
            </Show>
            <Show
                when={jobs().length > 0}
                fallback={
                    <Show when={list() || !model.error()}>
                        <div class="warden-section-stub">
                            {list() ? t("jobs.manager.none") : t("jobs.manager.loading")}
                        </div>
                    </Show>
                }
            >
                <table class="warden-manager-table warden-jobs-table">
                    <thead>
                        <tr>
                            <th>{t("jobs.manager.colJob")}</th>
                            <th>{t("jobs.manager.colTrigger")}</th>
                            <th>{t("jobs.manager.colLastRun")}</th>
                            <th>{t("jobs.manager.colRecent")}</th>
                            <th>{t("jobs.manager.colCost")}</th>
                            <th>{t("jobs.manager.colState")}</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={jobs()}>
                            {(job) => (
                                <JobRow
                                    job={job}
                                    now={now()}
                                    selected={selected() === job.id}
                                    onOpen={() => setSelected(selected() === job.id ? null : job.id)}
                                />
                            )}
                        </For>
                    </tbody>
                </table>
            </Show>
            <Show when={selectedJob()}>
                {(job) => (
                    <JobHistory
                        job={job()}
                        now={now()}
                        pausedAll={pausedAll() != null}
                        loadRuns={(id, before, limit) => model.runs(id, before, limit)}
                        changed={model.changed()}
                        polled={model.polled()}
                        act={(req) => model.act(req)}
                        dryRun={(id) => model.dryRun({ job_id: id })}
                        onEdit={() => openEditor({ mode: "edit", jobId: job().id })}
                        onClone={() => openEditor({ mode: "clone", jobId: job().id })}
                        onReviewUpdate={() => {
                            setEditor(null);
                            setUpdating(job().id);
                        }}
                        onClose={() => setSelected(null)}
                    />
                )}
            </Show>
        </div>
    );
};
