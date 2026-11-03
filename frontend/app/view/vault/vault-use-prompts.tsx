// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An agent's use of a Vault entry, as a prompt for the person
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §6.4): which
// entry, which site, which field, and why they are being asked. The answer goes to
// srv the one way it can, through the host. Closing the prompt denies. srv's
// deadline is shown as it counts down
// (SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §4.3).

import { createEffect, createRoot, createSignal, on, onCleanup, Show, type JSX } from "solid-js";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button, Field, TextInput } from "@/app/element/ui";
import { modalsModel, type ModalCloseProps } from "@/app/store/modalmodel";
import type { UseRequest } from "@/types/rpc/UseRequest";
import { answerUse, followVault, holdUse, refreshVaultRequests, vaultAvailable, vaultUses, type UseDecision } from "./vault-store";

/** The host name of an origin, for the "Always on" button. */
export function siteOf(origin: string): string {
    try {
        return new URL(origin).host || origin;
    } catch {
        return origin;
    }
}

/** A grant for commands is stored as `command:<agent>:<part>`; a site's is its origin. */
export const COMMAND_GRANT = "command:";

/** An agent's "Always" for signing in with a saved sign-in is stored as
 *  `signin:<agent>|<profile>|<origin>`: the site and profile the person approved. */
export const SIGNIN_GRANT = "signin:";

/** `signin:<agent>|<profile>|<origin>` as a person reads it: `Agent1 on example.com`. */
export function signinGrantLabel(grant: string): string {
    const [agent, , origin] = grant.slice(SIGNIN_GRANT.length).split("|");
    return origin ? `${agent} on ${siteOf(origin)}` : agent;
}

/** A grant for a site, not one for an agent's commands or sign-ins. */
export function isSiteGrant(grant: string): boolean {
    return !grant.startsWith(COMMAND_GRANT) && !grant.startsWith(SIGNIN_GRANT);
}

/** `command:<agent>:<part>` as a person reads it: `Agent1 (value)`. */
export function commandGrantLabel(grant: string): string {
    const rest = grant.slice(COMMAND_GRANT.length);
    const cut = rest.lastIndexOf(":");
    return cut < 0 ? rest : `${rest.slice(0, cut)} (${rest.slice(cut + 1)})`;
}

/** A grant as a person reads it, in the list of what an agent may do without asking. */
export function grantText(g: string): string {
    if (g.startsWith(COMMAND_GRANT)) return `runs commands: ${commandGrantLabel(g)}`;
    return g.startsWith(SIGNIN_GRANT) ? `signs in: ${signinGrantLabel(g)}` : siteOf(g);
}

/** The prompt's title: who wants to do what. */
function promptTitle(u: UseRequest): string {
    if (u.signin) return `${u.agent} wants to sign in to ${siteOf(u.origin)} as ${u.field}`;
    if (u.replaces) return `${u.agent} wants to replace ${u.entry_label} with what a command prints`;
    if (u.command) return `${u.agent} wants to run a command with your ${u.what}`;
    return u.needs_value ? `${u.agent} needs your ${u.what}` : `${u.agent} wants to fill your ${u.what}`;
}

/** The line under the prompt: what the agent never sees. */
function promptNote(u: UseRequest): string {
    if (u.replaces) return "The agent never sees the new value: the command's output is cleaned of it.";
    if (u.command) return "The agent never sees the value, and the command's output is cleaned of it.";
    if (u.signin) return "The agent never sees the user name or the password. Check the site is the one you expect.";
    return "The agent never sees the value. Check the site is the one you expect.";
}

/** The "Always" button: for the agent, on the site, or both for a sign-in. */
function alwaysLabel(u: UseRequest): string {
    if (u.signin) return `Always for ${u.agent} on ${siteOf(u.origin)}`;
    return u.command ? `Always for ${u.agent}` : `Always on ${siteOf(u.origin)}`;
}

/** Where a fill or a sign-in goes, for a use that runs no command. */
function UseTarget(props: { use: UseRequest }): JSX.Element {
    const u = props.use;
    return (
        <Show
            when={u.signin}
            fallback={
                <p>
                    Into <strong>{u.field}</strong> on <strong>{siteOf(u.origin)}</strong>
                    <span class="warden-manager-dim"> ({u.origin})</span>
                </p>
            }
        >
            <p>
                The user name and password are typed into the sign-in form on <strong>{siteOf(u.origin)}</strong>
                <span class="warden-manager-dim"> ({u.origin})</span> in {u.agent}'s tab.
            </p>
        </Show>
    );
}

