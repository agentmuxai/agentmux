// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Reviews pane: every pull request a Kanban task names and where it stands in review
 * (docs/specs/SPEC_REVIEWS_PANE_2026_11_15.md). A list grouped by task with the ones that wait on a person first, and
 * the selected one's detail beside it; below about 440 px the detail replaces the list, with a back button, so no row
 * scrolls sideways. It only shows: a click opens the pull request on the web or the task on the Kanban.
 */

import { tip } from "@/app/element/tip";
import {
    Button,
    EmptyState,
    FilterInput,
    Notice,
    PaneBody,
    PaneScroll,
    Pill,
    SectionHeader,
    SegmentedControl,
    type PillTone,
} from "@/app/element/ui";
import { openLink } from "@/app/store/global";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { openKanbanTask } from "@/app/util/open-kanban-task";
import type { ReviewRow } from "@/types/rpc/ReviewRow";
import type { ReviewsVerdict } from "@/types/rpc/ReviewsVerdict";
import type { WaitsOn } from "@/types/rpc/WaitsOn";
import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import {
    ageText,
    filterRows,
    groupRows,
    inTaskRef,
    kanbanIdOf,
    rowKey,
    WAITS_LABEL,
    WAITS_OPTIONS,
    WAITS_SHORT,
    whyText,
    type ReviewsGroup,
} from "./reviews-model";
import type { ReviewsData } from "./reviews-store";
import "./reviews.scss";

/** The pane's meta key that opens it filtered to one task (a task number). */
export const REVIEWS_TASK_META = "reviews:task";

/** Below this width the detail replaces the list (the thin-pane rule: no sideways scroll at 280 to 440 px). */
export const NARROW_PX = 440;

const WAITS_TONE: Record<WaitsOn, PillTone> = {
    person: "warn",
    author: "warn",
    reviewer: "neutral",
    ci: "neutral",
    queue: "accent",
    none: "neutral",
    unknown: "neutral",
};

const BY_LABEL: Record<ReviewsVerdict["by"], string> = { bot: "Review bot", codex: "Codex", person: "Person" };

function verdictTone(v: ReviewsVerdict): PillTone {
    if (v.stale) return "warn";
    return v.state === "approved" ? "ok" : v.state === "changes_requested" ? "error" : "neutral";
}

function verdictText(v: ReviewsVerdict): string {
    const state =
        v.state === "approved"
            ? "approved"
            : v.state === "changes_requested"
              ? "changes requested"
              : v.state.replace(/_/g, " ");
    return v.stale ? `${state}, older commit` : state;
}

function checksTone(state: string): PillTone {
    return state === "passing" ? "ok" : state === "failing" ? "error" : "neutral";
}

function mergeTone(merge: string): PillTone {
    return merge === "merged" ? "ok" : merge === "open" ? "neutral" : "accent";
}

/** The row's title: its task's, else the branch, else its number. */
function titleOf(row: ReviewRow): string {
    return row.task?.title || row.branch || `#${row.number}`;
}

/** The row's number as people say it: `#7407`, or `owner/repo#7407`. */
function nameOf(row: ReviewRow): string {
    return row.repo ? `${row.repo}#${row.number}` : `#${row.number}`;
}

function ListRow(props: { row: ReviewRow; now: number; selected: boolean; onSelect: () => void }): JSX.Element {
    const row = () => props.row;
    return (
        <Button
            variant="row"
            pressed={props.selected}
            class="reviews-row"
            onClick={props.onSelect}
            data-testid="reviews-row"
        >
            <span class="reviews-row-line">
                <span class="reviews-row-number">#{row().number}</span>
                <span class="reviews-row-title">{titleOf(row())}</span>
            </span>
            <span class="reviews-row-chips">
                <Pill tone={WAITS_TONE[row().waits_on]} title={whyText(row().why)}>
                    {WAITS_SHORT[row().waits_on]}
                </Pill>
                <Show when={row().merge !== "open"}>
                    <Pill tone={mergeTone(row().merge)}>{row().merge}</Pill>
                </Show>
                <Show when={row().checks?.state === "failing"}>
                    <Pill tone="error">checks failing</Pill>
                </Show>
                <Show when={row().reviews.some((v) => v.stale)}>
                    <Pill tone="warn">stale review</Pill>
                </Show>
                <Show when={row().agent}>
                    <span class="reviews-row-agent">{row().agent}</span>
                </Show>
                <span class="reviews-row-age">{ageText(row().updated_at_ms, props.now)}</span>
            </span>
        </Button>
    );
}

function Fact(props: { label: string; children: JSX.Element }): JSX.Element {
    return (
        <>
            <dt>{props.label}</dt>
            <dd>{props.children}</dd>
        </>
    );
}

