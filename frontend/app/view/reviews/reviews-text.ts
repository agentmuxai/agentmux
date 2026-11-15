// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The words of the Reviews pane, from the dictionary (`reviews.*` in i18n/locales/en.json). srv sends codes (a
 * waits-on, a reason, a state); this turns each into text. A code this build does not know is shown as its own words,
 * so a newer srv never shows a bare key.
 */

import { t } from "@/app/i18n/i18n";
import type { ReviewsVerdict } from "@/types/rpc/ReviewsVerdict";
import type { WaitsOn } from "@/types/rpc/WaitsOn";
import type { ReviewsGroup } from "./reviews-model";

const words = (code: string): string => code.replace(/_/g, " ");

/** What a pull request waits on, in a sentence ("Waits on its author"). */
export function waitsLabel(waits: WaitsOn): string {
    switch (waits) {
        case "person":
            return t("reviews.waits.person.label");
        case "author":
            return t("reviews.waits.author.label");
        case "reviewer":
            return t("reviews.waits.reviewer.label");
        case "ci":
            return t("reviews.waits.ci.label");
        case "queue":
            return t("reviews.waits.queue.label");
        case "none":
            return t("reviews.waits.none.label");
        default:
            return t("reviews.waits.unknown.label");
    }
}

/** What a pull request waits on, in one word, for a chip. */
export function waitsShort(waits: WaitsOn): string {
    switch (waits) {
        case "person":
            return t("reviews.waits.person.short");
        case "author":
            return t("reviews.waits.author.short");
        case "reviewer":
            return t("reviews.waits.reviewer.short");
        case "ci":
            return t("reviews.waits.ci.short");
        case "queue":
            return t("reviews.waits.queue.short");
        case "none":
            return t("reviews.waits.none.short");
        default:
            return t("reviews.waits.unknown.short");
    }
}

/** Why a pull request waits on what it does: srv's `why` code in words. */
export function whyText(why: string): string {
    switch (why) {
        case "changes_requested":
            return t("reviews.why.changesRequested");
        case "checks_failing":
            return t("reviews.why.checksFailing");
        case "checks_running":
            return t("reviews.why.checksRunning");
        case "no_review":
            return t("reviews.why.noReview");
        case "review_stale":
            return t("reviews.why.reviewStale");
        case "awaiting_accept":
            return t("reviews.why.awaitingAccept");
        case "ready_to_merge":
            return t("reviews.why.readyToMerge");
        case "merged":
            return t("reviews.why.merged");
        case "closed":
            return t("reviews.why.closed");
        case "task_finished":
            return t("reviews.why.taskFinished");
        case "queued":
            return t("reviews.why.queued");
        case "unknown":
            return t("reviews.why.unknown");
        default:
            return words(why);
    }
}

/** A merge state: `open`, `merged`, `closed`, or another well-formed name as its words. */
export function mergeText(merge: string): string {
    switch (merge) {
        case "open":
            return t("reviews.merge.open");
        case "merged":
            return t("reviews.merge.merged");
        case "closed":
            return t("reviews.merge.closed");
        default:
            return words(merge);
    }
}

/** The state of a pull request's checks. */
export function checksText(state: string): string {
    switch (state) {
        case "passing":
            return t("reviews.checks.passing");
        case "failing":
            return t("reviews.checks.failing");
        case "pending":
            return t("reviews.checks.pending");
        default:
            return words(state);
    }
}

/** A reviewer's verdict, with "older commit" when the head moved after it. */
export function verdictText(v: ReviewsVerdict): string {
    const state =
        v.state === "approved"
            ? t("reviews.verdict.approved")
            : v.state === "changes_requested"
              ? t("reviews.verdict.changesRequested")
              : words(v.state);
    return v.stale ? t("reviews.verdict.stale", { state }) : state;
}

/** Who gave a verdict. */
export function byText(by: ReviewsVerdict["by"]): string {
    switch (by) {
        case "bot":
            return t("reviews.by.bot");
        case "codex":
            return t("reviews.by.codex");
        default:
            return t("reviews.by.person");
    }
}

/** The heading of a group: the two fixed ones are words, a task's is its own title. */
export function groupTitle(group: ReviewsGroup): string {
    if (group.kind === "person") return t("reviews.group.person");
    if (group.kind === "none") return t("reviews.group.none");
    return group.title;
}

/** "now", "3m", "5h", "3d": how long ago a time was. */
export function ageText(atMs: number, nowMs: number): string {
    const s = Math.max(0, Math.round((nowMs - atMs) / 1000));
    if (s < 60) return t("reviews.age.now");
    const m = Math.floor(s / 60);
    if (m < 60) return t("reviews.age.minutes", { n: m });
    const h = Math.floor(m / 60);
    if (h < 48) return t("reviews.age.hours", { n: h });
    return t("reviews.age.days", { n: Math.floor(h / 24) });
}

/** The waits-on switch: the value each segment filters to (null for all) and its words. */
export function waitsOptions(): { value: WaitsOn | null; label: string }[] {
    return [
        { value: null, label: t("reviews.filter.all") },
        { value: "person", label: t("reviews.filter.person") },
        { value: "author", label: t("reviews.filter.author") },
        { value: "reviewer", label: t("reviews.filter.reviewer") },
        { value: "ci", label: t("reviews.filter.checks") },
    ];
}
