// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A task opened on the Kanban: its prompt, the operator's own words, the checklist, links, its history,
// and the moves its column allows (SPEC_WARDEN_KANBAN_2026_10_20.md §6.4, §6.7).

import { createEffect, createMemo, createSignal, For, on, Show, type Accessor, type JSX, type Setter } from "solid-js";

import { Button, Select, SidePanel, TextArea } from "@/app/element/ui";
import { createBlock } from "@/app/store/block-layout-actions";
import { openLink } from "@/app/store/misc-utils";
import { formatAge } from "@/app/view/warden-shared/warden-shared";

import { KanbanAfterSection } from "./kanban-after";
import { DrawerMedia } from "./kanban-media";
import { agentStateWords, AgentChip } from "./kanban-card";
import { columnOf, isFinishedColumn, isResearchKind, needsReason, requestedBy } from "./kanban-columns";
import type { KanbanModel } from "./kanban-model";
import type {
    KanbanActRequest,
    KanbanActResult,
    KanbanAfter,
    KanbanAgent,
    KanbanEvent,
    KanbanTask,
    KanbanTaskDetail,
} from "./kanban-types";
import { agentsByRecentUse } from "./kanban-agent-order";

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
    reopened: "took the acceptance back",
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

interface DrawerProps {
    model: KanbanModel;
    task: KanbanTask;
    agents: KanbanAgent[];
    now: number;
    onClose: () => void;
    onOpenAgent: (blockId: string) => void;
    onOpenTask: (id: string) => void;
}

/** What the drawer's parts share: its signals, what it reads off the task, and the two moves it makes. */
interface DrawerState {
    props: DrawerProps;
    t: () => KanbanTask;
    detail: Accessor<KanbanTaskDetail | null>;
    text: Accessor<string>;
    setText: Setter<string>;
    startOn: Accessor<string>;
    setStartOn: Setter<string>;
    busy: Accessor<boolean>;
    confirming: Accessor<{ words: string; req: KanbanActRequest } | null>;
    setConfirming: Setter<{ words: string; req: KanbanActRequest } | null>;
    giveTo: Accessor<string>;
    setGiveTo: Setter<string>;
    cancelBuild: Accessor<boolean>;
    setCancelBuild: Setter<boolean>;
    openFollowers: () => KanbanAfter[];
    docs: () => { label: string; path: string }[];
    column: () => ReturnType<typeof columnOf>;
    heldByAgent: () => boolean;
    otherAgents: () => KanbanAgent[];
    specUnlocks: () => string;
    runningAgents: () => KanbanAgent[];
    act: (req: KanbanActRequest) => Promise<KanbanActResult | null>;
    takeBack: (req: KanbanActRequest) => Promise<void>;
}

function createDrawerState(props: DrawerProps): DrawerState {
    const t = () => props.task;
    // Re-read the history whenever the task changes on the board. A signal filled by the fetch, never a
    // `createResource`: a pending resource read suspends the nearest `<Suspense>`, which is the pane's, and
    // the whole pane would be swapped for its spinner (and the board come back scrolled to the top).
    // A string, so an equal task after a refresh is the same source and isn't fetched again.
    const [fetched, setFetched] = createSignal<KanbanTaskDetail | null>(null);
    let asked = 0;
    const key = createMemo(() => `${t().id}|${t().updated_at}|${t().card.updated_at}`);
    createEffect(
        on(
            key,
            (key) => {
                const id = key.split("|")[0];
                const turn = ++asked;
                void props.model.detail(id).then((d) => {
                    if (turn === asked) setFetched(d);
                });
            }
        )
    );
    // The last answer stays while the same task is read again; another task's is never shown under this one.
    const detail = () => {
        const d = fetched();
        return d && d.task.id === t().id ? d : null;
    };
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
    const runningAgents = () => agentsByRecentUse(props.agents.filter((a) => a.state !== "stopped" && a.state !== "gone"));

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

    return {
        props, t, detail, text, setText, startOn, setStartOn, busy, confirming, setConfirming, giveTo, setGiveTo,
        cancelBuild, setCancelBuild, openFollowers, docs, column, heldByAgent, otherAgents, specUnlocks,
        runningAgents, act, takeBack,
    };
}

function DrawerFacts(props: { m: DrawerState }): JSX.Element {
    const m = props.m;
    const t = m.t;
    return (
        <div class="kanban-drawer__facts">
            <Show when={t().claimed_by || t().target_agent}>
                {(name) => (
                    <span>
                        <AgentChip name={name()} agent={t().holder ?? m.props.agents.find((a) => a.name === name())} />
                        <span class="warden-manager-dim"> {agentStateWords((t().holder ?? m.props.agents.find((a) => a.name === name()))?.state)}</span>
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
                        merged {formatAge(Math.max(0, m.props.now - at()))} ago
                        {t().card.pr?.released_in ? ` · released in ${t().card.pr?.released_in}` : ""}
                    </span>
                )}
            </Show>
        </div>
    );
}