function Detail(props: { row: ReviewRow; now: number; narrow: boolean; onBack: () => void }): JSX.Element {
    const row = () => props.row;
    return (
        <div class="reviews-detail" data-testid="reviews-detail">
            <Show when={props.narrow}>
                <Button variant="link" icon="arrow-left" onClick={props.onBack} data-testid="reviews-back">
                    Back to the list
                </Button>
            </Show>
            <h3 class="reviews-detail-title">
                {nameOf(row())} {titleOf(row())}
            </h3>
            <div class="reviews-row-chips">
                <Pill tone={WAITS_TONE[row().waits_on]}>{WAITS_LABEL[row().waits_on]}</Pill>
                <Pill tone={mergeTone(row().merge)}>{row().merge}</Pill>
            </div>
            <p class="reviews-why">{whyText(row().why)}</p>
            <dl class="reviews-facts">
                <Show when={row().task}>
                    {(task) => (
                        <Fact label="Task">
                            {task().ref} {task().title} <span class="reviews-muted">({task().state})</span>
                        </Fact>
                    )}
                </Show>
                <Show when={row().agent}>
                    <Fact label="Agent">{row().agent}</Fact>
                </Show>
                <Show when={row().branch}>
                    <Fact label="Branch">{row().branch}</Fact>
                </Show>
                <Fact label="Reviews">
                    <Show when={row().reviews.length > 0} fallback={<span class="reviews-muted">none yet</span>}>
                        <For each={row().reviews}>
                            {(v) => (
                                <span class="reviews-verdict">
                                    {BY_LABEL[v.by]} <Pill tone={verdictTone(v)}>{verdictText(v)}</Pill>{" "}
                                    <span class="reviews-muted">{ageText(v.at_ms, props.now)}</span>
                                </span>
                            )}
                        </For>
                    </Show>
                </Fact>
                <Fact label="Checks">
                    <Show when={row().checks} fallback={<span class="reviews-muted">not reported</span>}>
                        {(checks) => <Pill tone={checksTone(checks().state)}>{checks().state}</Pill>}
                    </Show>
                </Fact>
                <Show when={row().released_in}>
                    <Fact label="Released in">{row().released_in}</Fact>
                </Show>
            </dl>
            <div class="reviews-actions">
                <Show when={row().url}>
                    {(url) => (
                        <Button icon="arrow-up-right-from-square" onClick={() => openLink(url())}>
                            Open on GitHub
                        </Button>
                    )}
                </Show>
                <Show when={row().task}>
                    {(task) => (
                        <Button icon="list-check" onClick={() => void openKanbanTask(kanbanIdOf(task().ref))}>
                            Open the task
                        </Button>
                    )}
                </Show>
            </div>
        </div>
    );
}

/** Whether the pane is narrower than [NARROW_PX], by its own width (a ResizeObserver; before it first reports, wide). */
function useNarrow(): { ref: (el: HTMLDivElement) => void; narrow: () => boolean } {
    const [width, setWidth] = createSignal(0);
    const ref = (root: HTMLDivElement) => {
        onMount(() => {
            if (typeof ResizeObserver === "undefined") return;
            const observer = new ResizeObserver((entries) => setWidth(entries[0]?.contentRect.width ?? 0));
            observer.observe(root);
            setWidth(root.getBoundingClientRect().width);
            onCleanup(() => observer.disconnect());
        });
    };
    return { ref, narrow: () => width() > 0 && width() < NARROW_PX };
}

/** The task this pane was opened for (`reviews:task`), when it has one. */
function taskOf(ctx: PaneTabHostContext): string | null {
    const v = ctx.meta()?.[REVIEWS_TASK_META];
    return typeof v === "number" || (typeof v === "string" && v !== "") ? String(v) : null;
}

/** The rows of the task the pane was opened for, or every row when it was opened for none. */
function inTask(rows: readonly ReviewRow[], task: string | null): readonly ReviewRow[] {
    return task ? rows.filter((r) => inTaskRef(r, task)) : rows;
}

function asOfText(atMs: number | undefined): string {
    return atMs ? new Date(atMs).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "";
}

function Bar(props: {
    text: string;
    onText: (text: string) => void;
    waitsOn: WaitsOn | null;
    onWaitsOn: (w: WaitsOn | null) => void;
    asOf: string;
}): JSX.Element {
    return (
        <div class="reviews-bar">
            <FilterInput
                value={props.text}
                onInput={props.onText}
                placeholder="Find a pull request, task or agent"
                testId="reviews-filter"
            />
            <SegmentedControl
                ariaLabel="Waits on"
                options={WAITS_OPTIONS.map((o) => ({ value: o.value ?? "all", label: o.label }))}
                value={props.waitsOn ?? "all"}
                onChange={(v) => props.onWaitsOn(v === "all" ? null : (v as WaitsOn))}
            />
            <span class="reviews-asof" {...tip("When this list was read")}>
                <Show when={props.asOf}>as of {props.asOf}</Show>
            </span>
        </div>
    );
}

