// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An agent's use of a Vault entry as the question the person answers, with srv's
// countdown: the card in the agent's pane and, for a use with no pane, the
// window's prompt (docs/specs/SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §4).

import { createEffect, createRoot, createSignal, For, onCleanup, Show, type Accessor, type Component, type JSX, type Setter } from "solid-js";

import { tip } from "@/app/element/tip";
import { Button, Field, Pill, Select, TextInput, type SelectOption } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultAgents } from "@/types/rpc/VaultAgents";
import { CardTyped } from "./vault-use-card";
import { answerUse, currentVaults, holdUse, vaultUses, type UseDecision } from "./vault-store";
// The question's styles, loaded with it: a card in an agent's pane is drawn
// whether or not a Vault pane was opened.
import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./vault-view.scss";

/** Longest an agent's own words are shown in a question (its name, an entry's
 *  label, its reason), so they can't push the answers out of view or pass for
 *  AgentMux's own wording. The whole text is in the tooltip. */
export const CLIP = { name: 40, label: 60, reason: 240, agent_reason: 140, secret: 90, about: 400 } as const;

/** `text` cut to `max` characters, with an ellipsis when it was longer. */
export function clip(text: string, max: number): string {
    const chars = Array.from(text);
    return chars.length <= max ? text : `${chars.slice(0, max - 1).join("").trimEnd()}…`;
}

/** `text` cut to `max` characters in the middle, keeping its start and its end, with an ellipsis. */
export function clipMiddle(text: string, max: number): string {
    const chars = Array.from(text);
    if (chars.length <= max) return text;
    const head = Math.ceil((max - 1) / 3);
    const tail = max - 1 - head;
    return `${chars.slice(0, head).join("")}…${chars.slice(chars.length - tail).join("")}`;
}

/** A secret's id cut in the middle (its name is at the end), the whole of it in the tooltip when cut. */
export function ClippedId(props: { text: string; max: number }): JSX.Element {
    const shown = () => clipMiddle(props.text, props.max);
    return (
        <span class="vault-clipped" {...tip(shown() === props.text ? undefined : props.text)}>
            {shown()}
        </span>
    );
}

/** An agent's words cut to `max`, the whole of them in the tooltip when cut. */
export function Clipped(props: { text: string; max: number }): JSX.Element {
    const shown = () => clip(props.text, props.max);
    return (
        <span class="vault-clipped" {...tip(shown() === props.text ? undefined : props.text)}>
            {shown()}
        </span>
    );
}

/** The host name of an origin, for the "Always on" button. */
export function siteOf(origin: string): string {
    try {
        return new URL(origin).host || origin;
    } catch {
        return origin;
    }
}

/** Whether every row of a batch is a link: nothing is read, and only where the keys live is recorded. */
function linksOnly(batch: NonNullable<UseRequest["batch"]>): boolean {
    return batch.items.every((item) => item.link);
}

/** The question's title: who wants to do what. `short` cuts the agent's words
 *  (`CLIP`), for where it is shown; the whole title is for its tooltip. */
export function promptTitle(u: UseRequest, short = false): string {
    const agent = short ? clip(u.agent, CLIP.name) : u.agent;
    const label = short ? clip(u.entry_label, CLIP.label) : u.entry_label;
    if (u.batch) {
        const params = { agent, count: u.batch.items.length, service: u.batch.service, name: short ? clip(u.batch.name, CLIP.label) : u.batch.name };
        return linksOnly(u.batch) ? t("vault.batchCard.titleLinks", params) : t("vault.batchCard.title", params);
    }
    if (u.import) {
        const from = u.import.service;
        return u.import.replaces ? `${agent} wants to replace the value of ${label} with a secret from ${from}` : `${agent} wants to copy a secret from ${from} into your Vault as ${label}`;
    }
    if (u.link) {
        // A link holds no value, so its place is where the key itself lives; and a change that leaves the place alone
        // is to what the entry says of the key (who reads it, when it last changed, where else it is kept).
        if (u.link.only || u.link.from === u.link.to) return t("vault.linkCard.titleAbout", { agent, label });
        if (!u.link.from) return `${agent} wants to record where the machine copy of ${label} is`;
        return u.link.to ? `${agent} wants to change where the machine copy of ${label} is` : `${agent} wants to remove where the machine copy of ${label} is`;
    }
    if (u.account && u.change) return `${agent} wants to change your password on ${siteOf(u.origin)}`;
    if (u.account) return `${agent} wants to create an account on ${siteOf(u.origin)}`;
    if (u.signin) return `${agent} wants to sign in to ${siteOf(u.origin)} as ${short ? clip(u.field, CLIP.label) : u.field}`;
    if (u.replaces) return `${agent} wants to replace ${label} with what a command prints`;
    if (u.command) return `${agent} wants to run a command with your ${u.what}`;
    return u.needs_value ? `${agent} needs your ${u.what}` : `${agent} wants to fill your ${u.what}`;
}