/** Why it needs you, what you asked, the task's own words and its checklist. */
function DrawerAbout(props: { m: DrawerState }): JSX.Element {
    const t = props.m.t;
    return (
        <>
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
        </>
    );
}

function DrawerDocs(props: { m: DrawerState }): JSX.Element {
    return (
        <Show when={props.m.docs().length > 0}>
            <section class="kanban-drawer__section">
                <h4>Document</h4>
                <For each={props.m.docs()}>
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
    );
}

function DrawerResult(props: { m: DrawerState }): JSX.Element {
    const t = props.m.t;
    return (
        <Show when={t().result}>
            <section class="kanban-drawer__section">
                <h4>{t().state === "failed" || t().state === "open" ? "Last handed back because" : "Result"}</h4>
                <div class="kanban-drawer__payload">{t().result}</div>
            </section>
        </Show>
    );
}

/** Take it back from the agent holding it, or give it to another. */
function TakeBackRows(props: { m: DrawerState }): JSX.Element {
    const m = props.m;
    const t = m.t;
    return (
        <Show when={m.heldByAgent()}>
            <Show when={t().holder_unavailable}>
                <p class="warden-manager-dim">
                    {t().claimed_by || "Its agent"} can't work right now. Take the task back to let someone else do it.
                </p>
            </Show>
            <div class="kanban-drawer__row">
                <Button
                    density="compact"
                    icon="rotate-left"
                    disabled={m.busy()}
                    title="Return it to the backlog; its agent is told to stop"
                    onClick={() => void m.takeBack({ action: "release" })}
                >
                    Take it back
                </Button>
                <Select
                    value={m.giveTo()}
                    onChange={m.setGiveTo}
                    density="compact"
                    ariaLabel="Agent to give it to"
                    options={[
                        { value: "", label: "Give it to…" },
                        { value: ANYONE, label: "Anyone" },
                        ...m.otherAgents().map((a) => ({ value: a.name, label: `${a.name} (${agentStateWords(a.state)})` })),
                    ]}
                />
                <Button
                    density="compact"
                    tone="accent"
                    icon="share"
                    disabled={!m.giveTo() || m.busy()}
                    onClick={() => void m.takeBack({ action: "reassign", agent: m.giveTo() === ANYONE ? "" : m.giveTo() })}
                >
                    Give
                </Button>
            </div>
            <Show when={m.confirming()}>
                {(c) => (
                    <div class="kanban-drawer__confirm" role="alert">
                        <p class="kanban-drawer__reason">{c().words}</p>
                        <div class="kanban-drawer__row">
                            <Button
                                density="compact"
                                tone="danger"
                                icon="rotate-left"
                                disabled={m.busy()}
                                onClick={() => void m.takeBack({ ...c().req, confirm: true })}
                            >
                                Take it back anyway
                            </Button>
                            <Button density="compact" tone="quiet" disabled={m.busy()} onClick={() => m.setConfirming(null)}>
                                Leave it
                            </Button>
                        </div>
                    </div>
                )}
            </Show>
        </Show>
    );
}

/** Start a waiting task on an agent, hold it, or let anyone take it. */
function StartRows(props: { m: DrawerState }): JSX.Element {
    const m = props.m;
    const t = m.t;
    return (
        <Show when={m.column() === "backlog" || m.column() === "specd"}>
            <div class="kanban-drawer__row">
                <Select
                    value={m.startOn()}
                    onChange={m.setStartOn}
                    density="compact"
                    ariaLabel="Agent to start it on"
                    options={[{ value: "", label: "Start on…" }, ...m.runningAgents().map((a) => ({ value: a.name, label: `${a.name} (${agentStateWords(a.state)})` }))]}
                />
                <Button density="compact" tone="accent" icon="play" disabled={!m.startOn() || m.busy()} onClick={() => void m.act({ action: "start", agent: m.startOn() })}>
                    Start
                </Button>
                <Button density="compact" icon={t().state === "held" ? "play" : "pause"} disabled={m.busy()} onClick={() => void m.act({ action: "move", hold: t().state !== "held" })}>
                    {t().state === "held" ? "Release hold" : "Hold"}
                </Button>
                <Show when={t().target_agent}>
                    <Button density="compact" tone="quiet" disabled={m.busy()} onClick={() => void m.act({ action: "move", agent: "" })}>
                        Give to anyone
                    </Button>
                </Show>
            </div>
        </Show>
    );
}

/** The operator's words for an answer, a send-back, a reopening or a cancellation. */
function WordsBox(props: { m: DrawerState }): JSX.Element {
    const m = props.m;
    return (
        <Show when={m.t().state === "blocked" || m.t().state === "review"}>
            <TextArea variant="plain"
                class="kanban-drawer__text"
                rows={3}
                placeholder={m.t().state === "blocked" ? "Your answer" : "What to fix (for Send back)"}
                value={m.text()}
                onInput={(e) => m.setText(e.currentTarget.value)}
            />
        </Show>
    );
}

