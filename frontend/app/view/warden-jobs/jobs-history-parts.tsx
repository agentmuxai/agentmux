// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The parts of one job's history (jobs-history.tsx lays them out): the title, the actions, the dry run's
// place, and the runs, newest first. A summary is another agent's words: shown as text, never as markup.

import { For, Show, type JSX } from "solid-js";

import { Button, CloseButton, Pill } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { t } from "@/app/i18n/i18n";
import { openLink } from "@/app/store/global";
import { revealBlock } from "@/app/util/reveal-block";
import { openDroneGraph } from "@/app/view/drone/open-drone";
import { openWarden } from "@/app/view/warden/open-warden";
import { vaultAvailable, vaultEntries } from "@/app/view/vault/vault-store";
import type { JobRunRow } from "@/types/rpc/JobRunRow";
import { formatDurationShort, formatTimeAgo } from "@/util/format-time";

import { CloudFacts, CloudRunNow } from "./cloud-job-bits";
import { JobDryRun } from "./job-dry-run";
import { costText, kanbanTask, reasonText, runBadge, stateText } from "./jobs-format";
import type { JobHistoryProps, JobHistoryState } from "./jobs-history-state";

/** A reported link: a URL, or `{ label, url }` (srv keeps only https ones). */
function reportedLinks(run: JobRunRow): { label: string; url: string }[] {
    const reported = (run.links as { reported?: unknown }).reported;
    if (!Array.isArray(reported)) return [];
    return reported.flatMap((l) => {
        if (typeof l === "string") return [{ label: l, url: l }];
        if (l && typeof l === "object" && typeof (l as { url?: unknown }).url === "string") {
            const { url, label } = l as { url: string; label?: unknown };
            return [{ label: typeof label === "string" && label ? label : url, url }];
        }
        return [];
    }).filter((l) => l.url.startsWith("https://"));
}

function transcriptBlock(run: JobRunRow): string | null {
    const transcript = (run.links as { transcript?: { block_id?: unknown } }).transcript;
    return typeof transcript?.block_id === "string" && transcript.block_id ? transcript.block_id : null;
}

function droneRunId(run: JobRunRow): string | null {
    const id = (run.links as { drone_run_id?: unknown }).drone_run_id;
    return typeof id === "string" && id ? id : null;
}

function RunRow(props: { run: JobRunRow; now: number }): JSX.Element {
    const badge = () => runBadge(props.run);
    const r = () => props.run;
    const when = () => r().scheduled_for_ms ?? r().started_at_ms;
    const duration = () => (r().ended_at_ms != null && r().state === "finished" ? formatDurationShort(r().ended_at_ms! - r().started_at_ms) : "");
    const cost = () => (r().cost_known ? costText(r().cost_usd, true) + (r().cost_partial ? ` (${t("jobs.history.sharedTurn")})` : "") : "");
    const transcript = () => transcriptBlock(r());
    return (
        <tr data-failed={r().failed ? "true" : undefined}>
            <td {...tip(new Date(when()).toLocaleString())}>
                {formatTimeAgo(when(), props.now)}
                <Show when={r().trigger !== "schedule"}>
                    <span class="warden-manager-dim"> · {r().trigger === "manual" ? t("jobs.history.runNow") : r().trigger.replace("_", "-")}</span>
                </Show>
            </td>
            <td>
                <Pill tone={badge().tone} title={badge().detail}>
                    {badge().label}
                </Pill>
                <Show when={r().reason && r().state !== "running"}>
                    <span class="warden-jobs-reason">{reasonText(r().reason)}</span>
                </Show>
                <Show when={r().skip_count > 1}>
                    <span class="warden-manager-dim"> ×{r().skip_count}</span>
                </Show>
            </td>
            <td class="warden-jobs-num">{duration()}</td>
            <td class="warden-jobs-summary">{r().summary}</td>
            <td class="warden-jobs-num">{cost()}</td>
            <td class="warden-manager-dim">{r().account_id || r().provider}</td>
            <td class="warden-jobs-links">
                <Show when={transcript()}>
                    {(b) => (
                        <Button density="compact" tone="quiet" icon="message" onClick={() => void revealBlock(b(), { source: "jobs" })}>
                            {t("jobs.history.agent")}
                        </Button>
                    )}
                </Show>
                <Show when={kanbanTask(r())}>
                    {(task) => (
                        <Button density="compact" tone="quiet" icon="table-columns" title={t("jobs.history.incidentTip")} onClick={() => void openWarden("kanban")}>
                            {t("jobs.history.kanban", { num: task().num })}
                        </Button>
                    )}
                </Show>
                <Show when={droneRunId(r())}>{(id) => <span class="warden-manager-dim" {...tip(id())}>{t("jobs.history.droneRun")}</span>}</Show>
                <For each={reportedLinks(r())}>
                    {(l) => (
                        <Button density="compact" tone="quiet" icon="arrow-up-right-from-square" title={l.url} onClick={() => void openLink(l.url)}>
                            {l.label.length > 24 ? `${l.label.slice(0, 23)}…` : l.label}
                        </Button>
                    )}
                </For>
            </td>
        </tr>
    );
}