/** The line under the question: what the agent never sees. */
function promptNote(u: UseRequest): string {
    if (u.batch) return linksOnly(u.batch) ? t("vault.batchCard.noteLinks") : t("vault.batchCard.note", { count: u.batch.items.length });
    if (u.import) return "The agent never sees the value: AgentMux reads it.";
    if (u.link?.only) return t("vault.linkCard.noteLinked");
    if (u.link) return "The place decides what a check of this entry reads. No value is in it, and the agent never sees one.";
    if (u.replaces) return "The agent never sees the new value: the command's output is cleaned of it.";
    if (u.command) return "The agent never sees the value, and the command's output is cleaned of it.";
    if (u.account && u.change) return "The Vault makes the new password. The agent never sees it. The old one is kept for 7 days, and you can put it back. Check the site is the one you expect.";
    if (u.account) return "The Vault makes the password and keeps it for you. The agent never sees it. Check the site is the one you expect.";
    if (u.signin) return "The agent never sees the user name or the password. Check the site is the one you expect.";
    return "The agent never sees the value. Check the site is the one you expect.";
}

/** The "Always" button: for the agent, on the site, or both for a sign-in. */
function alwaysLabel(u: UseRequest): string {
    const agent = clip(u.agent, CLIP.name);
    if (u.signin || u.account) return `Always for ${agent} on ${clip(siteOf(u.origin), CLIP.label)}`;
    // A field in another site's frame is granted as that frame on that page.
    if (u.page) return `Always on ${siteOf(u.page)} (${siteOf(u.origin)} fields)`;
    return u.command ? `Always for ${agent}` : `Always on ${siteOf(u.origin)}`;
}

/** Where a fill or a sign-in goes, for a use that runs no command. */
function UseTarget(props: { use: UseRequest }): JSX.Element {
    const u = props.use;
    return (
        <Show
            when={u.signin || u.account}
            fallback={
                <p>
                    Into{" "}
                    <strong>
                        <Clipped text={u.field} max={CLIP.label} />
                    </strong>{" "}
                    <Show
                        when={u.page}
                        fallback={
                            <>
                                on <strong>{siteOf(u.origin)}</strong>
                                <span class="warden-manager-dim"> ({u.origin})</span>
                            </>
                        }
                    >
                        in a frame from <strong>{siteOf(u.origin)}</strong>
                        <span class="warden-manager-dim"> ({u.origin})</span> on <strong>{siteOf(u.page)}</strong>
                        <span class="warden-manager-dim"> ({u.page})</span>
                    </Show>
                </p>
            }
        >
            <p>
                <Show when={u.account} fallback={<>The user name and password are typed into the sign-in form on</>}>
                    <Show when={u.change} fallback={<>A password the Vault makes is kept in your Vault and typed into the sign-up form on</>}>
                        A new password the Vault makes is typed into the password-change form on
                    </Show>
                </Show>{" "}
                <strong>{siteOf(u.origin)}</strong>
                <span class="warden-manager-dim"> ({u.origin})</span> in <Clipped text={u.agent} max={CLIP.name} />
                's tab.
            </p>
        </Show>
    );
}

/** A line of what an entry says of its key, and how the change asked for touches it. */
export type SaidLine = { text: string; mark?: "changed" | "added" | "removed" };

const SAID_MARKS = { changed: "vault.linkCard.changed", added: "vault.linkCard.added", removed: "vault.linkCard.removed" } as const;

