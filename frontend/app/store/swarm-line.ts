// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The line a Swarm row shows for an agent, which is never empty.
 *
 * A row used to show the agent's generated title and nothing at all when there
 * was none: a fresh session, a model call that failed or abstained, or the clear
 * that happens when a session ends. A blank row says nothing about whether the
 * agent is idle, stuck or new. This resolves the line from the first of four
 * sources that has something, and says which one it used so the view can show a
 * fallback as a fallback (muted, with a tooltip) and never as the agent's own
 * account of its work.
 *
 * 1. `generated`: the title the model wrote (or the CLI's own OSC title).
 * 2. `restored`: the last good title of the session that just ended.
 * 3. `heuristic`: a deterministic one-liner from the user's latest message.
 * 4. `status`: a plain phrase from what the agent is doing.
 *
 * One deviation from the spec's order, on purpose: an agent that is waiting on
 * the user says so ahead of `restored` and `heuristic`. That is the one state
 * someone has to act on, and an old goal would bury it.
 *
 * docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md section 5.4.
 */

import { readSwarmSummary } from "./activitySummary";
import { isUsableTitle } from "./ambient-title";

export type SwarmLineSource = "generated" | "restored" | "heuristic" | "status";

export interface SwarmLine {
    text: string;
    source: SwarmLineSource;
}

/** Block meta keys this module reads. Written by the agent pane's hooks. */
export const META_RESTORED = "term:restored_summary";
export const META_LAST_PROMPT = "term:last_prompt";
export const META_AWAITING_USER = "term:awaiting_user";

/** How much of a message is kept as `term:last_prompt`. Only a one-liner is ever
 *  shown, so there is no reason to keep a long message in block meta. */
export const LAST_PROMPT_MAX_CHARS = 240;

export const STATUS_WORKING = "Working";
export const STATUS_THINKING = "Thinking";
export const STATUS_WAITING = "Waiting for you";
export const STATUS_SUMMARIZING = "Summarizing work completed";
export const STATUS_NO_ACTIVITY = "No activity yet";

/** The tooltip that says a fallback is a fallback. */
export const FALLBACK_TOOLTIP = "Shown until a summary is ready";
export const RESTORED_TOOLTIP = "From the previous session, shown until a new summary is ready";

const MAX_HEURISTIC_WORDS = 8;

/** Messages that carry no goal, compared after lower-casing and stripping
 *  punctuation. "u there" and "continue" tell the agent nothing the swarm should
 *  repeat as what the agent is working on. */
const NUDGES = new Set([
    "continue", "go on", "go ahead", "keep going", "proceed", "next", "again", "retry", "try again",
    "yes", "yeah", "yep", "yup", "no", "nope", "ok", "okay", "k", "kk", "sure", "right", "fine",
    "thanks", "thank you", "thx", "ty", "great", "nice", "good", "cool", "perfect", "lgtm", "done",
    "u there", "you there", "are you there", "still there", "hello", "hi", "hey", "hello there",
    "status", "any update", "any updates", "update", "how is it going", "hows it going", "how's it going",
    "do it", "do that", "sounds good", "go for it", "use your recommendations", "your call",
]);

/** Leading words that frame a request without being part of it. Stripped one
 *  after another from the front, so "ok so can you please fix the login" becomes
 *  "fix the login". */
const FILLER_PREFIXES = [
    "hi", "hello", "hey", "ok", "okay", "so", "right", "alright", "well", "now", "also", "and", "then",
    "please", "pls", "can you", "could you", "would you", "will you", "can u", "could u",
    "i want you to", "i need you to", "i would like you to", "i'd like you to", "i want to", "i need to",
    "lets", "let's", "let us", "we need to", "we should", "go ahead and", "just",
];

/** Text that arrives as a user turn but is not a person talking: an agent-to-agent
 *  message or a swarm broadcast wrapper. */
function isInjected(message: string): boolean {
    return /^\s*\[(JEKT|BROADCAST):/i.test(message);
}

function normalizeForNudge(text: string): string {
    return text
        .toLowerCase()
        .replace(/[^a-z0-9' ]+/g, " ")
        .replace(/\s+/g, " ")
        .trim();
}

/**
 * A short, deterministic title from a user's message, or `null` when the message
 * has no goal in it (a nudge, an injected message, nothing left once filler is
 * stripped). No model is involved, so it can never invent anything: every word is
 * the user's.
 */
export function heuristicTitle(message: string | null | undefined): string | null {
    if (!message || isInjected(message)) return null;
    const firstLine = message.split(/\r?\n/).find((l) => l.trim().length > 0)?.trim() ?? "";
    if (!firstLine || NUDGES.has(normalizeForNudge(firstLine))) return null;

    // First sentence only, then peel filler words off the front.
    let text = firstLine.split(/(?<=[.!?])\s+/)[0].replace(/\s+/g, " ").trim();
    for (let changed = true; changed; ) {
        changed = false;
        for (const prefix of FILLER_PREFIXES) {
            const re = new RegExp(`^${prefix.replace(/'/g, "['’]")}(?=[\\s,.:;!-]|$)[\\s,.:;!-]*`, "i");
            if (re.test(text)) {
                text = text.replace(re, "");
                changed = true;
            }
        }
    }
    text = text.replace(/[\s,.:;!?-]+$/g, "").trim();
    const words = text.split(" ").filter(Boolean);
    if (words.length < 2 || NUDGES.has(normalizeForNudge(text))) return null;

    let cut = words.slice(0, MAX_HEURISTIC_WORDS).join(" ");
    if (words.length > MAX_HEURISTIC_WORDS) cut += "…";
    const title = cut.charAt(0).toUpperCase() + cut.slice(1);
    // The same predicate every stored title passes: a user message that is itself
    // about the absence of a title must not become one.
    return isUsableTitle(title.replace(/…$/, "")) ? title : null;
}

/** What the pane stores as `term:last_prompt`: the message, trimmed and capped,
 *  or `null` when it carries no goal (so a nudge never overwrites a real one). */
export function lastPromptToStore(message: string | null | undefined): string | null {
    if (heuristicTitle(message) === null) return null;
    return (message ?? "").trim().slice(0, LAST_PROMPT_MAX_CHARS);
}

export interface SwarmLineInput {
    meta: Record<string, unknown> | undefined;
    status: "running" | "idle";
    /** The tool in flight right now, or null between tools. */
    currentTool: string | null;
    /** Context tokens used so far, as a sign that there is conversation history. */
    contextTokens: number | null;
}

/** The line for one agent row. Always returns something. */
export function resolveSwarmLine(input: SwarmLineInput): SwarmLine {
    const { meta, status, currentTool, contextTokens } = input;

    const generated = readSwarmSummary(meta);
    if (generated) return { text: generated, source: "generated" };

    if (meta?.[META_AWAITING_USER] === true) return { text: STATUS_WAITING, source: "status" };

    const restored = meta?.[META_RESTORED];
    if (typeof restored === "string" && isUsableTitle(restored)) {
        return { text: restored.trim(), source: "restored" };
    }

    const lastPrompt = meta?.[META_LAST_PROMPT];
    const heuristic = typeof lastPrompt === "string" ? heuristicTitle(lastPrompt) : null;
    if (heuristic) return { text: heuristic, source: "heuristic" };

    if (status === "running") {
        return { text: currentTool ? STATUS_WORKING : STATUS_THINKING, source: "status" };
    }
    return {
        text: (contextTokens ?? 0) > 0 ? STATUS_SUMMARIZING : STATUS_NO_ACTIVITY,
        source: "status",
    };
}

/** The tooltip for a row's line, or `undefined` for the agent's own title. */
export function swarmLineTooltip(line: SwarmLine): string | undefined {
    switch (line.source) {
        case "generated":
            return undefined;
        case "restored":
            return RESTORED_TOOLTIP;
        default:
            return FALLBACK_TOOLTIP;
    }
}