/** Answer, accept, send back, reopen, try again, open the agent, and cancel. */
function MoveButtons(props: { m: DrawerState }): JSX.Element {
    const m = props.m;
    const t = m.t;
    return (
        <div class="kanban-drawer__row">
            <Show when={t().state === "blocked"}>
                <Button density="compact" tone="accent" icon="reply" disabled={!m.text().trim() || m.busy()} onClick={() => void m.act({ action: "answer", text: m.text() })}>
                    Answer
                </Button>
            </Show>
            <Show when={t().state === "review"}>
                <Button
                    density="compact"
                    tone="accent"
                    icon="check"
                    disabled={m.busy()}
                    title={m.specUnlocks() || undefined}
                    onClick={() => void m.act({ action: "accept" })}
                >
                    {isResearchKind(t().kind) ? `Accept ${t().kind.trim().toLowerCase()}` : "Accept"}
                </Button>
                <Button density="compact" icon="rotate-left" disabled={m.busy()} onClick={() => void m.act({ action: "send_back", text: m.text() })}>
                    Send back
                </Button>
            </Show>
            <Show when={t().accepted_by}>
                {(a) => (
                    <Button
                        density="compact"
                        icon="rotate-left"
                        disabled={m.busy()}
                        title={`${a().name} accepted it. This puts it back in Review.`}
                        onClick={() => void m.act({ action: "reopen", text: m.text() })}
                    >
                        Reopen
                    </Button>
                )}
            </Show>
            <Show when={t().state === "failed"}>
                <Button density="compact" tone="accent" icon="rotate-right" disabled={m.busy()} onClick={() => void m.act({ action: "retry" })}>
                    Try again
                </Button>
            </Show>
            <Show when={t().holder}>
                {(h) => (
                    <Button density="compact" tone="quiet" icon="arrow-up-right-from-square" onClick={() => m.props.onOpenAgent(h().block_id)}>
                        Open {h().name}
                    </Button>
                )}
            </Show>
            <CancelControls m={m} />
        </div>
    );
}

/** Cancel the task, and say what becomes of the builds waiting on a spec. */
function CancelControls(props: { m: DrawerState }): JSX.Element {
    const m = props.m;
    return (
        <>
            <Show when={!isFinishedColumn(m.column()) && m.openFollowers().length > 0}>
                <label class="kanban-new__hold">
                    <input
                        type="checkbox"
                        checked={m.cancelBuild()}
                        onChange={(e) => m.setCancelBuild(e.currentTarget.checked)}
                    />
                    Cancelling also cancels {m.openFollowers().map((f) => `#${f.num}`).join(", ")} (else they go on
                    hold)
                </label>
            </Show>
            <Show when={!isFinishedColumn(m.column())}>
                <Button density="compact" tone="danger" icon="ban" disabled={m.busy()} onClick={() =>
                        void m.act({
                            action: "cancel",
                            text: m.text(),
                            followers: m.openFollowers().length ? (m.cancelBuild() ? "cancel" : "hold") : undefined,
                        })
                    }>
                    Cancel task
                </Button>
            </Show>
        </>
    );
}

function DrawerActions(props: { m: DrawerState }): JSX.Element {
    return (
        <section class="kanban-drawer__actions" data-toast-avoid>
            <TakeBackRows m={props.m} />
            <StartRows m={props.m} />
            <WordsBox m={props.m} />
            <MoveButtons m={props.m} />
        </section>
    );
}

function DrawerHistory(props: { m: DrawerState }): JSX.Element {
    return (
        <section class="kanban-drawer__section">
            <h4>History</h4>
            <Show when={props.m.detail()?.events} fallback={<p class="warden-manager-dim">Loading…</p>}>
                {(events) => (
                    <ul class="kanban-history">
                        <For each={events()}>{(ev) => <EventLine ev={ev} now={props.m.props.now} />}</For>
                    </ul>
                )}
            </Show>
        </section>
    );
}

export function KanbanDrawer(props: DrawerProps): JSX.Element {
    const m = createDrawerState(props);
    return (
        <SidePanel
            class="kanban-drawer"
            label={`Task ${m.t().card.num}`}
            lead={<span class="kanban-card__num">#{m.t().card.num}</span>}
            title={m.t().title}
            onClose={m.props.onClose}
        >
            <DrawerFacts m={m} />
            <DrawerAbout m={m} />

            <KanbanAfterSection model={props.model} task={m.t()} onOpen={props.onOpenTask} />

            <DrawerMedia task={m.t()} items={m.detail()?.media ?? []} now={props.now} busy={m.busy()} onAct={m.act} />
            <DrawerDocs m={m} />
            <DrawerResult m={m} />
            <DrawerActions m={m} />
            <DrawerHistory m={m} />
        </SidePanel>
    );
}
