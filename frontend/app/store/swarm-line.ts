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
 * 0. `declared` / `task`: the line the agent set itself with `Presence status`, or the title of the
 *    task it claimed (written by srv, see work_facts/live_phrase.rs). The agent's own model knows what
 *    it is doing, so this comes first. It carries its age, and is muted once it is old.
 * 1. `generated`: the title the model wrote (or the CLI's own OSC title).
 * 2. `restored`: the last good title of the session that just ended.
 * 3. `heuristic`: a deterministic one-liner from the user's latest message.
 * 4. `status`: a plain phrase from what the agent is doing.
 *
 * One deviation from the spec's order, on purpose: an agent that is waiting on
 * the user says so ahead of every title, as the agent pane's working row does.
 * That is the one state someone has to act on, and any title would bury it.
 * Which status wins comes from the shared ranking (agent-status.ts), which
 * honours the flag only while the agent has a turn in flight: a flag left
 * behind by a crash, or by a question answered while the pane was not mounted
 * to clear it, cannot outrank the other lines once the agent is idle.
 *
 * docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md section 5.4.
 */

import { readSwarmSummary } from "./activitySummary";
import { META_AWAITING_BROWSER, META_AWAITING_CONFIRM, rankAgentStatus, type AgentStatus } from "./agent-status";
import { isUsableTitle } from "./ambient-title";
import { META_TITLE_AT } from "./meta-keys";
import { supervisorPauseReason } from "./supervisor";

export type SwarmLineSource = "declared" | "task" | "generated" | "restored" | "heuristic" | "status";

export interface SwarmLine {
    text: string;
    source: SwarmLineSource;
    /** `declared` and `task` lines: when it was set (epoch ms), when it says. */
    setAt?: number;
    /** `declared` and `task` lines: how long ago that was, at the time the line was resolved. */
    ageMs?: number;
    /** `declared` and `task` lines: older than [`LIVE_PHRASE_STALE_MS`], or of no known age. */
    stale?: boolean;
}

/** Block meta keys this module reads. Written by the agent pane's hooks. */
export const META_RESTORED = "term:restored_summary";
export const META_LAST_PROMPT = "term:last_prompt";
export const META_AWAITING_USER = "term:awaiting_user";
/** The line the agent set (or the task title srv wrote on a claim), when it was set, and by whom
 *  (`agent` or `task`). Written by srv (crates/srv/src/backend/work_facts/live_phrase.rs). */
export const META_LIVE_PHRASE = "term:live_phrase";
export const META_LIVE_PHRASE_AT = "term:live_phrase_at";
export const META_LIVE_PHRASE_SRC = "term:live_phrase_src";

/** A line older than this reads as old: muted, with its age. */
export const LIVE_PHRASE_STALE_MS = 30 * 60_000;

/** How much of a message is kept as `term:last_prompt`. Only a one-liner is ever
 *  shown, so there is no reason to keep a long message in block meta. */
export const LAST_PROMPT_MAX_CHARS = 240;

export const STATUS_WORKING = "Working";
export const STATUS_THINKING = "Thinking";
export const STATUS_WAITING = "Waiting for you";
/** The line while only a Browser request of the agent waits: the answer is given in that pane, not the agent's. */
export const STATUS_WAITING_BROWSER = "Waiting for you in a browser pane";
/** The line's lead when the Supervisor stopped the agent: "Needs you: <why>". */
export const STATUS_NEEDS_YOU = "Needs you";
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
 *  message, a swarm broadcast wrapper, or a Supervisor continuation. */
function isInjected(message: string): boolean {
    return /^\s*\[(JEKT|BROADCAST|SUPERVISOR):/i.test(message);
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
    /** The time the line's age is measured against (epoch ms); now when absent. */
    now?: number;
}

/** "just now", "5m ago", "2h ago", "3d ago". */
export function formatLineAge(ms: number): string {
    const minutes = Math.floor(Math.max(0, ms) / 60_000);
    if (minutes < 1) return "just now";
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    return `${Math.floor(hours / 24)}d ago`;
}

/** The line the agent set, or the task title srv wrote when it claimed one. */
function readLivePhrase(meta: Record<string, unknown> | undefined, now: number): SwarmLine | null {
    const text = meta?.[META_LIVE_PHRASE];
    // No usability gate: srv cleaned the line, and "Blocked: I cannot reach the API" is the line to show.
    if (typeof text !== "string" || !text.trim()) return null;
    const at = meta?.[META_LIVE_PHRASE_AT];
    const hasTime = typeof at === "number" && at > 0;
    const ageMs = hasTime ? Math.max(0, now - at) : undefined;
    const source: SwarmLineSource = meta?.[META_LIVE_PHRASE_SRC] === "task" ? "task" : "declared";
    return {
        text: text.trim(),
        source,
        ...(hasTime ? { setAt: at } : {}),
        ageMs,
        stale: ageMs === undefined || ageMs > LIVE_PHRASE_STALE_MS,
    };
}

/** An agent's own status line wins over the generated title in the pane while it is this fresh. */
export const FRESH_STATUS_MS = 60 * 60_000;

/**
 * The line an agent pane shows for what the agent is doing (the working row, a tab's tooltip):
 * the agent's own status line (`Presence status`) while it is fresh and newer than the generated
 * title, else the generated title. The agent knows what it is doing; the title is a guess made
 * from its messages, and it can lag behind a change of topic. A task title srv wrote at a claim
 * is not the agent's word, so it does not count here, and neither does an old line.
 */
export function readPaneSummary(meta: Record<string, unknown> | undefined, now: number = Date.now()): string | null {
    const title = readSwarmSummary(meta);
    const phrase = readLivePhrase(meta, now);
    if (!phrase || phrase.source !== "declared" || phrase.setAt === undefined) return title;
    if ((phrase.ageMs ?? Infinity) > FRESH_STATUS_MS) return title;
    const titleAt = meta?.[META_TITLE_AT];
    const titleIsNewer = title !== null && typeof titleAt === "number" && titleAt >= phrase.setAt;
    return titleIsNewer ? title : phrase.text;
}

/** The ranked status of a Swarm agent, from what its row knows: whether a
 *  turn is in flight, the pending-question flag (`term:awaiting_user`),
 *  whether the Swarm Supervisor stopped it for the user, and whether a Browser
 *  request of its is open between turns (`term:awaiting_browser`). */
function swarmAgentStatus(meta: Record<string, unknown> | undefined, status: "running" | "idle"): AgentStatus {
    const live = status === "running";
    return rankAgentStatus({
        live,
        needsYou: meta?.[META_AWAITING_USER] === true,
        confirmPending: meta?.[META_AWAITING_CONFIRM] === true,
        browserRequest: meta?.[META_AWAITING_BROWSER] === true,
        pausedForYou: supervisorPauseReason(meta, live) !== null,
    });
}

/**
 * Whether the agent is waiting on the user: a question is pending and a turn
 * is in flight, the Supervisor stopped it for the user, or it ended its turn
 * with a Browser request still open. The row's line and
 * the status chip's "question" state both use this.
 */
export function isAwaitingUser(meta: Record<string, unknown> | undefined, status: "running" | "idle"): boolean {
    return swarmAgentStatus(meta, status).kind === "needs-you";
}

/** Whether the agent ended its turn with a Browser request still open: the person is asked in that Browser
 *  pane, with no turn in flight (`term:awaiting_browser`, kept by srv). */
export function awaitsBrowserAnswer(meta: Record<string, unknown> | undefined, status: "running" | "idle"): boolean {
    return status !== "running" && meta?.[META_AWAITING_BROWSER] === true;
}

/** The line for one agent row. Always returns something. */
export function resolveSwarmLine(input: SwarmLineInput): SwarmLine {
    const { meta, currentTool, contextTokens } = input;
    const status = swarmAgentStatus(meta, input.status);

    if (status.kind === "needs-you") {
        const paused = supervisorPauseReason(meta, input.status === "running");
        if (paused) return { text: `${STATUS_NEEDS_YOU}: ${paused}`, source: "status" };
        // Only a Browser request asks: the line says where the answer is given.
        const inBrowser = awaitsBrowserAnswer(meta, input.status) && meta?.[META_AWAITING_CONFIRM] !== true;
        return { text: inBrowser ? STATUS_WAITING_BROWSER : STATUS_WAITING, source: "status" };
    }

    const declared = readLivePhrase(meta, input.now ?? Date.now());
    if (declared) return declared;

    const generated = readSwarmSummary(meta);
    if (generated) return { text: generated, source: "generated" };

    const restored = meta?.[META_RESTORED];
    if (typeof restored === "string" && isUsableTitle(restored)) {
        return { text: restored.trim(), source: "restored" };
    }

    const lastPrompt = meta?.[META_LAST_PROMPT];
    const heuristic = typeof lastPrompt === "string" ? heuristicTitle(lastPrompt) : null;
    if (heuristic) return { text: heuristic, source: "heuristic" };

    // The Swarm knows no held reason, so a live agent is working; with no
    // tool in flight it is the model's own turn.
    if (status.kind === "working") {
        return { text: currentTool ? STATUS_WORKING : STATUS_THINKING, source: "status" };
    }
    return {
        // Any transcript counts as history: a context reading exists only for
        // providers that report one (Claude), and only once the pane mirrored it.
        text:
            (contextTokens ?? 0) > 0 || ((meta?.["session:line_count"] as number | undefined) ?? 0) > 0
                ? STATUS_SUMMARIZING
                : STATUS_NO_ACTIVITY,
        source: "status",
    };
}

/** `line` as it reads at `now`: the age and staleness of the agent's own line or task, counted from when
 *  it was set. The resolved line is memoized without a clock, so a row that ticks asks this instead. */
export function lineAt(line: SwarmLine, now: number): SwarmLine {
    if (line.setAt === undefined) return line;
    const ageMs = Math.max(0, now - line.setAt);
    return { ...line, ageMs, stale: ageMs > LIVE_PHRASE_STALE_MS };
}

/** Whether a row shows its line as a fallback (muted): everything but the generated title and the
 *  agent's own line or task while it is fresh. Pass `now` to count the age from the row's clock. */
export function isMutedLine(resolved: SwarmLine, now?: number): boolean {
    const line = now === undefined ? resolved : lineAt(resolved, now);
    switch (line.source) {
        case "generated":
            return false;
        case "declared":
        case "task":
            return line.stale === true;
        default:
            return true;
    }
}

/** The tooltip for a row's line, or `undefined` for the agent's own title. */
export function swarmLineTooltip(resolved: SwarmLine, now?: number): string | undefined {
    const line = now === undefined ? resolved : lineAt(resolved, now);
    switch (line.source) {
        case "generated":
            return undefined;
        case "declared":
            if (!line.stale) return undefined;
            return line.ageMs === undefined ? "Set by the agent, a while ago" : `Set by the agent ${formatLineAge(line.ageMs)}`;
        case "task":
            if (line.stale) return line.ageMs === undefined ? "The task it claimed, a while ago" : `The task it claimed ${formatLineAge(line.ageMs)}`;
            return "The task it claimed; the agent has not said more yet";
        case "restored":
            return RESTORED_TOOLTIP;
        default:
            return FALLBACK_TOOLTIP;
    }
}
