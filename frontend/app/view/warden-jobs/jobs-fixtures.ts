// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Runs and jobs for the Jobs section's tests.

import type { CapDefaults } from "@/types/rpc/CapDefaults";
import type { JobRunRow } from "@/types/rpc/JobRunRow";
import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";
import type { JobsDryRunResult } from "@/types/rpc/JobsDryRunResult";
import type { JobView } from "@/types/rpc/JobView";

/** "Now" in the tests: ten days after the epoch. */
export const NOW = 10 * 86_400_000;

/** Settings' caps as srv sends them. */
export const DEFAULTS: CapDefaults = { run_usd: 5, job_day_usd: 20, account_day_usd: 50, messages_per_day: 24 };

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
        trigger_kind: "cron",
        tz: "UTC",
        every_secs: null,
        once_at_ms: null,
        next_run_ms: NOW + 3 * 3_600_000,
        state: "active",
        catch_up: "skip",
        fire_count: 3,
        max_fires: null,
        missed_count: 0,
        recent: [],
        failing: false,
        incident: null,
        cost_today_usd: 0,
        cost_week_usd: 0,
        cost_known: false,
        actions: true,
        over_cap: null,
        run_cap_usd: 5,
        day_cap_usd: 20,
        cost_not_reported: false,
        template: null,
        cloud: null,
        ...over,
    };
}

/** A job template as `jobs.templates.list` returns it: a weekly report with one required text
 *  parameter, a list, a choice and a credential. */
export function mkTemplate(over: Partial<JobTemplateInfo> = {}): JobTemplateInfo {
    return {
        id: "agentmux.weekly-report",
        source: "builtin",
        state: "approved",
        error: null,
        hash: "hash-1",
        folder: null,
        signature: { state: "unsigned", publisher: "agentmux", fingerprint: null, pinned: null },
        manifest: {
            templateVersion: 1,
            id: "agentmux.weekly-report",
            name: "Weekly report",
            version: "1.0.0",
            kind: "job",
            description: "Reads repositories since the last run and reports.",
            readsUntrusted: false,
            params: {
                type: "object",
                properties: {
                    repo: { type: "string", title: "Repository", description: "owner/repo" },
                    focus: { type: "string", title: "Focus", enum: ["security", "docs"], default: "security" },
                    paths: { type: "array", title: "Paths", items: { type: "string" } },
                    max_items: { type: "number", title: "Most items", default: 10 },
                    token: { type: "string", title: "Token", format: "credential" },
                },
                required: ["repo"],
            },
            trigger: { kind: "cron", cron: "0 6 * * 1" },
            action: { kind: "message" },
            policy: { overlap: "skip", catch_up: "latest", timeout_minutes: 60, caps: { run_usd: 3 } },
            outputs: ["report_file", "pull_request"],
            secrets: [{ name: "REPO_TOKEN", param: "token" }],
        },
        ...over,
    };
}

export function mkDryRun(over: Partial<JobsDryRunResult> = {}): JobsDryRunResult {
    return {
        ok: true,
        error: null,
        name: "Weekly report",
        action_kind: "message",
        target: "uid-b",
        brief: "Report on acme/site.",
        tz: "Europe/Berlin",
        fires: [
            { at_ms: NOW + 1_000, local: "Mon 2026-10-26 06:00 CET" },
            { at_ms: NOW + 2_000, local: "Mon 2026-11-02 06:00 CET" },
        ],
        caps: { run_usd: 3, day_usd: 20, account_day_usd: 50, messages_per_day: 24, clamped: false },
        outputs: ["report_file"],
        secrets: [{ name: "REPO_TOKEN", vault: "v-1" }],
        group: null,
        notes: [],
        ...over,
    };
}
