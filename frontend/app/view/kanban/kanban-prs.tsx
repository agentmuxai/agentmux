// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A card's pull requests: the chip that says how many have merged, the line for a held task whose work has
// all merged, and the drawer's list. srv derives the roll-up (`pr_rollup`); nothing here recomputes it, so an
// older srv with no list shows the card as it always did.

import { createSignal, For, Show, type JSX } from "solid-js";

import { t as tr } from "@/app/i18n/i18n";
import { Button, CloseButton, Pill, TextInput, type PillTone } from "@/app/element/ui";
import { openLink } from "@/app/store/misc-utils";
import { formatAge } from "@/app/view/warden-shared/warden-shared";

import type { KanbanActRequest, KanbanActResult, KanbanCardFields, KanbanCardPr } from "./kanban-types";

/** The drawer's move on its task: srv is asked with the task's id filled in; null when the call failed (the board shows why). */
export type PrAct = (req: KanbanActRequest) => Promise<KanbanActResult | null>;

/** The pull requests a card lists, newest first as srv sends them; none for an older srv. */
export function prsOf(card: KanbanCardFields): KanbanCardPr[] {
    return (card.prs ?? []).filter((p) => !p.unlinked);
}

/** One pull request's state in words: "merged", "open, approved", "open, checks failing". */
export function prStateWords(p: KanbanCardPr): string {
    if (p.state === "merged") return "merged";
    if (p.state === "closed") return "closed without merging";
    const parts = ["open"];
    if (p.review === "approved") parts.push("approved");
    if (p.review === "changes_requested") parts.push("changes requested");
    if (p.checks === "failing") parts.push("checks failing");
    if (p.checks === "pending") parts.push("checks running");
    return parts.join(", ");
}

/** A pull request's name: "owner/name#12". */
export const prName = (p: KanbanCardPr): string => `${p.repo}#${p.number}`;

export interface PrChipView {
    label: string;
    tone: PillTone;
    title: string;
}

/** The count and worst state of a card with several pull requests; null for one or none (the PR's own pill stays). */
export function prChip(card: KanbanCardFields): PrChipView | null {
    const roll = card.pr_rollup;
    const prs = prsOf(card);
    if (!roll || roll.total < 2) return null;
    const label = (roll.merged > 0 ? `${roll.merged} of ${roll.total} merged` : `${roll.total} PRs`) + (roll.follow_up_open ? " · follow-up open" : "");
    const trouble = prs.some((p) => p.state === "open" && (p.checks === "failing" || p.review === "changes_requested"));
    const tone: PillTone = trouble ? "warn" : roll.state === "merged" ? "ok" : "accent";
    return { label, tone, title: prs.map((p) => `${prName(p)}: ${prStateWords(p)}`).join("\n") };
}

/** The chip of a card with several pull requests. */
export function PrChip(props: { card: KanbanCardFields }): JSX.Element {
    return (
        <Show when={prChip(props.card)}>
            {(c) => (
                <Pill icon="code-pull-request" tone={c().tone} title={c().title}>
                    {c().label}
                </Pill>
            )}
        </Show>
    );
}

/** "3 of 3 merged, waiting on" and the holder, for a held task whose pull requests have all merged. */
export function WaitingOnLine(props: { card: KanbanCardFields; holder: () => JSX.Element }): JSX.Element {
    return (
        <Show when={props.card.pr_rollup?.all_merged_waiting ? props.card.pr_rollup : undefined}>
            {(r) => (
                <div class="kanban-card__waiting">
                    {tr("kanban.prs.waitingOn", { merged: r().merged, total: r().total })} {props.holder()}
                </div>
            )}
        </Show>
    );
}

/** When a pull request merged, or else opened, as the drawer's row says it. */
function prAge(p: KanbanCardPr, now: number): string | null {
    const at = p.merged_at ?? p.opened_at;
    if (!at) return null;
    return tr(p.state === "merged" ? "kanban.prs.mergedAgo" : "kanban.prs.openedAgo", { age: formatAge(Math.max(0, now - at)) });
}