/** A failing job's incident: its Kanban task and the failures in a row. */
function IncidentButton(props: { num: number; failures: number }): JSX.Element {
    return (
        <Button
            density="compact"
            tone="quiet"
            icon="table-columns"
            title={t("jobs.history.failingTip")}
            onClick={() => void openWarden("kanban")}
        >
            {t("jobs.history.failing", { num: props.num, failures: props.failures })}
        </Button>
    );
}

/** The job's name and state, its owner and target, the cloud facts and the incident. */
export function HistoryTitle(part: { props: JobHistoryProps }): JSX.Element {
    const { props } = part;
    const state = () => stateText(props.job.state);
    return (
        <div class="warden-jobs-history-title">
            <strong>{props.job.name}</strong>
            <Pill tone={state().tone}>{state().label}</Pill>
            <Show when={props.job.owner}>
                <span class="warden-manager-dim">{t("jobs.history.by", { owner: props.job.owner })}</span>
            </Show>
            <Show when={props.job.target}>
                <span class="warden-manager-dim">→ {props.job.target}</span>
            </Show>
            <Show when={props.job.cloud}>{(c) => <CloudFacts cloud={c()} />}</Show>
            <Show when={props.job.incident?.task_num}>
                {(num) => <IncidentButton num={num()} failures={props.job.incident!.failures} />}
            </Show>
        </div>
    );
}