/** What an entry says of its key, one thing a line ("Read by: …", "Last changed: …", "Also kept at: …"), as srv
 *  writes it. With `against` (what is recorded now) each line of what is asked for is marked when it is new or
 *  differs, and a thing the change takes away is a line of its own: the difference is said in a word, not left
 *  for the eye to find in two long lines. */
export function saidLines(text: string, against?: string): SaidLine[] {
    const split = (said: string) => said.split("\n").filter(Boolean).map((line) => ({ what: line.split(": ")[0], text: line }));
    const lines = split(text);
    if (against === undefined) return lines.map(({ text }) => ({ text }));
    const before = split(against);
    const asked = lines.map(({ what, text }): SaidLine => {
        const was = before.find((b) => b.what === what);
        return was?.text === text ? { text } : { text, mark: was ? "changed" : "added" };
    });
    const gone = before.filter((b) => !lines.some((line) => line.what === b.what)).map((b): SaidLine => ({ text: b.what, mark: "removed" }));
    return [...asked, ...gone];
}

function SaidLines(props: { name: string; text: string; against?: string }): JSX.Element {
    return (
        <span data-link={props.name}>
            <For each={saidLines(props.text, props.against)}>
                {(line) => (
                    <span class="vault-link-said-line" data-said={line.mark ?? "same"}>
                        <Show when={line.mark}>{(mark) => <Pill tone="accent">{t(SAID_MARKS[mark()])}</Pill>}</Show> <Clipped text={line.text} max={CLIP.about} />
                    </span>
                )}
            </For>
        </span>
    );
}

/** A change to where an entry's machine copy is: the place it has now and the place the agent asks for. */
function UseLinkSubject(props: { use: UseRequest; link: NonNullable<UseRequest["link"]> }): JSX.Element {
    return (
        <>
            <p {...tip(clip(props.use.entry_label, CLIP.label) === props.use.entry_label ? undefined : props.use.entry_label)}>
                <Rich k={props.link.only ? "vault.linkCard.whatLinked" : "vault.linkCard.what"} params={{ label: clip(props.use.entry_label, CLIP.label) }} />
            </p>
            <p data-link="from">
                {t("vault.linkCard.now")} <Show when={props.link.from} fallback={t("vault.linkCard.noPlaceNow")}>
                    <strong class="warden-manager-mono vault-import-id"><ClippedId text={props.link.from} max={CLIP.reason} /></strong>
                </Show>
                <Show when={props.link.about_from}>
                    <SaidLines name="about-from" text={props.link.about_from} />
                </Show>
            </p>
            <p data-link="to">
                {t("vault.linkCard.askedFor")} <Show when={props.link.to} fallback={t("vault.linkCard.noPlaceAsked")}>
                    <strong class="warden-manager-mono vault-import-id"><ClippedId text={props.link.to} max={CLIP.reason} /></strong>
                </Show>
                <Show when={props.link.about_to || props.link.about_from}>
                    <SaidLines name="about-to" text={props.link.about_to} against={props.link.about_from} />
                </Show>
            </p>
        </>
    );
}

/** The secret an import copies, and where it goes: the secret, its key and region, the Vault, and what happens to a value it replaces. */
function UseImportSubject(props: { use: UseRequest; import: NonNullable<UseRequest["import"]> }): JSX.Element {
    const i = props.import;
    return (
        <>
            <p>
                The secret <strong class="warden-manager-mono vault-import-id"><ClippedId text={i.secret} max={CLIP.secret} /></strong>
                <Show when={i.key}>
                    {" "}
                    (key <strong class="warden-manager-mono vault-import-id"><ClippedId text={i.key} max={CLIP.label} /></strong>)
                </Show>
                <Show when={i.region}>
                    {" "}
                    in <strong class="warden-manager-mono vault-import-region">{i.region}</strong>
                </Show>{" "}
                from {i.service}.
            </p>
            <p>
                <Show
                    when={i.profile}
                    fallback="It is read with this computer's default AWS login."
                >
                    {i.grant ? "It is read under the AWS grant " : "It is read with the AWS profile "}
                    <strong class="warden-manager-mono vault-import-id"><Clipped text={i.profile} max={CLIP.label} /></strong>
                    {i.grant ? "." : " on this computer."}
                </Show>
            </p>
            <Show
                when={i.replaces}
                fallback={
                    <p>
                        It is saved to the Vault <strong><Clipped text={i.vault} max={CLIP.label} /></strong> as a new {props.use.kind} entry, <strong><Clipped text={props.use.entry_label} max={CLIP.label} /></strong>.
                    </p>
                }
            >
                <p>
                    It replaces the value of <strong><Clipped text={props.use.entry_label} max={CLIP.label} /></strong> in <strong><Clipped text={i.vault} max={CLIP.label} /></strong>. The old value is kept for 7 days, and you can put it back from the Vault pane.
                </p>
            </Show>
        </>
    );
}