function Rows(props: {
    groups: readonly ReviewsGroup[];
    any: boolean;
    now: number;
    picked: string | null;
    onPick: (key: string) => void;
}): JSX.Element {
    return (
        <PaneScroll class="reviews-list">
            <div data-testid="reviews-list">
                <Show
                    when={props.groups.length > 0}
                    fallback={
                        <EmptyState
                            title={props.any ? "Nothing matches" : "No pull requests on the board yet"}
                            icon="code-pull-request"
                        >
                            {props.any
                                ? "Clear the filter to see every pull request."
                                : "A task shows here once it has a pull request, or an agent is told of its review."}
                        </EmptyState>
                    }
                >
                    <For each={props.groups}>
                        {(group) => (
                            <section class="reviews-group" data-group={group.key}>
                                <SectionHeader variant="group" title={`${group.title} (${group.rows.length})`} />
                                <For each={group.rows}>
                                    {(row) => (
                                        <ListRow
                                            row={row}
                                            now={props.now}
                                            selected={rowKey(row) === props.picked}
                                            onSelect={() => props.onPick(rowKey(row))}
                                        />
                                    )}
                                </For>
                            </section>
                        )}
                    </For>
                </Show>
            </div>
        </PaneScroll>
    );
}

/** The notes between the bar and the list: the task the pane is limited to, an unreachable srv, where the list comes from. */
function Notes(props: { data: ReviewsData; ctx: PaneTabHostContext }): JSX.Element {
    return (
        <>
            <Show when={taskOf(props.ctx)}>
                {(task) => (
                    <div class="reviews-scope">
                        Showing task {task()}
                        <Button variant="link" onClick={() => void props.ctx.setMeta({ [REVIEWS_TASK_META]: null })}>
                            Show every task
                        </Button>
                    </div>
                )}
            </Show>
            <Show when={props.data.status() === "unavailable"}>
                <Notice tone="warning" title="Could not read the list">
                    This AgentMux cannot answer yet. It tries again every minute.
                </Notice>
            </Show>
            <Show when={props.data.list()?.last_notice_at_ms == null && props.data.status() === "ready"}>
                <p class="reviews-hint">
                    Showing pull requests your agents were notified about and the tasks that name one.
                </p>
            </Show>
        </>
    );
}

export function ReviewsView(props: { data: ReviewsData; ctx: PaneTabHostContext }): JSX.Element {
    const [text, setText] = createSignal("");
    const [waitsOn, setWaitsOn] = createSignal<WaitsOn | null>(null);
    const [picked, setPicked] = createSignal<string | null>(null);
    const { ref, narrow } = useNarrow();
    const rows = createMemo(() =>
        filterRows(inTask(props.data.list()?.rows ?? [], taskOf(props.ctx)), { text: text(), waitsOn: waitsOn() })
    );
    const groups = createMemo(() => groupRows(rows()));
    const selected = createMemo(() => rows().find((r) => rowKey(r) === picked()) ?? null);
    // On a narrow pane the detail takes the list's place, and the bar goes with the list.
    const detailOnly = () => narrow() && selected() !== null;

    return (
        <PaneBody class="reviews" density="compact">
            <div class="reviews-root" ref={ref} data-narrow={narrow() ? "true" : "false"}>
                <Show when={!detailOnly()}>
                    <Bar
                        text={text()}
                        onText={setText}
                        waitsOn={waitsOn()}
                        onWaitsOn={setWaitsOn}
                        asOf={asOfText(props.data.list()?.as_of_ms)}
                    />
                    <Notes data={props.data} ctx={props.ctx} />
                </Show>
                <div class="reviews-body">
                    <Show
                        when={props.data.status() !== "loading"}
                        fallback={
                            <EmptyState loading variant="fill">
                                Reading the pull requests
                            </EmptyState>
                        }
                    >
                        <Show when={!detailOnly()}>
                            <Rows
                                groups={groups()}
                                any={(props.data.list()?.rows.length ?? 0) > 0}
                                now={props.data.now()}
                                picked={picked()}
                                onPick={setPicked}
                            />
                        </Show>
                        <Show
                            when={selected()}
                            fallback={
                                <Show when={!narrow()}>
                                    <div class="reviews-detail reviews-detail--empty">
                                        <EmptyState>Select a pull request</EmptyState>
                                    </div>
                                </Show>
                            }
                        >
                            {(row) => (
                                <Detail
                                    row={row()}
                                    now={props.data.now()}
                                    narrow={narrow()}
                                    onBack={() => setPicked(null)}
                                />
                            )}
                        </Show>
                    </Show>
                </div>
            </div>
        </PaneBody>
    );
}
