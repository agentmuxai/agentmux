// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A task opened on the Kanban: its prompt, the operator's own words, the checklist, links, its history,
// and the moves its column allows (SPEC_WARDEN_KANBAN_2026_10_20.md §6.4, §6.7).

import { createResource, createSignal, For, Show, type JSX } from "solid-js";

import { Button, IconButton, Select } from "@/app/element/ui";
import { createBlock } from "@/app/store/block-layout-actions";
import { openLink } from "@/app/store/misc-utils";
import { formatAge } from "@/app/view/warden-shared/warden-shared";

import { KanbanAfterSection } from "./kanban-after";
import { agentStateWords, AgentChip } from "./kanban-card";
import { columnOf, isFinishedColumn, isResearchKind, needsReason, requestedBy } from "./kanban-columns";
import type { KanbanModel } from "./kanban-model";
import type { KanbanActRequest, KanbanAgent, KanbanEvent, KanbanTask } from "./kanban-types";

const EVENT_WORDS: Record<string, string> = {
    created: "created it",
    filed_for_later: "filed it for later",
    claimed: "took it",
    started: "started it",
    held: "put it on hold",
    unheld: "released it from hold",
    assigned: "moved it",
    edited: "edited it",
    updated: "updated it",
    asked: "asked",
    answered: "answered",
    submitted: "handed it in",
    completed: "finished it",
    accepted: "accepted it",
    sent_back: "sent it back",
    released: "handed it back",
    expired: "lost its lease",
    cancelled: "cancelled it",
    retried: "tried it again",
    linked: "linked",
    reminded: "reminded its agent",
    tracked: "started following it",
    taken_back: "took it back",
    reassigned: "took it back and gave it to someone else",
};

/** The "Give it to…" choice for anyone: no agent in particular. */
const ANYONE = "__anyone";

function EventLine(props: { ev: KanbanEvent; now: number }): JSX.Element {
    const who = () => (props.ev.actor_kind === "operator" ? "You" : props.ev.actor || "AgentMux");
    return (
        <li class="kanban-history__line">
            <span class="kanban-history__age">{formatAge(Math.max(0, props.now - props.ev.at))}</span>
            <span>
                {who()} {EVENT_WORDS[props.ev.kind] ?? props.ev.kind}
                <Show when={props.ev.body}>
                    {(b) => <span class="kanban-history__body">: {b()}</span>}
                </Show>
            </span>
        </li>
    );
}

