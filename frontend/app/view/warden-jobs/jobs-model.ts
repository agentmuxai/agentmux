// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Jobs section's state: every job from srv (`jobs.list`), refreshed when the run ledger or the
// cron jobs change (`jobs:changed`, `cron_changed`) and every 15 s while the section is shown, since a
// job another AgentMux on this computer fires raises no event here, and "in 3 min" goes stale
// (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.3). Nothing is fetched or subscribed while the
// section is hidden: Warden keeps every section mounted.

import { createSignal, type Accessor } from "solid-js";

import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { JobReportersResult } from "@/types/rpc/JobReportersResult";
import type { JobTemplateApproveReq } from "@/types/rpc/JobTemplateApproveReq";
import type { JobTemplateDetail } from "@/types/rpc/JobTemplateDetail";
import type { JobTemplatesResult } from "@/types/rpc/JobTemplatesResult";
import type { JobsActRequest } from "@/types/rpc/JobsActRequest";
import type { JobsActResult } from "@/types/rpc/JobsActResult";
import type { JobsDryRunRequest } from "@/types/rpc/JobsDryRunRequest";
import type { JobsDryRunResult } from "@/types/rpc/JobsDryRunResult";
import type { JobsGetResult } from "@/types/rpc/JobsGetResult";
import type { JobsListResult } from "@/types/rpc/JobsListResult";
import type { JobsRunsResult } from "@/types/rpc/JobsRunsResult";
import type { JobsSaveRequest } from "@/types/rpc/JobsSaveRequest";
import type { JobsSaveResult } from "@/types/rpc/JobsSaveResult";
import type { TemplateUpdateDiff } from "@/types/rpc/TemplateUpdateDiff";

const POLL_MS = 15_000;
const DEBOUNCE_MS = 200;
/** A history page, and the most runs one `jobs.runs` returns (srv's cap). */
export const RUNS_PAGE = 50;
export const RUNS_MAX = 200;

/** The last `jobs:changed`: the job whose runs changed (`""`: every job), numbered so the same job
 *  changing twice is two changes. */
export interface JobChange {
    jobId: string;
    seq: number;
}

export class JobsModel {
    readonly list: Accessor<JobsListResult | null>;
    readonly error: Accessor<string | null>;
    /** The last job srv said changed, for an open history to reload its own job's runs. */
    readonly changed: Accessor<JobChange>;
    /** Counts the 15 s polls: a run another AgentMux on this computer ends raises no event here. */
    readonly polled: Accessor<number>;
    private readonly setList: (l: JobsListResult | null) => void;
    private readonly setError: (e: string | null) => void;
    private readonly setChanged: (c: JobChange) => void;
    private readonly setPolled: (n: number) => void;
    private unsubs: Array<() => void> = [];
    private poll: ReturnType<typeof setInterval> | null = null;
    private debounce: ReturnType<typeof setTimeout> | null = null;
    private inflight = false;
    private again = false;

    constructor() {
        const [list, setList] = createSignal<JobsListResult | null>(null);
        const [error, setError] = createSignal<string | null>(null);
        this.list = list;
        this.error = error;
        this.setList = setList;
        this.setError = setError;
        const [changed, setChanged] = createSignal<JobChange>({ jobId: "", seq: 0 });
        const [polled, setPolled] = createSignal(0);
        this.changed = changed;
        this.polled = polled;
        this.setChanged = setChanged;
        this.setPolled = setPolled;
    }

    /** Follow the jobs while the section is shown; stop when it isn't. */
    setVisible(visible: boolean): void {
        if (visible && this.unsubs.length === 0) {
            this.unsubs = [
                onMuxEvent(WpsEvent.JobsChanged, (data) => {
                    this.setChanged({ jobId: data.job_id, seq: this.changed().seq + 1 });
                    this.scheduleRefresh();
                }),
                onMuxEvent(WpsEvent.CronChanged, () => this.scheduleRefresh()),
            ];
            this.poll = setInterval(() => {
                this.setPolled(this.polled() + 1);
                void this.refresh();
            }, POLL_MS);
            void this.refresh();
        } else if (!visible && this.unsubs.length > 0) {
            this.stop();
        }
    }

