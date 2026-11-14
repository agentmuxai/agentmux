// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Everything an agent is waiting on the user for, in this window
 * (docs/reports/REPORT_AGENT_ATTENTION_CTA_CONTRAST_AND_TONE_2026_10_10.md §2).
 *
 * Each source of a call to action says when it starts and ends waiting:
 * - an AskUserQuestion (`useAgentQuestions`);
 * - a tool permission (`useAgentDecisions`);
 * - and, through srv's `userattention` event, a browser hand-off or
 *   approval, an SSH consent and a widget install.
 *
 * A pane is waiting while any of its sources is. On its first source the
 * pane's `waiting-for-input` event fires, which loops the waiting tone (the
 * sound service), shows the OS notification and badges the taskbar (the OS
 * bridge); on its last, `waiting-ended`.
 *
 * Requests that belong to no pane are kept under an `app:` key. They play the
 * tone only while this window isn't in front, and send no OS notification.
 */

import { createEffect, createRoot, createSignal } from "solid-js";
import { fireEvent as firePaneEvent } from "@/app/store/agent-pane-state-store";
import { MOS } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { windowId } from "@/app/store/window-identity";

import type { UserAttentionEvent } from "@/types/rpc/UserAttentionEvent";
import { BROWSER_WHAT, browserWaitsShown } from "./browser-waits";
import { APP_KEY_PREFIX } from "./waiting-keys";


interface WaitEntry {
    /** What it wants, for the notification and the tab. */
    text?: string;
    /** What it wants in a few words, led by a verb ("asks to approve a Bash call"), for the toast and the list. */
    what: string;
}

const waiting = new Map<string, Map<string, WaitEntry>>();
/** `blockId` → when its first source began waiting (unix ms). */
const since = new Map<string, number>();
// Bumped on every change, so the list of waiting panes can be read reactively.
const [version, setVersion] = createSignal(0);
const bump = () => setVersion((v) => v + 1);
/** `blockId\0source` → disposes the watch that ends that wait on its own. */
const watches = new Map<string, () => void>();

export interface WaitingOptions {
    /** How many questions it asks, for the notification's "(+N more)". */
    questionCount?: number;
    /**
     * Whether it still waits, read reactively. The registry ends the wait
     * when this turns false or the pane is deleted, whether or not the pane
     * is mounted, so a wait outlives a tab switch but not its pane.
     */
    stillWaiting?: () => boolean;
    /** What it wants in a few words, led by a verb. Absent, a default from the source's name. */
    what?: string;
}

/** The default `what` of a source that gives none. */
export function defaultWhat(source: string, text?: string): string {
    if (source === "question") return "asks you a question";
    if (source === "permission") {
        // A permission's text is "Allow <tool>?".
        const tool = /^Allow (.+)\?$/.exec(text ?? "")?.[1];
        return tool ? `asks to approve a ${tool} call` : "asks you to approve a tool call";
    }
    if (source === "shutdown") return "will stop unless you keep it running";
    return "needs you";
}

/** `source` (e.g. "question", "permission", "srv:<key>") now waits on the
 *  user in pane `blockId`; `text` says what it wants, for the notification. */
export function startWaitingForYou(blockId: string, source: string, text?: string, opts: WaitingOptions = {}): void {
    if (!blockId) return;
    let sources = waiting.get(blockId);
    if (!sources) {
        sources = new Map();
        waiting.set(blockId, sources);
    }
    const first = sources.size === 0;
    sources.set(source, { text, what: opts.what ?? defaultWhat(source, text) });
    if (first) since.set(blockId, Date.now());
    bump();
    if (opts.stillWaiting) watch(blockId, source, opts.stillWaiting);
    if (first) firePaneEvent(blockId, { type: "waiting-for-input", question: text, questionCount: opts.questionCount });
}

function watch(blockId: string, source: string, stillWaiting: () => boolean): void {
    const key = `${blockId}\0${source}`;
    // A pane that mounts again brings its own check: the old one may read a
    // disposed mount's state.
    watches.get(key)?.();
    const dispose = createRoot((d) => {
        createEffect(() => {
            const gone = !MOS.getMuxObjectAtom<Block>(`block:${blockId}`)();
            if (gone || !stillWaiting()) queueMicrotask(() => endWaitingForYou(blockId, source, gone ? "closed" : "submitted"));
        });
        return d;
    });
    watches.set(key, dispose);
}

/** `source` no longer waits in pane `blockId`: `submitted` when the user
 *  answered, `closed` when it went away unanswered. */
