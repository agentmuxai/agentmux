// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An agent's use of a Vault entry as the question the person answers, with srv's
// countdown: the card in the agent's pane and, for a use with no pane, the
// window's prompt (docs/specs/SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §4).

import { createEffect, createSignal, onCleanup, Show, type Accessor, type Component, type JSX } from "solid-js";

import { tip } from "@/app/element/tip";
import { Button, Field, Select, TextInput, type SelectOption } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import type { UseRequest } from "@/types/rpc/UseRequest";
import { ImportLogin } from "./vault-import-login";
import { CardTyped } from "./vault-use-card";
import { formatWhen } from "./vault-format";
import { answerUse, holdUse, vaultUses, type UseDecision } from "./vault-store";
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

/** The question's title: who wants to do what. `short` cuts the agent's words
 *  (`CLIP`), for where it is shown; the whole title is for its tooltip. */
export function promptTitle(u: UseRequest, short = false): string {
    const agent = short ? clip(u.agent, CLIP.name) : u.agent;
    const label = short ? clip(u.entry_label, CLIP.label) : u.entry_label;
    if (u.import) {
        return t(u.import.replaces ? "vault.use.title.importReplaces" : "vault.use.title.import", { agent, label, from: u.import.service });
    }
    if (u.share) return t(u.prompt ? "vault.shareCard.titlePrompt" : "vault.shareCard.title", { agent, site: siteOf(u.origin), label });
    if (u.account && u.change) return t("vault.use.title.accountChange", { agent, site: siteOf(u.origin) });
    if (u.account) return t("vault.use.title.account", { agent, site: siteOf(u.origin) });
    if (u.signin) return t("vault.use.title.signin", { agent, site: siteOf(u.origin), field: short ? clip(u.field, CLIP.label) : u.field });
    if (u.replaces) return t("vault.use.title.replaces", { agent, label });
    if (u.command) return t("vault.use.title.command", { agent, what: u.what });
    return t(u.needs_value ? "vault.use.title.needsValue" : "vault.use.title.fill", { agent, what: u.what });
}

/** The line under the question: what the agent never sees. */
function promptNote(u: UseRequest): string {
    if (u.import) return t("vault.use.note.import");
    if (u.replaces) return t("vault.use.note.replaces");
    if (u.command) return t("vault.use.note.command");
    if (u.share) return t(u.prompt ? "vault.shareCard.notePrompt" : "vault.shareCard.note");
    if (u.account && u.change) return t("vault.use.note.accountChange");
    if (u.account) return t("vault.use.note.account");
    if (u.signin) return t("vault.use.note.signin");
    return t("vault.use.note.fill");
}

/** The "Always" button: for the agent, on the site, or both for a sign-in (a shared one the browser answers a prompt
 *  with included). */
function alwaysLabel(u: UseRequest): string {
    const agent = clip(u.agent, CLIP.name);
    if (u.signin || u.account || u.share) return t("vault.use.always.agentOnSite", { agent, site: clip(siteOf(u.origin), CLIP.label) });
    // A field in another site's frame is granted as that frame on that page.
    if (u.page) return t("vault.use.always.frame", { page: siteOf(u.page), origin: siteOf(u.origin) });
    return u.command ? t("vault.use.always.agent", { agent }) : t("vault.use.always.site", { site: siteOf(u.origin) });
}

/** Where a fill or a sign-in goes, for a use that runs no command. */
function UseTarget(props: { use: UseRequest }): JSX.Element {
    const u = props.use;
    if (u.share) {
        const params = { field: clip(u.field, CLIP.label), site: siteOf(u.origin), origin: u.origin, agent: clip(u.agent, CLIP.name), realm: clip(u.prompt?.realm ?? "", CLIP.label) };
        // A sign-in the browser answers a prompt with is typed into no form: the card says which prompt.
        const key = !u.prompt ? "vault.shareCard.target" : u.prompt.proxy ? "vault.shareCard.targetProxy" : "vault.shareCard.targetPrompt";
        return (
            <p>
                <Rich k={key} params={params} />
            </p>
        );
    }
    return (
        <Show
            when={u.signin || u.account}
            fallback={
                <p>
                    {t("vault.use.target.into")}{" "}
                    <strong>
                        <Clipped text={u.field} max={CLIP.label} />
                    </strong>{" "}
                    <Show
                        when={u.page}
                        fallback={
                            <>
                                {t("vault.use.target.on")} <strong>{siteOf(u.origin)}</strong>
                                <span class="warden-manager-dim"> ({u.origin})</span>
                            </>
                        }
                    >
                        {t("vault.use.target.frameFrom")} <strong>{siteOf(u.origin)}</strong>
                        <span class="warden-manager-dim"> ({u.origin})</span> {t("vault.use.target.on")} <strong>{siteOf(u.page)}</strong>
                        <span class="warden-manager-dim"> ({u.page})</span>
                    </Show>
                </p>
            }
        >
            <p>
                <Show when={u.account} fallback={<>{t("vault.use.target.signin")}</>}>
                    <Show when={u.change} fallback={<>{t("vault.use.target.signup")}</>}>
                        {t("vault.use.target.change")}
                    </Show>
                </Show>{" "}
                <strong>{siteOf(u.origin)}</strong>
                <span class="warden-manager-dim"> ({u.origin})</span> {t("vault.use.target.in")} <Clipped text={u.agent} max={CLIP.name} />
                {t("vault.use.target.tab")}
            </p>
        </Show>
    );
}

