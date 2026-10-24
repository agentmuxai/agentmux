// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Runs and jobs for the Jobs section's tests.

import type { JobRunRow } from "@/types/rpc/JobRunRow";
import type { JobView } from "@/types/rpc/JobView";

/** "Now" in the tests: ten days after the epoch. */
export const NOW = 10 * 86_400_000;

export function mkRun(over: Partial<JobRunRow> = {}): JobRunRow {
    return {
        id: "r1",
        job_id: "j1",
        job_source: "cron",
        job_name: "Nightly",
        template_id: "",
        template_version: "",
        trigger: "schedule",
        scheduled_for_ms: 1_000,
        triggered_by: "scheduler",
        attempt: 1,
        state: "finished",
        reason: "",
        infra: "ok",
        outcome: "ok",
        summary: "",
        target_agent_uid: "",
        block_id: "",
        turn_id: null,
        account_id: "",
        provider: "",
        started_at_ms: 1_000,
        ended_at_ms: 61_000,
        cost_usd: 0,
        cost_known: false,
        cost_partial: false,
        output_tokens: 0,
        steps: 0,
        links: {},
        skip_count: 0,
        last_skip_at_ms: null,
        reporter: "",
        recorded_by: "",
        failed: false,
        ...over,
    };
}

export function mkJob(over: Partial<JobView> = {}): JobView {
    return {
        id: "j1",
        source: "cron",
        name: "Nightly build",
        owner: "Builder",
        owner_uid: "uid-b",
        owner_block_id: null,
        target: "Builder",
        expression: "0 6 * * *",
        next_run_ms: NOW + 3 * 3_600_000,
        state: "active",
        catch_up: "skip",
        fire_count: 3,
        max_fires: null,
        missed_count: 0,
        recent: [],
        failing: false,
        cost_today_usd: 0,
        cost_week_usd: 0,
        cost_known: false,
        actions: true,
        ...over,
    };
}
