// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One job opened in the Jobs section: its actions (Run now, Pause / Resume, Delete, Open owner) and
// its runs, newest first, with the slot, start, duration, state and reason, outcome and summary,
// cost and account, and links (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.3), the Kanban
// task of a failing job's incident among them (§3.4). A summary is another agent's words: shown as
// text, never as markup.

import { createEffect, createSignal, For, on, Show, type JSX } from "solid-js";

import { Button, Pill } from "@/app/element/ui";
import { openLink } from "@/app/store/global";
import { revealBlock } from "@/app/util/reveal-block";
import { useAgentList } from "@/app/view/agent/use-agent-list";
import { vaultAvailable, vaultEntries } from "@/app/view/vault/vault-store";
import { openWarden } from "@/app/view/warden/open-warden";
import type { JobRunRow } from "@/types/rpc/JobRunRow";
import type { JobsDryRunResult } from "@/types/rpc/JobsDryRunResult";
import type { JobsActRequest } from "@/types/rpc/JobsActRequest";
import type { JobsActResult } from "@/types/rpc/JobsActResult";
import type { JobsRunsResult } from "@/types/rpc/JobsRunsResult";
import type { JobView } from "@/types/rpc/JobView";
import { formatDurationShort, formatTimeAgo } from "@/util/format-time";

import { JobDryRun } from "./job-dry-run";
import { costText, kanbanTask, reasonText, runBadge, stateText } from "./jobs-format";
import { RUNS_MAX, RUNS_PAGE, type JobChange } from "./jobs-model";

/** `fresh` (a reload from the newest run) with the older rows already loaded below it kept, so a
 *  reload doesn't take the view back to the first page. Exported for its test. */
export function mergeReload(fresh: JobRunRow[], shown: JobRunRow[]): JobRunRow[] {
    if (fresh.length === 0) return fresh;
    const seen = new Set(fresh.map((r) => r.id));
    const oldest = fresh[fresh.length - 1].started_at_ms;
    return [...fresh, ...shown.filter((r) => !seen.has(r.id) && r.started_at_ms <= oldest)];
}

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
    const t = (run.links as { transcript?: { block_id?: unknown } }).transcript;
    return typeof t?.block_id === "string" && t.block_id ? t.block_id : null;
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
    const cost = () => (r().cost_known ? costText(r().cost_usd, true) + (r().cost_partial ? " (shared turn)" : "") : "");
    const transcript = () => transcriptBlock(r());
    return (
        <tr data-failed={r().failed ? "true" : undefined}>
            <td title={new Date(when()).toLocaleString()}>
                {formatTimeAgo(when(), props.now)}
                <Show when={r().trigger !== "schedule"}>
                    <span class="warden-manager-dim"> · {r().trigger === "manual" ? "Run now" : r().trigger.replace("_", "-")}</span>
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
                            Agent
                        </Button>
                    )}
                </Show>
                <Show when={kanbanTask(r())}>
                    {(t) => (
                        <Button density="compact" tone="quiet" icon="table-columns" title="The incident's task, in the Kanban" onClick={() => void openWarden("kanban")}>
                            Kanban #{t().num}
                        </Button>
                    )}
                </Show>
                <Show when={droneRunId(r())}>{(id) => <span class="warden-manager-dim" title={id()}>Drone run</span>}</Show>
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