/** The secret an import copies, and where it goes: the secret, its key and region, the Vault, and what happens to a value it replaces. */
function UseImportSubject(props: { use: UseRequest; import: NonNullable<UseRequest["import"]>; answer: UseAnswerState }): JSX.Element {
    const i = props.import;
    return (
        <>
            <p>
                {t("vault.use.import.secret")} <strong class="warden-manager-mono vault-import-id"><ClippedId text={i.secret} max={CLIP.secret} /></strong>
                <Show when={i.key}>
                    {" "}
                    ({t("vault.use.import.key")} <strong class="warden-manager-mono vault-import-id"><ClippedId text={i.key} max={CLIP.label} /></strong>)
                </Show>
                <Show when={i.region}>
                    {" "}
                    {t("vault.use.target.in")} <strong class="warden-manager-mono vault-import-region">{i.region}</strong>
                </Show>{" "}
                {t("vault.use.import.fromService", { service: i.service })}
            </p>
            <p>
                <Show
                    when={i.profile}
                    fallback={t("vault.use.import.defaultLogin")}
                >
                    {i.grant ? t("vault.use.import.readGrant") : t("vault.use.import.readProfile")}{" "}
                    <strong class="warden-manager-mono vault-import-id"><Clipped text={i.profile} max={CLIP.label} /></strong>
                    {i.grant ? "." : ` ${t("vault.use.import.onThisComputer")}.`}
                </Show>
            </p>
            <Show
                when={i.replaces}
                fallback={
                    <>
                        <p>
                            {t("vault.use.import.savedTo")} <strong><Clipped text={i.vault} max={CLIP.label} /></strong> {t("vault.use.import.asNew", { kind: props.use.kind })} <strong><Clipped text={props.use.entry_label} max={CLIP.label} /></strong>.
                        </p>
                        <ImportLogin
                            import={i}
                            names={{ userKey: clip(i.user_key, CLIP.label), site: siteOf(i.site), agent: clip(props.use.agent, CLIP.name) }}
                            share={props.answer.share()}
                            onShare={props.answer.setShare}
                            disabled={props.answer.busy()}
                        />
                    </>
                }
            >
                <p>
                    {t("vault.use.import.replacesValue")} <strong><Clipped text={props.use.entry_label} max={CLIP.label} /></strong> {t("vault.use.target.in")} <strong><Clipped text={i.vault} max={CLIP.label} /></strong>. {t("vault.use.import.oldKept")}
                </p>
            </Show>
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
                        {t("vault.use.command.with")}{" "}
                        <strong>
                            <Clipped text={u.entry_label} max={CLIP.label} />
                        </strong>{" "}
                        {t("vault.use.command.envAs")} <strong class="warden-manager-mono">{u.field}</strong>:
                    </p>
                }
            >
                <p>
                    {t("vault.use.command.output", { what: u.what })}{" "}
                    <strong>
                        <Clipped text={u.entry_label} max={CLIP.label} />
                    </strong>
                    :
                </p>
            </Show>
            <pre class="vault-command">{u.command}</pre>
            <p class="warden-manager-dim">
                {t("vault.use.command.where", { where: u.cwd || t("vault.use.command.defaultFolder") })} {t("vault.use.command.warn")}
            </p>
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
                        <strong>{t("vault.use.deadline.waiting")}</strong>: {props.use.import ? t("vault.use.deadline.copyNeedsYes") : props.use.share ? t("vault.shareCard.needsYes") : t("vault.use.deadline.idNeedsYes")}
                    </p>
                </Show>
            }
        >
            <p class="vault-use-deadline" role="timer">
                <Rich k="vault.use.deadline.goesAhead" params={{ time: clock(left()) }} />
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

/** The person's answer to `u` as it is being given: the decision, and the value
 *  they type when no entry holds it yet (spec §6.5). `done` runs once it is
 *  answered, here or elsewhere (another window, or the agent gave up). */