/** Takes one pull request off the card (the operator's `pr_unlink`; srv records it so it is not matched back). */
function UnlinkPr(props: { pr: KanbanCardPr; act: PrAct }): JSX.Element {
    const [busy, setBusy] = createSignal(false);
    return (
        <CloseButton
            label={tr("kanban.prs.unlink", { pr: prName(props.pr) })}
            disabled={busy()}
            onClick={() => {
                setBusy(true);
                void props.act({ action: "pr_unlink", pr: prName(props.pr) }).finally(() => setBusy(false));
            }}
        />
    );
}

/** "Add a pull request": an address or `owner/repo#12` typed by the operator, linked to the card. */
function AddPr(props: { act: PrAct }): JSX.Element {
    const [text, setText] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    const [already, setAlready] = createSignal(false);
    return (
        <form
            class="kanban-drawer__pr-add"
            onSubmit={(e) => {
                e.preventDefault();
                const pr = text().trim();
                if (!pr || busy()) return;
                setBusy(true);
                void props
                    .act({ action: "pr_link", pr })
                    .then((res) => {
                        if (!res) return;
                        setText("");
                        setAlready(res.changed === false);
                    })
                    .finally(() => setBusy(false));
            }}
        >
            <TextInput
                density="compact"
                placeholder={tr("kanban.prs.addPlaceholder")}
                aria-label={tr("kanban.prs.addLabel")}
                value={text()}
                onInput={(e) => {
                    setText(e.currentTarget.value);
                    setAlready(false);
                }}
            />
            <Button type="submit" density="compact" icon="plus" busy={busy()} disabled={!text().trim()}>
                {tr("kanban.prs.add")}
            </Button>
            <Show when={already()}>
                <span class="warden-manager-dim">{tr("kanban.prs.already")}</span>
            </Show>
        </form>
    );
}

/**
 * The drawer's list: each pull request of the card, its state and age, newest first, each with Unlink; one pull
 * request keeps the drawer's own button and gets an Unlink beside the add box. `act` is absent where the card is
 * only shown (no moves).
 */
export function DrawerPrs(props: { card: KanbanCardFields; now: number; act?: PrAct }): JSX.Element {
    const prs = () => prsOf(props.card);
    return (
        <>
            <Show when={prs().length > 1}>
                <ul class="kanban-drawer__prs" aria-label={tr("kanban.prs.list")}>
                    <For each={prs()}>
                        {(p) => (
                            <li class="kanban-drawer__pr">
                                <a
                                    href={p.url}
                                    onClick={(e) => {
                                        e.preventDefault();
                                        void openLink(p.url);
                                    }}
                                >
                                    {prName(p)}
                                </a>
                                <Pill tone={p.state === "merged" ? "ok" : p.state === "closed" ? "neutral" : "accent"}>{prStateWords(p)}</Pill>
                                <Show when={p.followed === false}>
                                    <Pill tone="warn" title={tr("kanban.prs.notFollowedWhy")}>
                                        {tr("kanban.prs.notFollowed")}
                                    </Pill>
                                </Show>
                                <Show when={prAge(p, props.now)}>{(age) => <span class="warden-manager-dim">{age()}</span>}</Show>
                                <Show when={props.act}>{(act) => <UnlinkPr pr={p} act={act()} />}</Show>
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
            <Show when={prs().length === 1 && prs()[0].followed === false}>
                <Pill tone="warn" title={tr("kanban.prs.notFollowedWhy")}>
                    {tr("kanban.prs.notFollowed")}
                </Pill>
            </Show>
            <Show when={props.act}>
                {(act) => (
                    <>
                        <Show when={prs().length === 1 ? prs()[0] : undefined}>
                            {(p) => (
                                <Button
                                    density="compact"
                                    tone="quiet"
                                    icon="link-slash"
                                    onClick={() => void act()({ action: "pr_unlink", pr: prName(p()) })}
                                >
                                    {tr("kanban.prs.unlink", { pr: prName(p()) })}
                                </Button>
                            )}
                        </Show>
                        <AddPr act={act()} />
                    </>
                )}
            </Show>
        </>
    );
}
