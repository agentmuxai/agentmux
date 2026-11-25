// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Pull requests not matched to a task": the pull requests srv saw that no card claimed (the tag named no agent
// of this computer, the number was ambiguous, ...), as a panel in the pane, never a modal. The operator links
// one to a task by number or dismisses it; both are the operator's `kanban.act` moves (`pr_link`, `pr_dismiss`).

import { createSignal, For, onMount, Show, type JSX } from "solid-js";

import { Button, CloseButton, Pill, TextInput } from "@/app/element/ui";
import { t as tr } from "@/app/i18n/i18n";
import { openLink } from "@/app/store/misc-utils";

import { parseAfter } from "./kanban-columns";
import type { KanbanSectionState } from "./kanban-section-state";
import type { KanbanUnmatchedPr } from "./kanban-types";

/** The row's pull request as srv takes it: "owner/repo#12". */
const nameOf = (p: KanbanUnmatchedPr): string => `${p.repo}#${p.number}`;

function UnmatchedRow(props: { pr: KanbanUnmatchedPr; reload: () => Promise<void>; s: KanbanSectionState }): JSX.Element {
    const [task, setTask] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    const run = async (req: Parameters<typeof props.s.model.act>[0]) => {
        setBusy(true);
        const res = await props.s.model.act(req);
        setBusy(false);
        if (res) await props.reload();
    };
    const taskId = () => parseAfter(task())[0];
    return (
        <li class="kanban-unmatched__row" data-testid={`kanban-unmatched-${props.pr.repo}-${props.pr.number}`}>
            <div class="kanban-unmatched__main">
                <a
                    href={props.pr.url}
                    onClick={(e) => {
                        e.preventDefault();
                        void openLink(props.pr.url);
                    }}
                >
                    {nameOf(props.pr)}
                </a>
                <Pill tone={props.pr.state === "merged" ? "ok" : props.pr.state === "closed" ? "neutral" : "accent"}>{props.pr.state}</Pill>
                <Show when={props.pr.why}>
                    <span class="warden-manager-dim">{props.pr.why}</span>
                </Show>
            </div>
            <form
                class="kanban-unmatched__actions"
                onSubmit={(e) => {
                    e.preventDefault();
                    const id = taskId();
                    if (id && !busy()) void run({ action: "pr_link", id, pr: nameOf(props.pr) });
                }}
            >
                <TextInput
                    density="compact"
                    placeholder={tr("kanban.prs.taskPlaceholder")}
                    aria-label={tr("kanban.prs.linkToLabel", { pr: nameOf(props.pr) })}
                    value={task()}
                    onInput={(e) => setTask(e.currentTarget.value)}
                />
                <Button type="submit" density="compact" icon="link" busy={busy()} disabled={!taskId()}>
                    {tr("kanban.prs.linkTo")}
                </Button>
                <Button density="compact" tone="quiet" disabled={busy()} onClick={() => run({ action: "pr_dismiss", pr: nameOf(props.pr) })}>
                    {tr("kanban.prs.dismiss")}
                </Button>
            </form>
        </li>
    );
}

export function KanbanUnmatchedPrsPanel(props: { s: KanbanSectionState }): JSX.Element {
    const s = props.s;
    const [rows, setRows] = createSignal<KanbanUnmatchedPr[] | null>(null);
    const reload = async () => {
        const res = await s.model.act({ action: "pr_unmatched" });
        if (res) setRows(res.unmatched ?? []);
    };
    onMount(() => void reload());
    return (
        <section class="kanban-boards kanban-unmatched" aria-label={tr("kanban.prs.unmatchedTitle")}>
            <div class="kanban-boards__head">
                <strong>{tr("kanban.prs.unmatchedTitle")}</strong>
                <CloseButton label={tr("kanban.prs.close")} onClick={() => s.setManaging(false)} />
            </div>
            <Show when={rows()} fallback={<p class="warden-manager-dim">{tr("kanban.prs.unmatchedLoading")}</p>}>
                {(list) => (
                    <Show when={list().length > 0} fallback={<p class="warden-manager-dim">{tr("kanban.prs.unmatchedNone")}</p>}>
                        <ul class="kanban-boards__list">
                            <For each={list()}>{(pr) => <UnmatchedRow pr={pr} reload={reload} s={s} />}</For>
                        </ul>
                    </Show>
                )}
            </Show>
        </section>
    );
}