export function createUseAnswer(u: UseRequest, done: () => void) {
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    const [typed, setTyped] = createSignal("");
    const [save, setSave] = createSignal(false);
    // An import's share, ticked as the card shows it: the person can remove it.
    const [share, setShare] = createSignal(true);
    // Set once the answer is sent: what the person typed is cleared from the form itself, not only from the answer.
    const [answered, setAnswered] = createSignal(false);
    // The card the person picks for a whole-card fill: the agent's own, or the default, to begin with; none otherwise.
    const [card, setCard] = createSignal(u.choices?.some((c) => c.id === u.entry_id) ? u.entry_id : "");
    createEffect(() => {
        if (!vaultUses().some((x) => x.id === u.id)) done();
    });
    const decide = async (decision: UseDecision) => {
        setBusy(true);
        try {
            await answerUse(u.id, decision, u.needs_value && decision !== "deny" ? { value: typed(), save: save() } : undefined, u.choices?.length && decision !== "deny" ? card() : undefined, u.import?.share && decision !== "deny" ? share() : undefined);
            // The typed value doesn't outlive the answer.
            setTyped("");
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
    return { decide, busy, error, typed, setTyped, save, setSave, share, setShare, card, setCard, answered };
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
                why={t("vault.use.askedBecause", { reason: clip(u.reason, CLIP.reason) })}
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
            <Field label={clip(u.entry_label, CLIP.label)} layout="stacked" description={t("vault.use.askedBecause", { reason: clip(u.reason, CLIP.reason) })}>
                <TextInput type="password" autocomplete="new-password" spellcheck={false} autofocus={props.focus} value={a.typed()} onInput={(e) => a.setTyped(e.currentTarget.value)} disabled={a.busy()} />
            </Field>
            <label class="vault-check">
                <input type="checkbox" checked={a.save()} onChange={(e) => a.setSave(e.currentTarget.checked)} disabled={a.busy()} />
                {t("vault.use.saveAs", { label: clip(u.entry_label, CLIP.label) })}
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
        <>
            <Show when={props.use.agent_reason}>
                <p class="vault-use-says">
                    <strong>{clip(props.use.agent, CLIP.name)}</strong> {t("vault.use.says")} <q><Clipped text={props.use.agent_reason} max={CLIP.agent_reason} /></q>
                </p>
            </Show>
            <Show when={props.use.set_by}>
                <p class="vault-use-unseen">{unseenLine(props.use)}</p>
            </Show>
        </>
    );
}

/** What an agent set on this entry and the person has not looked at: the value, the place it is offered, or both. It is the
 *  reason this use waits for a yes and offers no Always. */
export function unseenLine(u: Pick<UseRequest, "set_by" | "set_what" | "set_ms">): string {
    const key = u.set_what === "site" ? "vault.use.unseenSite" : u.set_what === "value and site" ? "vault.use.unseenBoth" : "vault.use.unseenValue";
    return t(key, { agent: u.set_by, when: formatWhen(u.set_ms) });
}

/** The cards the person may pick from for a whole-card fill, and why they are asked. */
function CardPicker(props: { use: UseRequest; answer: UseAnswerState }): JSX.Element {
    const u = props.use;
    const a = props.answer;
    const options = (): SelectOption[] => [
        { value: "", label: t("vault.use.card.choose"), disabled: true },
        ...(u.choices ?? []).map((c) => ({ value: c.id, label: `${clip(c.label, CLIP.label)}: ${c.detail}${c.default ? ` (${t("vault.use.card.default")})` : ""}` })),
    ];
    return (
        <>
            <Field label={t("vault.use.card.label")} layout="stacked" description={t("vault.use.askedBecauseColon", { reason: clip(u.reason, CLIP.reason) })}>
                <Select value={a.card()} onChange={a.setCard} options={options()} ariaLabel={t("vault.use.card.label")} disabled={a.busy()} />
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
                                    {t("vault.use.subject.fromEntry")}{" "}
                                    <strong>
                                        <Clipped text={u.entry_label} max={CLIP.label} />
                                    </strong>
                                    . {t("vault.use.subject.askedBecause")} <Clipped text={u.reason} max={CLIP.reason} />.
                                </>
                            }
                        >
                            {t("vault.use.subject.currentKept")}
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
                <Show
                    when={u.import}
                    fallback={
                        <>
                            <UseDeadline use={u} />
                            <UseSays use={u} />
                            <UseSubject use={u} answer={a} focus={props.focus} />
                        </>
                    }
                >
                    {(i) => (
                        <>
                            <UseImportSubject use={u} import={i()} answer={a} />
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
                <Button data-modal-dismiss tone="danger" disabled={a.busy()} onClick={() => a.decide("deny")}>
                    {t("vault.use.deny")}
                </Button>
                {/* The yeses wrap as one group, so a narrow card never puts Always beside Deny. */}
                <span class="vault-use-allow">
                    <Button tone={u.needs_value ? "accent" : "neutral"} disabled={a.busy() || (u.needs_value && !a.typed()) || (!!u.choices?.length && !a.card())} onClick={() => a.decide("once")}>
                        {u.needs_value ? t("vault.use.fillIn") : u.share ? t("vault.shareCard.allow") : t("vault.use.allowOnce")}
                    </Button>
                    <Show when={u.offer_always && !pickedExpired(u, a)}>
                        <Button tone="attention" disabled={a.busy() || (!!u.choices?.length && !a.card())} onClick={() => a.decide("always")}>
                            {alwaysLabel(u)}
                        </Button>
                    </Show>
                </span>
            </props.Actions>
        </div>
    );
}
