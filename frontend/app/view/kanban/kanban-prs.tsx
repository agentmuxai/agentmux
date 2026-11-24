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
import { openConnectors } from "@/app/store/section-panes";
import { formatAge } from "@/app/view/warden-shared/warden-shared";

import type { KanbanActRequest, KanbanActResult, KanbanCardFields, KanbanCardPr, KanbanWaitsOn } from "./kanban-types";

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

/** A chip of the card's pull request line: its words (the tooltip when the card is narrow), an icon and a tone. */
export interface FlowChip {
    word: string;
    icon: string;
    tone: PillTone;
}

/** What the pull request line of a card shows: the badge for who it waits on (none when srv cannot place it), the
 *  verdict and checks chips of the pull request that blocks it, and how many other open ones the card has. */
export interface PrFlowView {
    badge: { word: string; tone: PillTone; title: string } | null;
    chips: FlowChip[];
    more: number;
}

/** Why a pull request waits, in a sentence (the badge's tooltip). */
export function whyWords(why: string | undefined): string {
    switch (why) {
        case "changes_requested":
            return tr("kanban.why.changesRequested");
        case "checks_failing":
            return tr("kanban.why.checksFailing");
        case "checks_running":
            return tr("kanban.why.checksRunning");
        case "no_review":
            return tr("kanban.why.noReview");
        case "review_stale":
            return tr("kanban.why.reviewStale");
        case "awaiting_accept":
            return tr("kanban.why.awaitingAccept");
        case "ready_to_merge":
            return tr("kanban.why.readyToMerge");
        case "queued":
            return tr("kanban.why.queued");
        default:
            return "";
    }
}

/** The badge's words and tone for who a pull request waits on; null for nothing or what the rule could not place. */
function badgeOf(wait: KanbanWaitsOn | undefined, why: string | undefined): { word: string; tone: PillTone } | null {
    if (why === "ready_to_merge") return { word: tr("kanban.flow.ready"), tone: "ok" };
    switch (wait) {
        case "person":
            return { word: tr("kanban.flow.person"), tone: "accent" };
        case "author":
            return { word: tr("kanban.flow.author"), tone: "warn" };
        case "reviewer":
            return { word: tr("kanban.flow.reviewer"), tone: "neutral" };
        case "ci":
            return { word: tr("kanban.flow.ci"), tone: "neutral" };
        case "queue":
            return { word: tr("kanban.flow.queue"), tone: "neutral" };
        default:
            return null;
    }
}

/** The verdict and checks chips of one pull request; none for what srv was not told. */
export function flowChips(p: KanbanCardPr): FlowChip[] {
    const chips: FlowChip[] = [];
    const stale = (p.verdicts ?? []).some((v) => v.stale);
    if (p.review === "approved") chips.push({ word: tr("kanban.flow.approved"), icon: "check", tone: "ok" });
    else if (p.review === "changes_requested") chips.push({ word: tr("kanban.flow.changes"), icon: "pen", tone: "warn" });
    else if (stale) chips.push({ word: tr("kanban.flow.stale"), icon: "clock-rotate-left", tone: "neutral" });
    if (p.checks === "passing") chips.push({ word: tr("kanban.flow.checksPassing"), icon: "circle-check", tone: "ok" });
    else if (p.checks === "failing") chips.push({ word: tr("kanban.flow.checksFailing"), icon: "circle-exclamation", tone: "error" });
    else if (p.checks === "pending") chips.push({ word: tr("kanban.flow.checksRunning"), icon: "spinner", tone: "neutral" });
    return chips;
}

/** The card's pull request line, or null for a card with no open pull request (no line). A pull request srv
 *  cannot place has the line with no badge: never a guess. */
export function prFlow(card: KanbanCardFields): PrFlowView | null {
    const roll = card.pr_rollup;
    if (!roll || roll.open < 1) return null;
    const blocking = prsOf(card).find((p) => prName(p) === roll.blocking);
    const badge = badgeOf(roll.waits_on, roll.why);
    return {
        badge: badge && { ...badge, title: [blocking ? prName(blocking) : "", whyWords(roll.why)].filter(Boolean).join(": ") },
        chips: blocking ? flowChips(blocking) : [],
        more: roll.waits_more ?? 0,
    };
}

