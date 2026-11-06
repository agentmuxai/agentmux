// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What one job's history holds (jobs-history.tsx draws it): the runs shown and how they are loaded, reloaded
// and paged, the job's actions and the dry run, and the note an action or a load leaves.

import { createEffect, createSignal, on, type Setter } from "solid-js";

import { useAgentList } from "@/app/view/agent/use-agent-list";
import type { JobRunRow } from "@/types/rpc/JobRunRow";
import type { JobsDryRunResult } from "@/types/rpc/JobsDryRunResult";
import type { JobsActRequest } from "@/types/rpc/JobsActRequest";
import type { JobsActResult } from "@/types/rpc/JobsActResult";
import type { JobsRunsResult } from "@/types/rpc/JobsRunsResult";
import type { JobView } from "@/types/rpc/JobView";

import { RUNS_MAX, RUNS_PAGE, type JobChange } from "./jobs-model";

export interface JobHistoryProps {
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
}

/** `fresh` (a reload from the newest run) with the older rows already loaded below it kept, so a
 *  reload doesn't take the view back to the first page. Exported for its test. */
export function mergeReload(fresh: JobRunRow[], shown: JobRunRow[]): JobRunRow[] {
    if (fresh.length === 0) return fresh;
    const seen = new Set(fresh.map((r) => r.id));
    const oldest = fresh[fresh.length - 1].started_at_ms;
    return [...fresh, ...shown.filter((r) => !seen.has(r.id) && r.started_at_ms <= oldest)];
}

/** The runs shown, loaded a page at a time and read again when the job changes or the list polls. */
function createRunsLoader(props: JobHistoryProps, setNote: Setter<string | null>, onJobOpened: () => void) {
    const [runs, setRuns] = createSignal<JobRunRow[]>([]);
    const [more, setMore] = createSignal(false);

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
                onJobOpened();
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

    return { runs, more, loadOlder };
}

/** The job's actions and its dry run: which one is running, and what each left behind. */
function createJobActions(
    props: JobHistoryProps,
    ui: { setNote: Setter<string | null>; setConfirmDelete: Setter<boolean>; setDry: Setter<JobsDryRunResult | null> }
) {
    const [busy, setBusy] = createSignal<string | null>(null);

    const doAct = async (action: JobsActRequest["action"]) => {
        setBusy(action);
        ui.setNote(null);
        try {
            const res = await props.act({ action, job_id: props.job.id });
            if (action === "run_now") ui.setNote(res.ok ? "Started." : `Not run: ${res.error ?? res.outcome ?? "unknown"}`);
            if (action === "delete") props.onClose();
        } catch (e) {
            ui.setNote(String(e));
        } finally {
            setBusy(null);
            ui.setConfirmDelete(false);
        }
    };

    const doDryRun = async () => {
        setBusy("dryrun");
        ui.setNote(null);
        try {
            ui.setDry(await props.dryRun(props.job.id));
        } catch (e) {
            ui.setNote(String(e));
        } finally {
            setBusy(null);
        }
    };

    return { busy, doAct, doDryRun };
}

export function createJobHistory(props: JobHistoryProps) {
    const [note, setNote] = createSignal<string | null>(null);
    const [confirmDelete, setConfirmDelete] = createSignal(false);
    const [dry, setDry] = createSignal<JobsDryRunResult | null>(null);
    const [agents] = useAgentList();
    const loader = createRunsLoader(props, setNote, () => {
        setConfirmDelete(false);
        setDry(null);
    });
    const actions = createJobActions(props, { setNote, setConfirmDelete, setDry });
    return { note, confirmDelete, setConfirmDelete, dry, agents, ...loader, ...actions };
}

export type JobHistoryState = ReturnType<typeof createJobHistory>;