/** `m:ss` of `seconds`. */
export function clock(seconds: number): string {
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** How long the person has, from srv's deadline: an API key, sign-in or note
 *  goes ahead once when it runs out; an ID or card entry waits for a yes. A
 *  value only the person can type says nothing: it waits for them anyway. */
function UseDeadline(props: { use: UseRequest }): JSX.Element {
    const [now, setNow] = createSignal(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    onCleanup(() => clearInterval(timer));
    // A hold moves srv's deadline; the list has the latest.
    const live = () => vaultUses().find((x) => x.id === props.use.id) ?? props.use;
    const left = () => Math.max(0, Math.ceil((live().deadline_ms - now()) / 1000));
    return (
        <Show
            when={props.use.goes_ahead}
            fallback={
                <Show when={!props.use.needs_value}>
                    <p class="vault-use-deadline">
                        <strong>Waiting for you</strong>: ID and card details are never used without a yes.
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

/** How often the person's activity in a prompt tells srv to hold its countdown. */
const HOLD_EVERY_MS = 5000;

/** A handler for the person's activity in `u`'s prompt: it holds the countdown
 *  (spec §5, "Hold"), at most every [`HOLD_EVERY_MS`]. */
function holdOnActivity(u: UseRequest): () => void {
    let heldAt = 0;
    return () => {
        if (!u.goes_ahead || Date.now() - heldAt < HOLD_EVERY_MS) return;
        heldAt = Date.now();
        void holdUse(u.id).catch(() => {});
    };
}

function PromptError(props: { error: string | null }): JSX.Element {
    return (
        <Show when={props.error}>
            <div class="warden-manager-error" role="alert">
                {props.error}
            </div>
        </Show>
    );
}

export function UsePrompt(props: { use: UseRequest } & ModalCloseProps): JSX.Element {
    const u = props.use;
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    // When no entry holds the value yet, the person types it here (spec §6.5).
    const [typed, setTyped] = createSignal("");
    const [save, setSave] = createSignal(false);
    const held = holdOnActivity(u);
    // Answered elsewhere (another window), or the agent gave up: this prompt goes.
    createEffect(() => {
        if (!vaultUses().some((x) => x.id === u.id)) props.close();
    });
    const decide = async (decision: UseDecision) => {
        setBusy(true);
        try {
            await answerUse(u.id, decision, u.needs_value && decision !== "deny" ? { value: typed(), save: save() } : undefined);
            // The typed value doesn't outlive the answer.
            setTyped("");
            props.close();
        } catch (e) {
            const message = e instanceof Error ? e.message : String(e);
            // The agent gave up, or another window answered: nothing is waiting any more.
            if (/no longer waiting/i.test(message)) {
                props.close();
                return;
            }
            setError(message);
            setBusy(false);
        }
    };
    return (
        <Modal open={true} scope="window" operatorOnly onClose={() => void decide("deny")} size="sm" closeOnBackdropClick={false} closeOnEscape={!busy()}>
            <ModalHeader title={promptTitle(u)} />
            <ModalBody>
                <div onPointerMove={held} onKeyDown={held} onFocusIn={held}>
                    <UseDeadline use={u} />
                    <Show when={u.command} fallback={<UseTarget use={u} />}>
                        <Show
                            when={u.replaces}
                            fallback={
                                <p>
                                    The command, with <strong>{u.entry_label}</strong> in its environment as <strong class="warden-manager-mono">{u.field}</strong>:
                                </p>
                            }
                        >
                            <p>
                                The command, whose output becomes the {u.what} of <strong>{u.entry_label}</strong>:
                            </p>
                        </Show>
                        <pre class="vault-command">{u.command}</pre>
                        <p class="warden-manager-dim">In {u.cwd || "its default folder"}. A command can do anything its line says, including sending the value somewhere: read it first.</p>
                    </Show>
                    <Show
                        when={u.needs_value}
                        fallback={
                            <p>
                                <Show when={u.replaces} fallback={<>From your Vault entry <strong>{u.entry_label}</strong>. You're asked because: {u.reason}.</>}>
                                    The current value is kept for 7 days, and you can put it back from the Vault pane.
                                </Show>
                            </p>
                        }
                    >
                        <form
                            onSubmit={(e) => {
                                e.preventDefault();
                                if (typed()) void decide("once");
                            }}
                        >
                            <Field label={u.entry_label} layout="stacked" description={`You're asked because ${u.reason}.`}>
                                <TextInput
                                    type="password"
                                    autocomplete="new-password"
                                    spellcheck={false}
                                    autofocus
                                    value={typed()}
                                    onInput={(e) => setTyped(e.currentTarget.value)}
                                    disabled={busy()}
                                />
                            </Field>
                            <label class="vault-check">
                                <input type="checkbox" checked={save()} onChange={(e) => setSave(e.currentTarget.checked)} disabled={busy()} />
                                Save to the Vault as "{u.entry_label}"
                            </label>
                            <input type="submit" hidden />
                        </form>
                    </Show>
                    <p class="warden-manager-dim">{promptNote(u)}</p>
                    <PromptError error={error()} />
                </div>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss tone="danger" disabled={busy()} onClick={() => void decide("deny")}>
                    Deny
                </Button>
                <Button tone={u.needs_value ? "accent" : "neutral"} disabled={busy() || (u.needs_value && !typed())} onClick={() => void decide("once")}>
                    {u.needs_value ? "Fill it in" : "Allow once"}
                </Button>
                <Show when={u.offer_always}>
                    <Button tone="attention" disabled={busy()} onClick={() => void decide("always")}>
                        {alwaysLabel(u)}
                    </Button>
                </Show>
            </ModalFooter>
        </Modal>
    );
}

let started = false;

/** Follow srv's waiting uses and prompt for each new one. Call once. */
export function startVaultUsePrompts(): void {
    if (started || !vaultAvailable()) return;
    started = true;
    followVault();
    const shown = new Set<string>();
    createRoot(() =>
        createEffect(
            on(vaultUses, (list) => {
                const live = new Set(list.map((x) => x.id));
                for (const id of [...shown]) if (!live.has(id)) shown.delete(id);
                for (const u of list) {
                    if (shown.has(u.id)) continue;
                    shown.add(u.id);
                    modalsModel.openModal(UsePrompt, { use: u });
                }
            })
        )
    );
    // Uses that were waiting before this window started: needs no keychain read.
    void refreshVaultRequests();
}
