// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's context reading: how much of the model's context window the
 * conversation fills, with where each number came from.
 *
 * One definition of "context used" for every path that produces one:
 *
 *   - live:       the main agent's latest API call, from its `message_start`
 *                 (or `assistant`) usage — fresh + cache creation + cache read;
 *   - history:    the same figure for the last such call in the restored
 *                 transcript, when the pane opens.
 *
 * After a compaction there is no reading until the next call: the boundary's
 * `post_tokens` counts only the summary messages, not the system prompt and
 * tools every call carries (CLI 2.1.288: post_tokens 1,417, next call 39,490).
 *
 * A `result` frame's `usage` is NEVER a context size: it sums every API call of
 * the turn, so a 60-call turn at 300K reads as 18M. Seeding the meter from it
 * is how a freshly opened pane came to show "17m / 200k"
 * (docs/reports/REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md §2).
 *
 * The window, per model (`resolveContextWindow`):
 *
 *   - reported: Claude Code's own `result.modelUsage[<model id>].contextWindow`;
 *   - learned:  proven by observation — the API accepted a prompt larger than
 *               the window we had, so the window is at least the next known
 *               tier. Outranks a smaller reported window, which it refutes;
 *   - model:    the model-name table in context-window.ts, until either of the
 *               above exists.
 *
 * Never a per-provider constant: an unknown window is shown as unknown. Both
 * maps are kept per model id for the pane's lifetime — a window is a fact about
 * a model, not about one conversation.
 *
 * And one rule at the display: a reading larger than its window (or, with no
 * window, larger than any known window) is not shown. Every consumer — the
 * composer strip, the session popover, the Swarm row, the meta mirror — reads
 * through `plausibleReading`, so a wrong number degrades to a missing one.
 */

import { contextWindowForModel, MAX_KNOWN_CONTEXT_WINDOW, nextTierAbove } from "./context-window";

/** Which path produced a reading. */
export type ContextSource = "live" | "history";

/** Where a reading's window came from. */
export type ContextWindowSource =
    /** Claude Code's own `modelUsage[model].contextWindow`. */
    | "reported"
    /** The model-name table (context-window.ts). */
    | "model"
    /** Proven by an observed prompt larger than the window we had: the next
     *  known tier above it. */
    | "learned";

export interface ContextReading {
    /** Tokens in context: the main agent's last call's whole prompt. */
    tokens: number;
    /** Resolved model id the tokens were measured on (e.g. `claude-sonnet-5-5`);
     *  null when no model has been seen. */
    model: string | null;
    /** The model's context window; null when unknown. */
    window: number | null;
    windowSource: ContextWindowSource | null;
    source: ContextSource;
    /** When the reading was taken (unix ms); null when unknown (a replayed line
     *  with no receive stamp). */
    at: number | null;
    /** The pane's model setting changed to this since the reading was taken
     *  (`ContextModelSwitched`): the conversation carries over, but the window
     *  is the new model's and unknown until its first reply. Null otherwise. */
    switchedTo: string | null;
}

/** Context windows keyed by model id. */
export type ContextWindowMap = Readonly<Record<string, number>>;
/** Context windows Claude Code reported, keyed by model id. */
export type ReportedContextWindows = ContextWindowMap;

/** What the pane knows about context windows, per model. */
export interface KnownContextWindows {
    /** From `result.modelUsage` (`reportedContextWindowsFromResult`). */
    reported: ContextWindowMap;
    /** Proven by observed prompts (`learnedWindowsAfter`). */
    learned: ContextWindowMap;
}

/** A window outside this range is not a context window. */
const MIN_WINDOW = 1_000;
const MAX_WINDOW = 100_000_000;

function isPositiveInt(n: unknown): n is number {
    return typeof n === "number" && Number.isFinite(n) && Number.isInteger(n) && n > 0;
}

/** Claude Code's spelling of a model's 1M-context variant: `claude-sonnet-4-6[1m]`. */
const ONE_M_SUFFIX = /\[1m\]$/i;

/**
 * The windows a `result` frame reports: `modelUsage[<model>].contextWindow`, one
 * per model the session used (subagents' models included, and models of earlier
 * runs of a resumed session — the map is cumulative; each entry is a true fact
 * about that model).
 *
 * Each window is recorded under its key, under the entry's `canonicalModel`,
 * and under the key without a `[1m]` suffix: a reading is measured on the API's
 * `message.model`, the canonical id with no suffix, while the key is the raw
 * model string (CLI 2.1.288: key `claude-haiku-4-5-20251001`, canonical
 * `claude-haiku-4-5`). When two entries land on one id — a `[1m]` main agent
 * and a plain subagent of the same model — the larger window wins: the smaller
 * one would show a 1M conversation against 200K, while a too-large window is
 * the safe error (and a 1M window can't be refuted by any accepted prompt
 * either way). Returns null when the frame reports none.
 */
