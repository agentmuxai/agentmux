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
import type { JobsActRequest } from "@/types/rpc/JobsActRequest";
import type { JobsActResult } from "@/types/rpc/JobsActResult";
import type { JobsListResult } from "@/types/rpc/JobsListResult";
import type { JobsRunsResult } from "@/types/rpc/JobsRunsResult";

const POLL_MS = 15_000;
const DEBOUNCE_MS = 200;

export class JobsModel {
    readonly list: Accessor<JobsListResult | null>;
    readonly error: Accessor<string | null>;
    private readonly setList: (l: JobsListResult | null) => void;
    private readonly setError: (e: string | null) => void;
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
    }

    /** Follow the jobs while the section is shown; stop when it isn't. */
    setVisible(visible: boolean): void {
        if (visible && this.unsubs.length === 0) {
            this.unsubs = [
                onMuxEvent(WpsEvent.JobsChanged, () => this.scheduleRefresh()),
                onMuxEvent(WpsEvent.CronChanged, () => this.scheduleRefresh()),
            ];
            this.poll = setInterval(() => void this.refresh(), POLL_MS);
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

    /** One job's runs, newest first; `before` pages on from the last one shown. */
    runs(jobId: string, before?: number): Promise<JobsRunsResult> {
        return RpcApi.JobsRunsCommand(TabRpcClient, { job_id: jobId, limit: 50, ...(before != null ? { before } : {}) });
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