/** The most agents a batch's card names before it counts the rest: a long list pushed the rows out of a narrow card. */
const NAMED_AGENTS = 3;

/** Who may use what is in a Vault, as the batch card says it. `whole` is every name, when the line counts some. */
function vaultOpenTo(agents: VaultAgents): { text: string; whole?: string } {
    if (agents.mode === "all") return { text: t("vault.batchCard.openToAll") };
    if (agents.mode === "none" || agents.agents.length === 0) return { text: t("vault.batchCard.openToNone") };
    const names = agents.agents.map((name) => clip(name, CLIP.name));
    if (names.length <= NAMED_AGENTS) return { text: t("vault.batchCard.openToNamed", { agents: names.join(", ") }) };
    const shown = names.slice(0, NAMED_AGENTS).join(", ");
    return { text: t("vault.batchCard.openToNamedMore", { agents: shown, count: names.length - NAMED_AGENTS }), whole: agents.agents.join(", ") };
}

/** A batch of imports: the one account, region and Vault, and a row for every secret with its tick. A row left
 *  unticked is not read. Every row is shown; a long list scrolls. */
function UseBatchSubject(props: { batch: NonNullable<UseRequest["batch"]>; answer: UseAnswerState }): JSX.Element {
    const b = props.batch;
    const a = props.answer;
    const links = () => linksOnly(b);
    // "The key is", "It is read": one row is said as one.
    const count = b.items.length;
    // The Vault the rows go into as the card stands: the one asked for, or the one the person chose. Who may use it
    // is read from the person's Vaults while that list is current, so a change made in the Vaults dialog shows on a
    // card that is already open; otherwise it is what srv said when it asked.
    const into = () => {
        const asked = b.vaults.find((v) => v.name === a.vault()) ?? { name: b.vault, agents: b.agents };
        return { name: asked.name, agents: currentVaults()?.find((v) => v.name === asked.name)?.agents ?? asked.agents };
    };
    const openTo = () => vaultOpenTo(into().agents);
    // A batch of both kinds: how many of the ticked rows are read and copied, and how many are only recorded.
    const mixed = () => !links() && b.items.some((item) => item.link);
    const ticked = () => {
        const recorded = a.rows().filter((i) => b.items[i]?.link).length;
        return { read: a.rows().length - recorded, links: recorded };
    };
    const offered = (): SelectOption[] => b.vaults.map((v) => ({ value: v.name, label: clip(v.name, CLIP.label) }));
    return (
        <>
            <p data-batch="source">
                <Show when={b.region} fallback={t(links() ? "vault.batchCard.linksAtDefaultRegion" : "vault.batchCard.fromDefaultRegion", { service: b.service, count })}>
                    <Rich k={links() ? "vault.batchCard.linksAt" : "vault.batchCard.from"} params={{ service: b.service, region: b.region, count }} />
                </Show>{" "}
                <Show when={!links()}>
                    <span {...tip(clip(b.profile, CLIP.label) === b.profile ? undefined : b.profile)}>
                        <Show when={b.profile} fallback={t("vault.batchCard.defaultLogin", { count })}>
                            <Show when={b.grant} fallback={<Rich k="vault.batchCard.profile" params={{ profile: clip(b.profile, CLIP.label), count }} />}>
                                <Rich k="vault.batchCard.grant" params={{ profile: clip(b.profile, CLIP.label), count }} />
                            </Show>
                        </Show>
                    </span>
                </Show>
            </p>
            <p data-batch="vault" {...tip(clip(into().name, CLIP.label) === into().name ? undefined : into().name)}>
                <Rich k={links() ? "vault.batchCard.linksInto" : "vault.batchCard.into"} params={{ vault: clip(into().name, CLIP.label) }} />{" "}
                <span data-batch="agents" {...tip(openTo().whole)}>
                    {openTo().text}
                </span>
            </p>
            <Show when={b.vaults.length > 1}>
                <Field label={t("vault.batchCard.chooseVault")} layout="stacked" class="vault-batch-vault">
                    <Select value={a.vault()} onChange={a.setVault} options={offered()} ariaLabel={t("vault.batchCard.chooseVault")} disabled={a.busy()} />
                </Field>
            </Show>
            <Show when={mixed()}>
                <p data-batch="mixed">{t("vault.batchCard.mixed", ticked())}</p>
            </Show>
            <ul class="vault-batch-rows" aria-label={t("vault.batchCard.rows")}>
                <For each={b.items}>
                    {(item, i) => (
                        <li>
                            <label class="vault-batch-row">
                                <input type="checkbox" checked={a.rows().includes(i())} onChange={(e) => a.tickRow(i(), e.currentTarget.checked)} disabled={a.busy()} />
                                <span class="vault-batch-entry">
                                    <strong>
                                        <Clipped text={item.label} max={CLIP.label} />
                                    </strong>
                                    <span class="warden-manager-dim" data-row={item.link ? "link" : "copy"}>
                                        {item.link ? t("vault.batchCard.link") : item.replaces ? t("vault.batchCard.replaces") : t("vault.batchCard.new", { kind: item.kind })}
                                    </span>
                                </span>
                                <span class="vault-batch-secret warden-manager-mono warden-manager-dim vault-import-id">
                                    <ClippedId text={item.secret} max={CLIP.secret} />
                                    <Show when={item.key}>
                                        {" "}
                                        ({t("vault.batchCard.key")} <ClippedId text={item.key} max={CLIP.label} />)
                                    </Show>
                                </span>
                            </label>
                        </li>
                    )}
                </For>
            </ul>
        </>
    );
}