export function reportedContextWindowsFromResult(frame: unknown): Record<string, number> | null {
    if (!frame || typeof frame !== "object") return null;
    const f = frame as { type?: unknown; modelUsage?: unknown };
    if (f.type !== "result") return null;
    const usage = f.modelUsage;
    if (!usage || typeof usage !== "object") return null;
    const out: Record<string, number> = {};
    const put = (id: unknown, w: number) => {
        if (typeof id !== "string" || !id) return;
        if (!(id in out) || out[id] < w) out[id] = w;
    };
    for (const [model, entry] of Object.entries(usage as Record<string, unknown>)) {
        if (!model || !entry || typeof entry !== "object") continue;
        const e = entry as { contextWindow?: unknown; canonicalModel?: unknown };
        if (!isPositiveInt(e.contextWindow) || e.contextWindow < MIN_WINDOW || e.contextWindow > MAX_WINDOW) continue;
        put(model, e.contextWindow);
        put(e.canonicalModel, e.contextWindow);
        put(model.replace(ONE_M_SUFFIX, ""), e.contextWindow);
    }
    return Object.keys(out).length > 0 ? out : null;
}

/** `base` with `newer`'s entries on top; `base` itself when nothing changes. */
export function mergeContextWindows(base: ContextWindowMap, newer: ContextWindowMap | null | undefined): ContextWindowMap {
    if (!newer) return base;
    for (const [model, w] of Object.entries(newer)) {
        if (base[model] !== w) return { ...base, ...newer };
    }
    return base;
}

/** The window `map` holds for `model`: exact id, then case-insensitive, then
 *  the same id without a `[1m]` suffix. */
export function windowFor(map: ContextWindowMap, model: string | null): number | undefined {
    if (!model) return undefined;
    const lookup = (id: string): number | undefined => {
        const exact = map[id];
        if (exact != null) return exact;
        const lower = id.toLowerCase();
        for (const [key, w] of Object.entries(map)) {
            if (key.toLowerCase() === lower) return w;
        }
        return undefined;
    };
    const direct = lookup(model);
    if (direct != null) return direct;
    const bare = model.replace(ONE_M_SUFFIX, "");
    return bare !== model ? lookup(bare) : undefined;
}

/** The window Claude Code reported for `model`. */
export function reportedWindowFor(reported: ContextWindowMap, model: string | null): number | undefined {
    return windowFor(reported, model);
}

/**
 * The window for a reading of `tokens` on `model`: reported, unless a larger
 * window was learned for the model; else the learned one; else the table.
 * Then, if the prompt is larger than that, the next known tier above it — the
 * API accepted the prompt, so the window is at least that. A prompt above
 * every known tier proves nothing (it is an implausible reading, and
 * `implausibleReason` refuses it). The tiers are Claude's, so only a model
 * the table recognises learns; any other model has the window reported for
 * it, or none.
 */
export function resolveContextWindow(
    tokens: number,
    model: string | null,
    known: KnownContextWindows,
): { window: number | null; windowSource: ContextWindowSource | null } {
    const rep = windowFor(known.reported, model);
    const lrn = windowFor(known.learned, model);
    let window: number;
    let windowSource: ContextWindowSource;
    if (rep != null && (lrn == null || rep >= lrn)) {
        window = rep;
        windowSource = "reported";
    } else if (lrn != null) {
        window = lrn;
        windowSource = "learned";
    } else {
        const seed = contextWindowForModel(model);
        if (seed == null) return { window: null, windowSource: null };
        window = seed;
        windowSource = "model";
    }
    // The tiers are Claude's: a prompt proves the next one only for a model
    // the table knows. Any other model's larger prompt stays implausible.
    if (isPositiveInt(tokens) && tokens > window && contextWindowForModel(model) != null) {
        const proven = nextTierAbove(tokens);
        if (proven != null) return { window: proven, windowSource: "learned" };
    }
    return { window, windowSource };
}

/** A reading of `tokens` taken on `model` by `source`, with its window resolved. */
export function makeContextReading(
    input: { tokens: number; model: string | null; source: ContextSource; at: number | null },
    known: KnownContextWindows,
): ContextReading {
    const { window, windowSource } = resolveContextWindow(input.tokens, input.model, known);
    return { tokens: input.tokens, model: input.model, window, windowSource, source: input.source, at: input.at, switchedTo: null };
}

/** `reading` with its window re-resolved against `known`; the same object when
 *  that changes nothing. */