/** The line itself: reserved once the card has an open pull request, so late data does not change the card's height.
 *  Below the card's narrow width the chips keep their icon and the words move to the tooltip. */
export function PrFlowLine(props: { card: KanbanCardFields }): JSX.Element {
    return (
        <Show when={prFlow(props.card)}>
            {(f) => (
                <div class="kanban-card__flow">
                    <Show when={f().badge}>
                        {(b) => (
                            <Pill tone={b().tone} title={b().title}>
                                {b().word}
                            </Pill>
                        )}
                    </Show>
                    <For each={f().chips}>
                        {(c) => (
                            <Pill tone={c.tone} icon={c.icon} title={c.word} class="kanban-card__flow-chip">
                                <span class="kanban-card__flow-word">{c.word}</span>
                            </Pill>
                        )}
                    </For>
                    <Show when={f().more > 0}>
                        <span class="warden-manager-dim">{tr("kanban.flow.more", { n: f().more })}</span>
                    </Show>
                </div>
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

/** A reviewer kind as a person reads it. */
function byWords(by: string): string {
    return by === "bot" ? tr("kanban.by.bot") : by === "codex" ? tr("kanban.by.codex") : tr("kanban.by.person");
}

/** What a pull request waits on, each verdict with the commit it was given on, and its checks. */
function PrFlowDetail(props: { pr: KanbanCardPr }): JSX.Element {
    const badge = () => badgeOf(props.pr.waits_on, props.pr.why);
    const verdicts = () => props.pr.verdicts ?? [];
    const checks = () => flowChips(props.pr).filter((c) => c.icon.includes("circle") || c.icon === "spinner");
    return (
        <Show when={props.pr.state === "open" && (badge() || verdicts().length > 0 || checks().length > 0)}>
            <div class="kanban-drawer__pr-flow">
                <Show when={badge()}>
                    {(b) => (
                        <Pill tone={b().tone} title={whyWords(props.pr.why)}>
                            {b().word}
                        </Pill>
                    )}
                </Show>
                <For each={verdicts()}>
                    {(v) => (
                        <span class="warden-manager-dim">
                            {`${byWords(v.by)}: ${v.state.replace(/_/g, " ")}${v.head ? ` · ${v.head.slice(0, 7)}` : ""}${v.stale ? ` · ${tr("kanban.flow.olderCommit")}` : ""}`}
                        </span>
                    )}
                </For>
                <For each={checks()}>
                    {(c) => (
                        <Pill tone={c.tone} icon={c.icon}>
                            {c.word}
                        </Pill>
                    )}
                </For>
            </div>
        </Show>
    );
}

/** "Not followed" with the way out where the drawer can act: the Accounts pane, where GitHub is connected. */
function NotFollowed(props: { act?: PrAct }): JSX.Element {
    return (
        <>
            <Pill tone="warn" title={tr("kanban.prs.notFollowedWhy")}>
                {tr("kanban.prs.notFollowed")}
            </Pill>
            <Show when={props.act}>
                <Button density="compact" tone="quiet" icon="plug" onClick={() => void openConnectors("accounts")}>
                    {tr("kanban.prs.connectGithub")}
                </Button>
            </Show>
        </>
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
                                    <NotFollowed act={props.act} />
                                </Show>
                                <Show when={prAge(p, props.now)}>{(age) => <span class="warden-manager-dim">{age()}</span>}</Show>
                                <Show when={props.act}>{(act) => <UnlinkPr pr={p} act={act()} />}</Show>
                                <PrFlowDetail pr={p} />
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
            <Show when={prs().length === 1 ? prs()[0] : undefined}>{(p) => <PrFlowDetail pr={p()} />}</Show>
            <Show when={prs().length === 1 && prs()[0].followed === false}>
                <NotFollowed act={props.act} />
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