    stop(): void {
        for (const off of this.unsubs) off();
        this.unsubs = [];
        if (this.poll) clearInterval(this.poll);
        this.poll = null;
        if (this.debounce) clearTimeout(this.debounce);
        this.debounce = null;
    }

    private scheduleRefresh(): void {
        if (this.debounce) clearTimeout(this.debounce);
        this.debounce = setTimeout(() => {
            this.debounce = null;
            void this.refresh();
        }, DEBOUNCE_MS);
    }

    /** One `jobs.list` at a time; a refresh asked for meanwhile runs once after it. */
    async refresh(): Promise<void> {
        if (this.inflight) {
            this.again = true;
            return;
        }
        this.inflight = true;
        try {
            this.setList(await RpcApi.JobsListCommand(TabRpcClient, {}));
            this.setError(null);
        } catch (e) {
            // The last list stays: a failed refresh says nothing about it.
            this.setError(String(e));
        } finally {
            this.inflight = false;
            if (this.again) {
                this.again = false;
                void this.refresh();
            }
        }
    }

    /** One job's runs, newest first, at most `limit`; `before` pages on from the last one shown. */
    runs(jobId: string, before?: number, limit: number = RUNS_PAGE): Promise<JobsRunsResult> {
        return RpcApi.JobsRunsCommand(TabRpcClient, { job_id: jobId, limit, ...(before != null ? { before } : {}) });
    }

    /** A job's stored definition, for Edit and Clone. */
    get(jobId: string): Promise<JobsGetResult> {
        return RpcApi.JobsGetCommand(TabRpcClient, { job_id: jobId });
    }

    /** What a definition (or a stored job) would do; starts no agent. */
    dryRun(req: JobsDryRunRequest): Promise<JobsDryRunResult> {
        return RpcApi.JobsDryrunCommand(TabRpcClient, req);
    }

    /** Create a job, or edit one: the operator's window only. A refresh follows either way. */
    async save(req: JobsSaveRequest): Promise<JobsSaveResult> {
        try {
            return await RpcApi.JobsSaveCommand(TabRpcClient, req);
        } finally {
            this.scheduleRefresh();
        }
    }

    templates(): Promise<JobTemplatesResult> {
        return RpcApi.JobsTemplatesListCommand(TabRpcClient);
    }

    templateDetail(id: string): Promise<JobTemplateDetail> {
        return RpcApi.JobsTemplatesGetCommand(TabRpcClient, { id });
    }

    approveTemplate(req: JobTemplateApproveReq): Promise<JobTemplatesResult> {
        return RpcApi.JobsTemplatesApproveCommand(TabRpcClient, req);
    }

    updateDiff(jobId: string): Promise<TemplateUpdateDiff> {
        return RpcApi.JobsTemplatesUpdateDiffCommand(TabRpcClient, { job_id: jobId });
    }

    reporters(): Promise<JobReportersResult> {
        return RpcApi.JobsReportersListCommand(TabRpcClient);
    }

    /** Pin a cloud reporter's key. It lets no event in until an approved template names the same key. */
    async addReporter(name: string, publicKey: string): Promise<JobReportersResult> {
        try {
            return await RpcApi.JobsReportersAddCommand(TabRpcClient, { name, public_key: publicKey });
        } finally {
            this.scheduleRefresh();
        }
    }

    async forgetReporter(name: string): Promise<JobReportersResult> {
        try {
            return await RpcApi.JobsReportersForgetCommand(TabRpcClient, { name });
        } finally {
            this.scheduleRefresh();
        }
    }

    /** An action, then a refresh: the event usually comes first, but a refusal raises none. */
    async act(req: JobsActRequest): Promise<JobsActResult> {
        try {
            return await RpcApi.JobsActCommand(TabRpcClient, req);
        } finally {
            this.scheduleRefresh();
        }
    }
}