export function rewindowContextReading(reading: ContextReading | null, known: KnownContextWindows): ContextReading | null {
    // After a model switch the window is the new model's, which no report
    // about the old one can tell.
    if (!reading || reading.switchedTo != null) return reading;
    const { window, windowSource } = resolveContextWindow(reading.tokens, reading.model, known);
    if (window === reading.window && windowSource === reading.windowSource) return reading;
    return { ...reading, window, windowSource };
}

/** `learned` with the window `reading` proved recorded for its model; `learned`
 *  itself when the reading proves nothing new. */
export function learnedWindowsAfter(learned: ContextWindowMap, reading: ContextReading | null): ContextWindowMap {
    if (!reading || reading.windowSource !== "learned" || !reading.model || reading.window == null) return learned;
    const had = windowFor(learned, reading.model);
    if (had != null && had >= reading.window) return learned;
    return { ...learned, [reading.model]: reading.window };
}

/**
 * The reported window `reading` refutes, if any: Claude Code reported a window
 * for the model, and an accepted prompt was larger. Worth a log line — the
 * report is the authority everywhere else, so this is a disagreement to look at.
 */
export function refutedReportedWindow(reading: ContextReading, reported: ContextWindowMap): number | null {
    if (reading.windowSource !== "learned") return null;
    const rep = windowFor(reported, reading.model);
    return rep != null && reading.window != null && reading.window > rep ? rep : null;
}

/**
 * Why a reading can't be shown, or null when it can: a non-positive or
 * non-integer count, more tokens than its window, or — with no window — more
 * than any known window.
 */
export function implausibleReason(reading: ContextReading): string | null {
    if (!isPositiveInt(reading.tokens)) return "tokens is not a positive integer";
    if (reading.window != null) {
        if (!isPositiveInt(reading.window)) return "window is not a positive integer";
        if (reading.tokens > reading.window) return "tokens exceed the window";
        return null;
    }
    if (reading.tokens > MAX_KNOWN_CONTEXT_WINDOW) return "tokens exceed every known window";
    return null;
}

/** The reading when it is fit to show, otherwise null. Every display of the
 *  context meter goes through this. */
export function plausibleReading(reading: ContextReading | null | undefined): ContextReading | null {
    if (!reading) return null;
    return implausibleReason(reading) == null ? reading : null;
}

/**
 * A reading read back from block meta (`agent:context`), validated field by
 * field; null for anything else, including the legacy bare number the pane
 * used to write to `term:ctx-tokens`.
 */
export function contextReadingFromMeta(value: unknown): ContextReading | null {
    if (!value || typeof value !== "object") return null;
    const v = value as Record<string, unknown>;
    if (!isPositiveInt(v.tokens)) return null;
    const sources: ContextSource[] = ["live", "history"];
    const windowSources: ContextWindowSource[] = ["reported", "model", "learned"];
    const source = sources.find((s) => s === v.source);
    if (!source) return null;
    const window = isPositiveInt(v.window) ? v.window : null;
    const windowSource = window != null ? (windowSources.find((s) => s === v.windowSource) ?? null) : null;
    return plausibleReading({
        tokens: v.tokens,
        model: typeof v.model === "string" && v.model ? v.model : null,
        window,
        windowSource,
        source,
        at: isPositiveInt(v.at) ? v.at : null,
        switchedTo: typeof v.switchedTo === "string" && v.switchedTo ? v.switchedTo : null,
    });
}

/**
 * Where the meter's numbers come from, in a sentence or two for its tooltip:
 * the reading's origin when it isn't the last live call, and the window's
 * source when Claude Code hasn't reported it.
 */
export function contextReadingNote(reading: ContextReading): string {
    const lines: string[] = [];
    if (reading.source === "history") {
        lines.push("From the conversation's last reply before this pane opened; updates with the next reply.");
    }
    const model = reading.model ?? "this model";
    if (reading.switchedTo != null) {
        lines.push(`Model changed to ${reading.switchedTo}; its window shows with its first reply.`);
        return lines.join("\n");
    }
    switch (reading.windowSource) {
        case "reported":
            lines.push(`Window reported by the CLI for ${model}.`);
            break;
        case "model":
            lines.push(`Window assumed from the model (${model}) until the CLI reports it.`);
            break;
        case "learned":
            lines.push(`Window inferred for ${model}: it accepted a prompt larger than the window it was thought to have.`);
            break;
        default:
            lines.push(`Window not known for ${model} yet.`);
    }
    return lines.join("\n");
}

/** Two readings that would display the same. */
export function sameContextReading(a: ContextReading | null, b: ContextReading | null): boolean {
    if (a === b) return true;
    if (!a || !b) return false;
    return (
        a.tokens === b.tokens &&
        a.model === b.model &&
        a.window === b.window &&
        a.windowSource === b.windowSource &&
        a.source === b.source &&
        a.at === b.at &&
        a.switchedTo === b.switchedTo
    );
}
