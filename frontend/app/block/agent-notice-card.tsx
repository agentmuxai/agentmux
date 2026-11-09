// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent starting an agent, told to the person in the agent's own pane
 * (docs/specs/SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md §2). Nothing waits for a click:
 * - `AgentNoticeCard`: the first start, with a countdown that goes ahead on its own. Stop it,
 *   Start now, and Always allow this agent (which belongs to the agent that was shown).
 * - `AgentNoticeLine`: a start that needs no answer (the person's own turn, the "Agents may start
 *   agents" setting, or an Always already given), one line that goes by itself.
 *
 * Both are drawn from srv's own record (`agent:notice`); the agent's words appear only as the
 * quoted brief, never as a title or a button. The dock is operator-only, so an agent's UI query,
 * click and keys reach neither. The tone, tab badge and notification come from srv's attention
 * event for the same pane, so the card adds no wait of its own.
 */

import { createSignal, onCleanup, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import type { PaneRequestProps } from "@/app/store/pane-requests";
import type { NoticeView } from "@/types/rpc/NoticeView";
import "./agent-notice-card.scss";

export type NoticeChoice = "veto" | "now" | "always";

/** `m:ss` of `seconds`. */
export function mmss(seconds: number): string {
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** Milliseconds left until `deadlineMs`, updated every 250 ms. */
function msLeft(deadlineMs: number): () => number {
    const [now, setNow] = createSignal(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 250);
    onCleanup(() => clearInterval(timer));
    return () => Math.max(0, deadlineMs - now());
}

function Quote(props: { text: string }): JSX.Element {
    return (
        <Show when={props.text}>
            <p class="agent-notice-quote">
                <q>{props.text}</q>
            </p>
        </Show>
    );
}

/** The words of the card's two timed actions: a start says Start, anything else Allow. */
function wording(kind: NoticeView["kind"]): { now: string; ahead: string } {
    return kind === "spawn" ? { now: "Start now", ahead: "Starts in" } : { now: "Allow now", ahead: "Goes ahead in" };
}

export function AgentNoticeCard(props: { notice: NoticeView; decide: (choice: NoticeChoice) => Promise<void> } & PaneRequestProps): JSX.Element {
    const n = props.notice;
    const words = wording(n.kind);
    const left = msLeft(n.deadline_ms);
    const total = Math.max(1, n.deadline_ms - n.opened_ms);
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    const answer = async (choice: NoticeChoice) => {
        setBusy(true);
        try {
            await props.decide(choice);
            props.close();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
            setBusy(false);
        }
    };
    return (
        <section class="agent-notice-card" aria-label={n.headline}>
            <h3 class="agent-notice-title">{n.headline}</h3>
            <div class="agent-notice-body">
                <p>{n.body}</p>
                <Quote text={n.quote} />
                <p class="agent-notice-deadline" role="timer">
                    <strong>
                        {words.ahead} {mmss(Math.ceil(left() / 1000))}
                    </strong>{" "}
                    unless you stop it.
                </p>
                <div class="agent-notice-bar" aria-hidden="true">
                    <div class="agent-notice-bar-fill" style={{ width: `${Math.min(100, (left() * 100) / total)}%` }} />
                </div>
                <Show when={error()}>
                    <div class="settings-config-error" role="alert">
                        {error()}
                    </div>
                </Show>
            </div>
            <div class="agent-notice-actions">
                <Button tone="danger" disabled={busy()} onClick={() => void answer("veto")}>
                    Stop it
                </Button>
                <Show when={n.always_label}>
                    <Button disabled={busy()} onClick={() => void answer("always")}>
                        {n.always_label}
                    </Button>
                </Show>
                <Button tone="accent" disabled={busy()} onClick={() => void answer("now")}>
                    {words.now}
                </Button>
            </div>
        </section>
    );
}

export function AgentNoticeLine(props: { notice: NoticeView } & PaneRequestProps): JSX.Element {
    const n = props.notice;
    return (
        <section class="agent-notice-line" aria-label={n.headline}>
            <p class="agent-notice-line-text">
                <strong>{n.headline}.</strong> {n.body}
                <Show when={n.quote}>
                    {" "}
                    <q>{n.quote}</q>
                </Show>
            </p>
            <Button variant="chrome" icon="xmark" aria-label="Dismiss" onClick={() => props.close()} />
        </section>
    );
}