/** The command a use runs, and where. */
function UseCommand(props: { use: UseRequest }): JSX.Element {
    const u = props.use;
    return (
        <>
            <Show
                when={u.replaces}
                fallback={
                    <p>
                        The command, with{" "}
                        <strong>
                            <Clipped text={u.entry_label} max={CLIP.label} />
                        </strong>{" "}
                        in its environment as <strong class="warden-manager-mono">{u.field}</strong>:
                    </p>
                }
            >
                <p>
                    The command, whose output becomes the {u.what} of{" "}
                    <strong>
                        <Clipped text={u.entry_label} max={CLIP.label} />
                    </strong>
                    :
                </p>
            </Show>
            <pre class="vault-command">{u.command}</pre>
            <p class="warden-manager-dim">In {u.cwd || "its default folder"}. A command can do anything its line says, including sending the value somewhere: read it first.</p>
        </>
    );
}

/** `m:ss` of `seconds`. */
export function clock(seconds: number): string {
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** Seconds left before srv decides `u` on silence, counting down; a hold moves
 *  srv's deadline, and the list has the latest. */
export function secondsLeft(u: UseRequest): Accessor<number> {
    const [now, setNow] = createSignal(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    onCleanup(() => clearInterval(timer));
    const live = () => vaultUses().find((x) => x.id === u.id) ?? u;
    return () => Math.max(0, Math.ceil((live().deadline_ms - now()) / 1000));
}

/** How long the person has, from srv's deadline: an API key, sign-in or note
 *  goes ahead once when it runs out; an ID or card entry waits for a yes. A
 *  value only the person can type says nothing: it waits for them anyway. */
function UseDeadline(props: { use: UseRequest }): JSX.Element {
    const left = secondsLeft(props.use);
    return (
        <Show
            when={props.use.goes_ahead}
            fallback={
                <Show when={!props.use.needs_value}>
                    <p class="vault-use-deadline">
                        <strong>Waiting for you</strong>: {props.use.batch && linksOnly(props.use.batch) ? t("vault.batchCard.needsYesLinks") : props.use.import || props.use.batch ? "nothing is copied without a yes." : props.use.link ? t(props.use.link.only || props.use.link.from === props.use.link.to ? "vault.linkCard.needsYesAbout" : "vault.linkCard.needsYes") : "ID and card details are never used without a yes."}
                    </p>
                </Show>
            }
        >
            <p class="vault-use-deadline" role="timer">
                <strong>Goes ahead in {clock(left())}</strong> unless you deny.
            </p>
        </Show>
    );
}

/** How often the person's activity in a question tells srv to hold its countdown. */
const HOLD_EVERY_MS = 5000;

/** A handler for the person's activity anywhere in `u`'s question, its answers
 *  included: it holds the countdown (spec §5, "Hold"), at most every [`HOLD_EVERY_MS`]. */
function holdOnActivity(u: UseRequest): () => void {
    let heldAt = 0;
    return () => {
        if (!u.goes_ahead || Date.now() - heldAt < HOLD_EVERY_MS) return;
        heldAt = Date.now();
        void holdUse(u.id).catch(() => {});
    };
}

/** What the person has chosen on a batch's card so far: the rows left ticked, by their place in its list, and the
 *  Vault they go into. */
interface BatchChoice {
    rows: Accessor<number[]>;
    setRows: Setter<number[]>;
    vault: Accessor<string>;
    setVault: Setter<string>;
}

/** Every row ticked, and the Vault that was asked for: how a batch's card starts. */
function newBatchChoice(u: UseRequest): BatchChoice {
    const [rows, setRows] = createSignal<number[]>(u.batch ? u.batch.items.map((_, i) => i) : []);
    const [vault, setVault] = createSignal(u.batch?.vault ?? "");
    return { rows, setRows, vault, setVault };
}

// A batch's card is shown in the agent's pane and, while the Vault pane is open, there too. Its choice is kept once
// for the use, so both copies show the same thing and either Allow sends what the person sees.
const batchChoices = new Map<string, BatchChoice>();
let forgetsBatchChoices = false;

/** The one choice of the batch `u`, shared by every copy of its card; a use that is no batch has none to share. */
function batchChoiceOf(u: UseRequest): BatchChoice {
    if (!u.batch) return newBatchChoice(u);
    if (!forgetsBatchChoices) {
        forgetsBatchChoices = true;
        // A use that was answered, or given up, takes its choice with it.
        createRoot(() =>
            createEffect(() => {
                const waiting = new Set(vaultUses().map((x) => x.id));
                for (const id of [...batchChoices.keys()]) if (!waiting.has(id)) batchChoices.delete(id);
            })
        );
    }
    let choice = batchChoices.get(u.id);
    if (!choice) {
        // Owned by no card, so it outlives the copy that made it.
        choice = createRoot(() => newBatchChoice(u));
        batchChoices.set(u.id, choice);
    }
    return choice;
}

/** The person's answer to `u` as it is being given: the decision, and the value
 *  they type when no entry holds it yet (spec §6.5). `done` runs once it is
 *  answered, here or elsewhere (another window, or the agent gave up). */
export function createUseAnswer(u: UseRequest, done: () => void) {
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    const [typed, setTyped] = createSignal("");
    const [save, setSave] = createSignal(false);
    // Set once the answer is sent: what the person typed is cleared from the form itself, not only from the answer.
    const [answered, setAnswered] = createSignal(false);
    // The card the person picks for a whole-card fill: the agent's own, or the default, to begin with; none otherwise.
    const [card, setCard] = createSignal(u.choices?.some((c) => c.id === u.entry_id) ? u.entry_id : "");
    // The rows of a batch the person leaves ticked (all of them to begin with) and the Vault they go into (the one
    // asked for, until the person chooses another the card offers): one choice for every copy of the card.
    const { rows, setRows, vault, setVault } = batchChoiceOf(u);
    const tickRow = (row: number, on: boolean) => setRows((was) => (on ? [...was.filter((r) => r !== row), row].sort((x, y) => x - y) : was.filter((r) => r !== row)));
    createEffect(() => {
        if (!vaultUses().some((x) => x.id === u.id)) done();
    });
    const decide = async (decision: UseDecision) => {
        setBusy(true);
        try {
            await answerUse(u.id, decision, u.needs_value && decision !== "deny" ? { value: typed(), save: save() } : undefined, u.choices?.length && decision !== "deny" ? card() : undefined, u.batch && decision !== "deny" ? rows() : undefined, u.batch && decision !== "deny" && vault() !== u.batch.vault ? vault() : undefined);
            // The typed value doesn't outlive the answer, and neither does what was chosen on a batch's card.
            setTyped("");
            batchChoices.delete(u.id);
            setAnswered(true);
            done();
        } catch (e) {
            const message = e instanceof Error ? e.message : String(e);
            // The agent gave up, or another window answered: nothing is waiting any more.
            if (/no longer waiting/i.test(message)) {
                done();
                return;
            }
            setError(message);
            setBusy(false);
        }
    };
    return { decide, busy, error, typed, setTyped, save, setSave, card, setCard, answered, rows, tickRow, vault, setVault };
}

export type UseAnswerState = ReturnType<typeof createUseAnswer>;

/** The value the person types for a use with no entry yet, and whether to keep it. */
function TypedValue(props: { use: UseRequest; answer: UseAnswerState; focus: boolean }): JSX.Element {
    const u = props.use;
    const a = props.answer;
    // A card is typed whole, in the person's own form: number, expiry, code, name and ZIP.
    if (u.kind === "card") {
        return (
            <CardTyped
                label={clip(u.entry_label, CLIP.label)}
                why={`You're asked because ${clip(u.reason, CLIP.reason)}.`}
                busy={a.busy()}
                answered={a.answered()}
                focus={props.focus}
                save={a.save()}
                onSave={a.setSave}
                onText={a.setTyped}
                onSubmit={() => {
                    if (a.typed()) void a.decide("once");
                }}
            />
        );
    }
    return (
        <form
            onSubmit={(e) => {
                e.preventDefault();
                if (a.typed()) void a.decide("once");
            }}
        >
            <Field label={clip(u.entry_label, CLIP.label)} layout="stacked" description={`You're asked because ${clip(u.reason, CLIP.reason)}.`}>
                <TextInput type="password" autocomplete="new-password" spellcheck={false} autofocus={props.focus} value={a.typed()} onInput={(e) => a.setTyped(e.currentTarget.value)} disabled={a.busy()} />
            </Field>
            <label class="vault-check">
                <input type="checkbox" checked={a.save()} onChange={(e) => a.setSave(e.currentTarget.checked)} disabled={a.busy()} />
                Save to the Vault as "{clip(u.entry_label, CLIP.label)}"
            </label>
            <input type="submit" hidden />
        </form>
    );
}

/** What the agent says it wants this for: quoted and attributed, never as the person's words, and it decides nothing.
 *  Cut to `CLIP.agent_reason` (the whole in the tooltip) so the command the person must read stays in view; the
 *  entry's History has all of it. */
function UseSays(props: { use: UseRequest }): JSX.Element {
    return (
        <Show when={props.use.agent_reason}>
            <p class="vault-use-says">
                <strong>{clip(props.use.agent, CLIP.name)}</strong> says: <q><Clipped text={props.use.agent_reason} max={CLIP.agent_reason} /></q>
            </p>
        </Show>
    );
}

/** The cards the person may pick from for a whole-card fill, and why they are asked. */
function CardPicker(props: { use: UseRequest; answer: UseAnswerState }): JSX.Element {
    const u = props.use;
    const a = props.answer;
    const options = (): SelectOption[] => [
        { value: "", label: "Choose a card…", disabled: true },
        ...(u.choices ?? []).map((c) => ({ value: c.id, label: `${clip(c.label, CLIP.label)}: ${c.detail}${c.default ? " (default)" : ""}` })),
    ];
    return (
        <>
            <Field label="Card" layout="stacked" description={`You're asked because: ${clip(u.reason, CLIP.reason)}.`}>
                <Select value={a.card()} onChange={a.setCard} options={options()} ariaLabel="Card" disabled={a.busy()} />
            </Field>
        </>
    );
}

/** Whether the card picked is past its month: "Always" is never for it. */
function pickedExpired(u: UseRequest, a: UseAnswerState): boolean {
    return !!u.choices?.find((c) => c.id === a.card())?.expired;
}

/** What a fill, sign-in, command or capture is about: where it goes, and whose value. */
function UseSubject(props: { use: UseRequest; answer: UseAnswerState; focus: boolean }): JSX.Element {
    const u = props.use;
    return (
        <>
            <Show when={u.command} fallback={<UseTarget use={u} />}>
                <UseCommand use={u} />
            </Show>
            <Show
                when={u.needs_value}
                fallback={
                    <Show
                        when={u.choices?.length}
                        fallback={
                    <p>
                        <Show
                            when={u.replaces}
                            fallback={
                                <>
                                    From your Vault entry{" "}
                                    <strong>
                                        <Clipped text={u.entry_label} max={CLIP.label} />
                                    </strong>
                                    . You're asked because: <Clipped text={u.reason} max={CLIP.reason} />.
                                </>
                            }
                        >
                            The current value is kept for 7 days, and you can put it back from the Vault pane.
                        </Show>
                    </p>
                        }
                    >
                        <CardPicker use={u} answer={props.answer} />
                    </Show>
                }
            >
                <TypedValue use={u} answer={props.answer} focus={props.focus} />
            </Show>
        </>
    );
}

/** The question and its answers, laid out by `Body` and `Actions` (the modal's
 *  parts, or the card's). `focus` lets a typed value's field take the focus: the
 *  window's prompt may, a card in the agent's pane never does (spec §4.4). */
export function UseQuestion(props: { use: UseRequest; answer: UseAnswerState; Body: Component<{ children: JSX.Element }>; Actions: Component<{ children: JSX.Element }>; focus: boolean }): JSX.Element {
    const u = props.use;
    const a = props.answer;
    const held = holdOnActivity(u);
    return (
        <div class="vault-use-activity" onPointerMove={held} onKeyDown={held} onFocusIn={held}>
            <props.Body>
                <Show when={u.batch}>
                    {(b) => (
                        <>
                            <UseBatchSubject batch={b()} answer={a} />
                            <UseSays use={u} />
                            <UseDeadline use={u} />
                        </>
                    )}
                </Show>
                <Show
                    when={u.import}
                    fallback={
                        <Show
                            when={u.link}
                            fallback={
                                <Show when={!u.batch}>
                                    <UseDeadline use={u} />
                                    <UseSays use={u} />
                                    <UseSubject use={u} answer={a} focus={props.focus} />
                                </Show>
                            }
                        >
                            {(l) => (
                                <>
                                    <UseLinkSubject use={u} link={l()} />
                                    <UseSays use={u} />
                                    <UseDeadline use={u} />
                                </>
                            )}
                        </Show>
                    }
                >
                    {(i) => (
                        <>
                            <UseImportSubject use={u} import={i()} />
                            <UseSays use={u} />
                            <UseDeadline use={u} />
                        </>
                    )}
                </Show>
                <p class="warden-manager-dim">{promptNote(u)}</p>
                <Show when={a.error()}>
                    <div class="warden-manager-error" role="alert">
                        {a.error()}
                    </div>
                </Show>
            </props.Body>
            <props.Actions>
                <Button data-modal-dismiss tone="danger" disabled={a.busy()} onClick={() => void a.decide("deny")}>
                    Deny
                </Button>
                {/* The yeses wrap as one group, so a narrow card never puts Always beside Deny. */}
                <span class="vault-use-allow">
                    <Button tone={u.needs_value ? "accent" : "neutral"} disabled={a.busy() || (u.needs_value && !a.typed()) || (!!u.choices?.length && !a.card()) || (!!u.batch && a.rows().length === 0)} onClick={() => void a.decide("once")}>
                        {u.batch ? t("vault.batchCard.allow", { count: a.rows().length }) : u.needs_value ? "Fill it in" : "Allow once"}
                    </Button>
                    <Show when={u.offer_always && !pickedExpired(u, a)}>
                        <Button tone="attention" disabled={a.busy() || (!!u.choices?.length && !a.card())} onClick={() => void a.decide("always")}>
                            {alwaysLabel(u)}
                        </Button>
                    </Show>
                </span>
            </props.Actions>
        </div>
    );
}
