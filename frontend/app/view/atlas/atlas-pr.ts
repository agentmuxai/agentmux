// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The review bot and CI on the Atlas map (docs/specs/SPEC_ATLAS_AGENT_MAP_2026_11_13.md §6.4): two nodes that are
 * not agents, and a line from each to the agent whose pull request it reviewed or checked. srv says so with the
 * `review-flow`, `checks-flow` and `merge-flow` events (docs/specs/SPEC_REVIEW_AND_CHECKS_FLOW_EVENTS_2026_11_14.md):
 * numbers, states and times only, never a title, a branch or a login. This file turns an event into a line
 * (atlas-flow.ts keeps and times the lines) and the lines on show into the two nodes. It is pure.
 */

import type { ChecksFlowEvent } from "@/types/rpc/ChecksFlowEvent";
import type { PrFlowBy } from "@/types/rpc/PrFlowBy";
import type { PrRef } from "@/types/rpc/PrRef";
import type { ReviewFlowEvent } from "@/types/rpc/ReviewFlowEvent";
import type { AtlasFlow, FlowTone } from "./atlas-flow";
import type { AtlasService, AtlasServiceLine } from "./atlas-graph";

/** The keys of the two nodes. */
export const REVIEW_NODE = "service:review";
export const CHECKS_NODE = "service:checks";

/** A pull request, as the key its lines share. Null for one srv did not number. */
export function prKey(pr: PrRef | undefined): string | null {
    return pr && pr.number > 0 ? `${pr.repo_id}/${pr.number}` : null;
}

type PrTone = Extract<FlowTone, "passed" | "failed" | "neutral">;

/** A state this build does not name is neutral: srv keeps any well-formed one, so a new one needs no release here. */
const REVIEW_TONES: Record<string, PrTone> = { approved: "passed", changes_requested: "failed" };
const CHECKS_TONES: Record<string, PrTone> = { passing: "passed", failing: "failed" };

/** Who reviewed, after the pull request's number: the bot's own reviews say nothing. */
const BY_WORDS: Record<PrFlowBy, string> = { bot: "", codex: " Codex", person: " person" };

function prFlow(
    kind: "review" | "checks",
    node: string,
    event: ReviewFlowEvent | ChecksFlowEvent,
    tone: PrTone,
    by = ""
): AtlasFlow | null {
    const pr = prKey(event.pr);
    if (!pr || !event.agent || !event.id) return null;
    return {
        id: `${kind}:${event.id}`,
        kind,
        from: { node },
        to: { agent: event.agent, local: true },
        tone,
        label: `#${event.pr.number}${by}`,
        atMs: event.at_ms,
        leaving: false,
        pr,
        state: event.state,
    };
}

/** The line for a review: from the review bot's node to the agent whose pull request it is. */
export function reviewFlow(event: ReviewFlowEvent): AtlasFlow | null {
    return prFlow("review", REVIEW_NODE, event, REVIEW_TONES[event.state] ?? "neutral", BY_WORDS[event.by] ?? "");
}

/** The line for a pull request's checks: from CI's node to the agent whose pull request it is. */
export function checksFlow(event: ChecksFlowEvent): AtlasFlow | null {
    return prFlow("checks", CHECKS_NODE, event, CHECKS_TONES[event.state] ?? "neutral");
}

/** What this machine knows of the bot itself. */
export interface PrBot {
    /** When the newest review, checks or merge notice came; null for none since srv started. */
    lastNoticeAtMs: number | null;
    /** Notices from GitHub are not reaching this machine now (cloud messages are paused). */
    paused: boolean;
}

function serviceLines(flows: readonly AtlasFlow[], kind: "review" | "checks"): AtlasServiceLine[] {
    return flows
        .filter((f) => f.kind === kind && "agent" in f.to)
        .sort((a, b) => b.atMs - a.atMs)
        .map((f) => ({
            id: f.id,
            pr: f.label ?? "",
            state: (f.state ?? "").replace(/_/g, " "),
            tone: f.tone === "passed" || f.tone === "failed" ? f.tone : "neutral",
            agent: "agent" in f.to ? f.to.agent : "",
            atMs: f.atMs,
        }));
}

const count = (lines: readonly AtlasServiceLine[], tone: AtlasServiceLine["tone"]) => lines.filter((l) => l.tone === tone).length;

/**
 * The review bot's node and CI's, from the pull request lines on show. Neither is drawn until a notice has come:
 * a machine that gets none (no review bot, not signed in to the cloud) has a map of agents only. `time` writes a
 * moment as the clock time the rest of the app shows.
 */
export function prServices(flows: readonly AtlasFlow[], bot: PrBot, time: (ms: number) => string): AtlasService[] {
    const reviews = serviceLines(flows, "review");
    const checks = serviceLines(flows, "checks");
    if (bot.lastNoticeAtMs === null && reviews.length + checks.length === 0) return [];
    const wanting = count(reviews, "failed");
    const failing = count(checks, "failed");
    const passing = count(checks, "passed");
    const state = (failed: number): AtlasService["state"] => (bot.paused ? "paused" : failed > 0 ? "warn" : "ok");
    const paused = "Cloud messages paused";
    const last = bot.lastNoticeAtMs === null ? "No notices yet" : `Last notice ${time(bot.lastNoticeAtMs)}`;
    return [
        {
            key: REVIEW_NODE,
            name: "Review bot",
            icon: "robot",
            state: state(wanting),
            sub: bot.paused ? paused : wanting > 0 ? `${wanting} ${wanting === 1 ? "needs" : "need"} changes` : last,
            lines: reviews,
        },
        {
            key: CHECKS_NODE,
            name: "CI",
            icon: "circle-check",
            state: state(failing),
            sub: bot.paused ? paused : failing > 0 ? `${failing} failing` : passing > 0 ? `${passing} passing` : "No results on show",
            lines: checks,
        },
    ];
}