export function endWaitingForYou(blockId: string, source: string, reason: "submitted" | "closed" = "submitted"): void {
    const key = `${blockId}\0${source}`;
    watches.get(key)?.();
    watches.delete(key);
    const sources = waiting.get(blockId);
    if (!sources?.delete(source)) return;
    if (sources.size > 0) return;
    waiting.delete(blockId);
    since.delete(blockId);
    bump();
    firePaneEvent(blockId, { type: "waiting-ended", reason });
}

/** A pane that waits on the user in this window. */
export interface WaitingPane {
    blockId: string;
    /** What it wants, in a few words, led by a verb. */
    what: string;
    /** When it began to wait (unix ms). */
    since: number;
}

/** The panes of this window that wait on the user, oldest first; reactive. A pane that belongs to no
 *  agent (an `app:` request) is not one of them. */
export function waitingPanes(): WaitingPane[] {
    version();
    const out: WaitingPane[] = [];
    for (const [blockId, sources] of waiting) {
        if (blockId.startsWith(APP_KEY_PREFIX)) continue;
        const first = sources.values().next().value;
        if (first) out.push({ blockId, what: first.what, since: since.get(blockId) ?? 0 });
    }
    return out.sort((a, b) => a.since - b.since);
}

export function isWaitingForYou(blockId: string): boolean {
    return (waiting.get(blockId)?.size ?? 0) > 0;
}

/** Whether any agent in this window waits for the user. */
export function anyAgentWaitingForYou(): boolean {
    for (const blockId of waiting.keys()) if (isAgentWaitingForYou(blockId)) return true;
    return false;
}

/** Whether the agent itself waits in this pane (a question, a tool
 *  permission, or one of srv's [`AGENT_WAITS`]), which the pane's
 *  `term:awaiting_user` meta reports. */
export function isAgentWaitingForYou(blockId: string): boolean {
    const s = waiting.get(blockId);
    return !!s && (s.has("question") || s.has("permission") || [...s.keys()].some((k) => k.startsWith(AGENT_SOURCE)));
}

/** srv's calls to action that are the agent's own wait, as a question is: its
 *  tool call waits on the answer. They set the pane's "Waiting for you" too, in
 *  its tab and the Swarm, so a pane in a tab not in front still shows it. */
const AGENT_WAITS = new Set(["vault"]);
const AGENT_SOURCE = "srv:agent:";

function whatForKind(kind: string): string {
    switch (kind) {
        case "browser":
            return BROWSER_WHAT;
        case "consent":
            return "asks for your consent";
        case "confirm":
            return "asks you to confirm";
        case "widget":
            return "asks to install a widget";
        case "vault":
            return "needs a secret from you";
        default:
            return "needs you";
    }
}

/** srv's announcement of a call to action it holds open (`userattention`,
 *  crates/srv/src/backend/user_attention.rs), as its Rust type. */
export type UserAttention = UserAttentionEvent;

/** Apply one of srv's announcements, if it's for this window: one that
 *  names windows is for those; one that names none is for every window. */
export function applyUserAttention(a: UserAttention, thisWindow: string): void {
    if (!a?.key) return;
    if (a.window_ids?.length && !a.window_ids.includes(thisWindow)) return;
    const blockId = a.block_id || `${APP_KEY_PREFIX}${a.key}`;
    // Browser hand-offs and approvals can be left out (Settings → Sounds);
    // an end is always applied.
    if (a.active && a.kind === "browser" && !browserWaitsShown()) return;
    const agentWaits = AGENT_WAITS.has(a.kind);
    const source = agentWaits ? `${AGENT_SOURCE}${a.key}` : `srv:${a.key}`;
    if (a.active) startWaitingForYou(blockId, source, a.text, { what: whatForKind(a.kind) });
    else endWaitingForYou(blockId, source, "submitted");
    if (agentWaits && a.block_id) {
        // Loaded when used: the pane hooks that keep the meta import this registry.
        void import("@/app/view/agent/hooks/awaiting-user").then((m) => m.syncAwaitingUser(blockId)).catch(() => {});
    }
}

let installed = false;

/** Follow srv's announcements. Call once per window. */
export function installWaitingForYou(): void {
    if (installed) return;
    installed = true;
    onMuxEvent(WpsEvent.UserAttention, (data) => applyUserAttention(data, windowId()));
    // What was already open when this window started.
    void RpcApi.NotifyAttentionCommand(TabRpcClient)
        .then((r) => {
            for (const a of r.attention ?? []) applyUserAttention(a, windowId());
        })
        .catch(() => {
            /* older srv without notify.attention */
        });
    // The toast that names who waits and links to it, over the same list.
    void import("./attention-toast").then((m) => m.installAttentionToast());
}

/** Test helper. */
export function __resetWaitingForYou(): void {
    for (const d of watches.values()) d();
    watches.clear();
    waiting.clear();
    since.clear();
    bump();
}