export function KanbanDrawer(props: {
    model: KanbanModel;
    task: KanbanTask;
    agents: KanbanAgent[];
    now: number;
    onClose: () => void;
    onOpenAgent: (blockId: string) => void;
    onOpenTask: (id: string) => void;
}): JSX.Element {
    const t = () => props.task;
    // Re-read the history whenever the task changes on the board.
    // A string, so an equal task after a refresh is the same source and isn't fetched again.
    const [detail] = createResource(
        () => `${t().id}|${t().updated_at}|${t().card.updated_at}`,
        (key) => props.model.detail(key.split("|")[0])
    );
    const [text, setText] = createSignal("");
    const [startOn, setStartOn] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    // Taking a task back that has a branch or PR asks first: what srv said, and the move to repeat once confirmed.
    const [confirming, setConfirming] = createSignal<{ words: string; req: KanbanActRequest } | null>(null);
    const [giveTo, setGiveTo] = createSignal("");
    // Cancelling a spec: its build is cancelled too, unless unticked (then held).
    const [cancelBuild, setCancelBuild] = createSignal(true);
    const openFollowers = () =>
        isResearchKind(t().kind)
            ? (t().card.followers ?? []).filter((f) => f.state === "open" || f.state === "held")
            : [];
    // The document a task wrote, or, for a build, the one its spec wrote.
    const docs = () => {
        const own = t().card.doc ? [{ label: "", path: t().card.doc as string }] : [];
        // From what it starts after, so a spec finished long ago (off the board) still shows.
        const fromSpecs = (t().card.after ?? [])
            .filter((a) => isResearchKind(a.kind) && a.doc)
            .map((a) => ({ label: `${a.kind} #${a.num}: `, path: a.doc as string }));
        return [...own, ...fromSpecs];
    };
    const column = () => columnOf(t());
    // An agent holds it (claimed, or blocked on a question): the operator can take it back from them.
    const heldByAgent = () => t().state === "claimed" || t().state === "blocked";
    const otherAgents = () => runningAgents().filter((a) => a.name !== t().claimed_by);
    // What accepting a spec unlocks: "Unlocks #1106 'Build: …' for Naki-2".
    const specUnlocks = () =>
        isResearchKind(t().kind)
            ? (t().card.followers ?? [])
                  .filter((f) => f.state === "open" || f.state === "held")
                  .map((f) => `Unlocks #${f.num} "${f.title}"`)
                  .join("; ")
            : "";
    const runningAgents = () => props.agents.filter((a) => a.state !== "stopped" && a.state !== "gone");

    const act = async (req: KanbanActRequest) => {
        setBusy(true);
        const res = await props.model.act({ id: t().id, ...req });
        setBusy(false);
        if (res) setText("");
        return res;
    };

    /** `release` or `reassign`; when srv asks, show its words and wait for the operator's yes. */
    const takeBack = async (req: KanbanActRequest) => {
        const res = await act(req);
        setConfirming(res && !res.ok && res.confirm ? { words: res.confirm, req } : null);
        if (res?.ok) setGiveTo("");
    };

    return (
        <aside class="kanban-drawer" aria-label={`Task ${t().card.num}`}>
            <header class="kanban-drawer__head">
                <span class="kanban-card__num">#{t().card.num}</span>
                <h3 class="kanban-drawer__title">{t().title}</h3>
                <IconButton icon="xmark" label="Close" onClick={props.onClose} />
            </header>

            <div class="kanban-drawer__facts">
                <Show when={t().claimed_by || t().target_agent}>
                    {(name) => (
                        <span>
                            <AgentChip name={name()} agent={t().holder ?? props.agents.find((a) => a.name === name())} />
                            <span class="warden-manager-dim"> {agentStateWords((t().holder ?? props.agents.find((a) => a.name === name()))?.state)}</span>
                        </span>
                    )}
                </Show>
                <span class="warden-manager-dim">asked by {requestedBy(t())}</span>
                <Show when={t().card.repo}>
                    <span class="warden-manager-dim">{t().card.repo}{t().card.branch ? ` · ${t().card.branch}` : ""}</span>
                </Show>
                <Show when={t().card.pr_url}>
                    <Button density="compact" tone="quiet" icon="code-pull-request" onClick={() => void openLink(t().card.pr_url)}>
                        Open the PR
                    </Button>
                </Show>
                <Show when={t().card.pr?.merged_at}>
                    {(at) => (
                        <span class="warden-manager-dim">
                            merged {formatAge(Math.max(0, props.now - at()))} ago
                            {t().card.pr?.released_in ? ` · released in ${t().card.pr?.released_in}` : ""}
                        </span>
                    )}
                </Show>
            </div>

            <Show when={needsReason(t())}>{(r) => <p class="kanban-drawer__reason">{r()}</p>}</Show>

            <Show when={t().card.request_text}>
                <section class="kanban-drawer__section">
                    <h4>What you asked</h4>
                    <blockquote class="kanban-drawer__quote">{t().card.request_text}</blockquote>
                </section>
            </Show>

            <section class="kanban-drawer__section">
                <h4>The task</h4>
                <div class="kanban-drawer__payload">{t().payload}</div>
            </section>

            <Show when={t().card.checklist.length > 0}>
                <section class="kanban-drawer__section">
                    <h4>Checklist</h4>
                    <ul class="kanban-checklist">
                        <For each={t().card.checklist}>
                            {(item) => (
                                <li classList={{ "kanban-checklist__done": item.status === "completed", "kanban-checklist__now": item.status === "in_progress" }}>
                                    <i
                                        class={`fa-solid fa-${item.status === "completed" ? "square-check" : item.status === "in_progress" ? "spinner" : "square"}`}
                                        aria-hidden="true"
                                    />
                                    {item.text}
                                </li>
                            )}
                        </For>
                    </ul>
                </section>
            </Show>

            <KanbanAfterSection model={props.model} task={t()} onOpen={props.onOpenTask} />

            <Show when={docs().length > 0}>
                <section class="kanban-drawer__section">
                    <h4>Document</h4>
                    <For each={docs()}>
                        {(d) => (
                            <div class="kanban-drawer__row">
                                <span class="kanban-drawer__doc">
                                    {d.label}
                                    {d.path}
                                </span>
                                <Button
                                    density="compact"
                                    tone="quiet"
                                    icon="file-lines"
                                    onClick={() => void createBlock({ meta: { view: "editor", file: d.path } })}
                                >
                                    Open
                                </Button>
                            </div>
                        )}
                    </For>
                </section>
            </Show>

            <Show when={t().result}>
                <section class="kanban-drawer__section">
                    <h4>{t().state === "failed" || t().state === "open" ? "Last handed back because" : "Result"}</h4>
                    <div class="kanban-drawer__payload">{t().result}</div>
                </section>
            </Show>

            <section class="kanban-drawer__actions">
                <Show when={heldByAgent()}>
                    <Show when={t().holder_unavailable}>
                        <p class="warden-manager-dim">
                            {t().claimed_by || "Its agent"} can't work right now. Take the task back to let someone else do it.
                        </p>
                    </Show>
                    <div class="kanban-drawer__row">
                        <Button
                            density="compact"
                            icon="rotate-left"
                            disabled={busy()}
                            title="Return it to the backlog; its agent is told to stop"
                            onClick={() => void takeBack({ action: "release" })}
                        >
                            Take it back
                        </Button>
                        <Select
                            value={giveTo()}
                            onChange={setGiveTo}
                            density="compact"
                            ariaLabel="Agent to give it to"
                            options={[
                                { value: "", label: "Give it to…" },
                                { value: ANYONE, label: "Anyone" },
                                ...otherAgents().map((a) => ({ value: a.name, label: `${a.name} (${agentStateWords(a.state)})` })),
                            ]}
                        />
                        <Button
                            density="compact"
                            tone="accent"
                            icon="share"
                            disabled={!giveTo() || busy()}
                            onClick={() => void takeBack({ action: "reassign", agent: giveTo() === ANYONE ? "" : giveTo() })}
                        >
                            Give
                        </Button>
                    </div>
                    <Show when={confirming()}>
                        {(c) => (
                            <div class="kanban-drawer__confirm" role="alert">
                                <p class="kanban-drawer__reason">{c().words}</p>
                                <div class="kanban-drawer__row">
                                    <Button
                                        density="compact"
                                        tone="danger"
                                        icon="rotate-left"
                                        disabled={busy()}
                                        onClick={() => void takeBack({ ...c().req, confirm: true })}
                                    >
                                        Take it back anyway
                                    </Button>
                                    <Button density="compact" tone="quiet" disabled={busy()} onClick={() => setConfirming(null)}>
                                        Leave it
                                    </Button>
                                </div>
                            </div>
                        )}
                    </Show>
                </Show>
                <Show when={column() === "backlog" || column() === "specd"}>
                    <div class="kanban-drawer__row">
                        <Select
                            value={startOn()}
                            onChange={setStartOn}
                            density="compact"
                            ariaLabel="Agent to start it on"
                            options={[{ value: "", label: "Start on…" }, ...runningAgents().map((a) => ({ value: a.name, label: `${a.name} (${agentStateWords(a.state)})` }))]}
                        />
                        <Button density="compact" tone="accent" icon="play" disabled={!startOn() || busy()} onClick={() => void act({ action: "start", agent: startOn() })}>
                            Start
                        </Button>
                        <Button density="compact" icon={t().state === "held" ? "play" : "pause"} disabled={busy()} onClick={() => void act({ action: "move", hold: t().state !== "held" })}>
                            {t().state === "held" ? "Release hold" : "Hold"}
                        </Button>
                        <Show when={t().target_agent}>
                            <Button density="compact" tone="quiet" disabled={busy()} onClick={() => void act({ action: "move", agent: "" })}>
                                Give to anyone
                            </Button>
                        </Show>
                    </div>
                </Show>
                <Show when={t().state === "blocked" || t().state === "review"}>
                    <textarea
                        class="kanban-drawer__text"
                        rows={3}
                        placeholder={t().state === "blocked" ? "Your answer" : "What to fix (for Send back)"}
                        value={text()}
                        onInput={(e) => setText(e.currentTarget.value)}
                    />
                </Show>
                <div class="kanban-drawer__row">
                    <Show when={t().state === "blocked"}>
                        <Button density="compact" tone="accent" icon="reply" disabled={!text().trim() || busy()} onClick={() => void act({ action: "answer", text: text() })}>
                            Answer
                        </Button>
                    </Show>
                    <Show when={t().state === "review"}>
                        <Button
                            density="compact"
                            tone="accent"
                            icon="check"
                            disabled={busy()}
                            title={specUnlocks() || undefined}
                            onClick={() => void act({ action: "accept" })}
                        >
                            {isResearchKind(t().kind) ? `Accept ${t().kind.trim().toLowerCase()}` : "Accept"}
                        </Button>
                        <Button density="compact" icon="rotate-left" disabled={busy()} onClick={() => void act({ action: "send_back", text: text() })}>
                            Send back
                        </Button>
                    </Show>
                    <Show when={t().state === "failed"}>
                        <Button density="compact" tone="accent" icon="rotate-right" disabled={busy()} onClick={() => void act({ action: "retry" })}>
                            Try again
                        </Button>
                    </Show>
                    <Show when={t().holder}>
                        {(h) => (
                            <Button density="compact" tone="quiet" icon="arrow-up-right-from-square" onClick={() => props.onOpenAgent(h().block_id)}>
                                Open {h().name}
                            </Button>
                        )}
                    </Show>
                    <Show when={!isFinishedColumn(column()) && openFollowers().length > 0}>
                        <label class="kanban-new__hold">
                            <input
                                type="checkbox"
                                checked={cancelBuild()}
                                onChange={(e) => setCancelBuild(e.currentTarget.checked)}
                            />
                            Cancelling also cancels {openFollowers().map((f) => `#${f.num}`).join(", ")} (else they go on
                            hold)
                        </label>
                    </Show>
                    <Show when={!isFinishedColumn(column())}>
                        <Button density="compact" tone="danger" icon="ban" disabled={busy()} onClick={() =>
                                void act({
                                    action: "cancel",
                                    text: text(),
                                    followers: openFollowers().length ? (cancelBuild() ? "cancel" : "hold") : undefined,
                                })
                            }>
                            Cancel task
                        </Button>
                    </Show>
                </div>
            </section>

            <section class="kanban-drawer__section">
                <h4>History</h4>
                <Show when={detail()?.events} fallback={<p class="warden-manager-dim">Loading…</p>}>
                    {(events) => (
                        <ul class="kanban-history">
                            <For each={events()}>{(ev) => <EventLine ev={ev} now={props.now} />}</For>
                        </ul>
                    )}
                </Show>
            </section>
        </aside>
    );
}