/** Run now, pause or resume, delete, edit, clone, dry run, the template update, the owner and close. */
export function HistoryActions(part: { props: JobHistoryProps; h: JobHistoryState }): JSX.Element {
    const { props, h } = part;
    const { busy, doAct, doDryRun, confirmDelete, setConfirmDelete } = h;
    const paused = () => props.job.state === "paused" || props.job.state === "ended";
    return (
        <div class="warden-jobs-actions">
            <Show when={props.job.actions}>
                <Button
                    density="compact"
                    icon="play"
                    busy={busy() === "run_now"}
                    disabled={props.pausedAll || busy() != null}
                    title={props.pausedAll ? t("jobs.history.allPaused") : t("jobs.history.runOnce")}
                    onClick={() => void doAct("run_now")}
                >
                    {t("jobs.history.runNow")}
                </Button>
                <Show
                    when={paused()}
                    fallback={
                        <Button density="compact" icon="pause" busy={busy() === "pause"} disabled={busy() != null} onClick={() => void doAct("pause")}>
                            {t("jobs.history.pause")}
                        </Button>
                    }
                >
                    <Button density="compact" icon="play" busy={busy() === "resume"} disabled={busy() != null} onClick={() => void doAct("resume")}>
                        {t("jobs.history.resume")}
                    </Button>
                </Show>
                <Show
                    when={confirmDelete()}
                    fallback={
                        <Button density="compact" tone="danger" icon="trash" disabled={busy() != null} onClick={() => setConfirmDelete(true)}>
                            {t("jobs.history.delete")}
                        </Button>
                    }
                >
                    <Button density="compact" tone="danger" icon="trash" busy={busy() === "delete"} onClick={() => void doAct("delete")}>
                        {t("jobs.history.deleteKeep")}
                    </Button>
                    <Button density="compact" tone="quiet" onClick={() => setConfirmDelete(false)}>
                        {t("jobs.history.keep")}
                    </Button>
                </Show>
            </Show>
            <Show when={props.job.actions}>
                <Button density="compact" icon="pen" disabled={busy() != null} onClick={() => props.onEdit()}>
                    {t("jobs.history.edit")}
                </Button>
                <Button density="compact" icon="clone" disabled={busy() != null} onClick={() => props.onClone()}>
                    {t("jobs.history.clone")}
                </Button>
                <Button density="compact" icon="vial" busy={busy() === "dryrun"} disabled={busy() != null} title={t("jobs.history.dryRunTip")} onClick={() => void doDryRun()}>
                    {t("jobs.history.dryRun")}
                </Button>
            </Show>
            <Show when={props.job.cloud}>{(c) => <CloudRunNow cloud={c()} />}</Show>
            <Show when={props.job.template?.update}>
                {(u) => (
                    <Button density="compact" tone="accent" icon="circle-up" onClick={() => props.onReviewUpdate()}>
                        {u().approved ? t("jobs.history.updateTo", { version: u().version }) : t("jobs.history.review", { version: u().version })}
                    </Button>
                )}
            </Show>
            <Show when={props.job.source === "drone"}>
                <Button density="compact" icon="diagram-project" title={t("jobs.openGraphTip")} onClick={() => void openDroneGraph(props.job.id)}>
                    {t("jobs.openGraph")}
                </Button>
            </Show>
            <Show when={props.job.owner_block_id}>
                {(b) => (
                    <Button density="compact" tone="quiet" icon="user" onClick={() => void revealBlock(b(), { source: "jobs" })}>
                        {t("jobs.history.openOwner")}
                    </Button>
                )}
            </Show>
            <CloseButton size="sm" text={t("jobs.history.close")} onClick={() => props.onClose()} />
        </div>
    );
}

/** What the dry run would do, where the stored job's vault and the agents' names are read. */
export function HistoryDryRun(part: { h: JobHistoryState }): JSX.Element {
    const { dry, agents } = part.h;
    return (
        <Show when={dry()}>
            {(d) => (
                <JobDryRun
                    result={d()}
                    vault={vaultAvailable() ? (vaultEntries() ?? []) : null}
                    nameOf={(id) => agents()?.find((a) => a.id === id)?.name ?? id}
                />
            )}
        </Show>
    );
}

/** The runs, newest first, and the button for the older ones. */
export function HistoryRuns(part: { props: JobHistoryProps; h: JobHistoryState }): JSX.Element {
    const { props, h } = part;
    const { runs, more, loadOlder } = h;
    return (
        <Show when={runs().length > 0} fallback={<div class="warden-section-stub">{t("jobs.history.noRuns")}</div>}>
            <table class="warden-manager-table warden-jobs-runs">
                <thead>
                    <tr>
                        <th>{t("jobs.history.colWhen")}</th>
                        <th>{t("jobs.history.colRun")}</th>
                        <th>{t("jobs.history.colTook")}</th>
                        <th>{t("jobs.history.colSummary")}</th>
                        <th>{t("jobs.history.colCost")}</th>
                        <th>{t("jobs.history.colAccount")}</th>
                        <th>{t("jobs.history.colLinks")}</th>
                    </tr>
                </thead>
                <tbody>
                    <For each={runs()}>{(run) => <RunRow run={run} now={props.now} />}</For>
                </tbody>
            </table>
            <Show when={more()}>
                <Button density="compact" tone="quiet" onClick={() => void loadOlder()}>
                    {t("jobs.history.older")}
                </Button>
            </Show>
        </Show>
    );
}