export function JobHistory(props: {
    job: JobView;
    now: number;
    pausedAll: boolean;
    loadRuns: (jobId: string, before?: number, limit?: number) => Promise<JobsRunsResult>;
    /** The last job srv said changed (`jobs:changed`): this job's, or every job's, reloads the runs. */
    changed: JobChange;
    /** Counts the list's polls; each reloads the runs too. */
    polled: number;
    act: (req: JobsActRequest) => Promise<JobsActResult>;
    /** What the stored job would do, starting nothing. */
    dryRun: (jobId: string) => Promise<JobsDryRunResult>;
    onEdit: () => void;
    onClone: () => void;
    onReviewUpdate: () => void;
    onClose: () => void;
}): JSX.Element {
    const [runs, setRuns] = createSignal<JobRunRow[]>([]);
    const [more, setMore] = createSignal(false);
    const [busy, setBusy] = createSignal<string | null>(null);
    const [note, setNote] = createSignal<string | null>(null);
    const [confirmDelete, setConfirmDelete] = createSignal(false);
    const [dry, setDry] = createSignal<JobsDryRunResult | null>(null);
    const [agents] = useAgentList();

    // Every request remembers the job it was for and the generation it was made in; an answer that
    // comes back after another job was opened, or after a newer reload, is dropped, so it can't show
    // under the wrong job or put back older rows.
    let generation = 0;
    let reloads = 0;
    const current = (gen: number, jobId: string) => gen === generation && jobId === props.job.id;
    const ask = async (before?: number, limit?: number): Promise<JobRunRow[] | null> => {
        try {
            return (await props.loadRuns(props.job.id, before, limit)).runs;
        } catch (e) {
            setNote(String(e));
            return null;
        }
    };

    /** Another job was opened: its first page. */
    const loadFirst = async () => {
        const gen = ++generation;
        const jobId = props.job.id;
        setRuns([]);
        setMore(false);
        const got = await ask(undefined, RUNS_PAGE);
        if (got == null || !current(gen, jobId)) return;
        setRuns(got);
        setMore(got.length >= RUNS_PAGE);
    };

    /** The next page below the rows shown. */
    const loadOlder = async () => {
        const gen = generation;
        const jobId = props.job.id;
        const last = runs()[runs().length - 1];
        if (!last) return;
        const got = await ask(last.started_at_ms, RUNS_PAGE);
        if (got == null || !current(gen, jobId)) return;
        const seen = new Set(runs().map((r) => r.id));
        setRuns([...runs(), ...got.filter((r) => !seen.has(r.id))]);
        setMore(got.length >= RUNS_PAGE);
    };

    /** The same job changed: the rows shown, as deep as they go, read again. */
    const reload = async () => {
        const gen = generation;
        const jobId = props.job.id;
        const mine = ++reloads;
        const depth = Math.min(Math.max(RUNS_PAGE, runs().length), RUNS_MAX);
        const got = await ask(undefined, depth);
        if (got == null || !current(gen, jobId) || mine !== reloads) return;
        setRuns(mergeReload(got, runs()));
    };

    createEffect(
        on(
            () => props.job.id,
            () => {
                setConfirmDelete(false);
                setDry(null);
                void loadFirst();
            }
        )
    );
    createEffect(
        on(
            () => props.changed,
            (c) => {
                if (c.jobId === "" || c.jobId === props.job.id) void reload();
            },
            { defer: true }
        )
    );
    createEffect(on(() => props.polled, () => void reload(), { defer: true }));

    const doAct = async (action: JobsActRequest["action"]) => {
        setBusy(action);
        setNote(null);
        try {
            const res = await props.act({ action, job_id: props.job.id });
            if (action === "run_now") setNote(res.ok ? "Started." : `Not run: ${res.error ?? res.outcome ?? "unknown"}`);
            if (action === "delete") props.onClose();
        } catch (e) {
            setNote(String(e));
        } finally {
            setBusy(null);
            setConfirmDelete(false);
        }
    };

    const doDryRun = async () => {
        setBusy("dryrun");
        setNote(null);
        try {
            setDry(await props.dryRun(props.job.id));
        } catch (e) {
            setNote(String(e));
        } finally {
            setBusy(null);
        }
    };

    const state = () => stateText(props.job.state);
    const paused = () => props.job.state === "paused" || props.job.state === "ended";

    return (
        <section class="warden-jobs-history" aria-label={`Runs of ${props.job.name}`}>
            <header class="warden-jobs-history-head">
                <div class="warden-jobs-history-title">
                    <strong>{props.job.name}</strong>
                    <Pill tone={state().tone}>{state().label}</Pill>
                    <Show when={props.job.owner}>
                        <span class="warden-manager-dim">by {props.job.owner}</span>
                    </Show>
                    <Show when={props.job.target}>
                        <span class="warden-manager-dim">→ {props.job.target}</span>
                    </Show>
                    <Show when={props.job.cloud}>
                        {(c) => (
                            <span class="warden-manager-dim">
                                reports as {c().reporter}
                                {c().grace_minutes != null ? ` · ${c().grace_minutes} min grace` : ""}
                                {c().max_run_minutes != null ? ` · lost after ${c().max_run_minutes} min` : ""}
                            </span>
                        )}
                    </Show>
                    <Show when={props.job.incident?.task_num}>
                        {(num) => (
                            <Button
                                density="compact"
                                tone="quiet"
                                icon="table-columns"
                                title="Its failures go to this task until the job succeeds again"
                                onClick={() => void openWarden("kanban")}
                            >
                                Failing: Kanban #{num()} · {props.job.incident!.failures} in a row
                            </Button>
                        )}
                    </Show>
                </div>
                <div class="warden-jobs-actions">
                    <Show when={props.job.actions}>
                        <Button
                            density="compact"
                            icon="play"
                            busy={busy() === "run_now"}
                            disabled={props.pausedAll || busy() != null}
                            title={props.pausedAll ? "Every job is paused (Pause all)" : "Run it once now"}
                            onClick={() => void doAct("run_now")}
                        >
                            Run now
                        </Button>
                        <Show
                            when={paused()}
                            fallback={
                                <Button density="compact" icon="pause" busy={busy() === "pause"} disabled={busy() != null} onClick={() => void doAct("pause")}>
                                    Pause
                                </Button>
                            }
                        >
                            <Button density="compact" icon="play" busy={busy() === "resume"} disabled={busy() != null} onClick={() => void doAct("resume")}>
                                Resume
                            </Button>
                        </Show>
                        <Show
                            when={confirmDelete()}
                            fallback={
                                <Button density="compact" tone="danger" icon="trash" disabled={busy() != null} onClick={() => setConfirmDelete(true)}>
                                    Delete
                                </Button>
                            }
                        >
                            <Button density="compact" tone="danger" icon="trash" busy={busy() === "delete"} onClick={() => void doAct("delete")}>
                                Delete it, keep its runs
                            </Button>
                            <Button density="compact" tone="quiet" onClick={() => setConfirmDelete(false)}>
                                Keep
                            </Button>
                        </Show>
                    </Show>
                    <Show when={props.job.actions}>
                        <Button density="compact" icon="pen" disabled={busy() != null} onClick={() => props.onEdit()}>
                            Edit
                        </Button>
                        <Button density="compact" icon="clone" disabled={busy() != null} onClick={() => props.onClone()}>
                            Clone
                        </Button>
                        <Button density="compact" icon="vial" busy={busy() === "dryrun"} disabled={busy() != null} title="What it would do, starting nothing" onClick={() => void doDryRun()}>
                            Dry run
                        </Button>
                    </Show>
                    <Show when={props.job.cloud?.run_now_link}>
                        {(url) => (
                            <Button
                                density="compact"
                                icon="arrow-up-right-from-square"
                                title={`Opens ${url()}: the job runs elsewhere, so AgentMux starts nothing`}
                                disabled={!url().startsWith("https://")}
                                onClick={() => void openLink(url())}
                            >
                                Run now
                            </Button>
                        )}
                    </Show>
                    <Show when={!props.job.cloud?.run_now_link && props.job.cloud?.run_now_ask_owner}>
                        {(ask) => (
                            <span class="warden-manager-dim" title="The job runs elsewhere, so AgentMux starts nothing">
                                To run it now: {ask()}
                            </span>
                        )}
                    </Show>
                    <Show when={props.job.template?.update}>
                        {(u) => (
                            <Button density="compact" tone="accent" icon="circle-up" onClick={() => props.onReviewUpdate()}>
                                {u().approved ? `Update to ${u().version}` : `Review ${u().version}`}
                            </Button>
                        )}
                    </Show>
                    <Show when={props.job.owner_block_id}>
                        {(b) => (
                            <Button density="compact" tone="quiet" icon="user" onClick={() => void revealBlock(b(), { source: "jobs" })}>
                                Open owner
                            </Button>
                        )}
                    </Show>
                    <Button density="compact" tone="quiet" icon="xmark" onClick={() => props.onClose()}>
                        Close
                    </Button>
                </div>
            </header>
            <Show when={note()}>
                <div class="warden-manager-error">{note()}</div>
            </Show>
            <Show when={dry()}>
                {(d) => (
                    <JobDryRun
                        result={d()}
                        vault={vaultAvailable() ? (vaultEntries() ?? []) : null}
                        nameOf={(id) => agents()?.find((a) => a.id === id)?.name ?? id}
                    />
                )}
            </Show>
            <Show when={runs().length > 0} fallback={<div class="warden-section-stub">No runs yet.</div>}>
                <table class="warden-manager-table warden-jobs-runs">
                    <thead>
                        <tr>
                            <th>When</th>
                            <th>Run</th>
                            <th>Took</th>
                            <th>Summary</th>
                            <th>Cost</th>
                            <th>Account</th>
                            <th>Links</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={runs()}>{(run) => <RunRow run={run} now={props.now} />}</For>
                    </tbody>
                </table>
                <Show when={more()}>
                    <Button density="compact" tone="quiet" onClick={() => void loadOlder()}>
                        Older runs
                    </Button>
                </Show>
            </Show>
        </section>
    );
}
