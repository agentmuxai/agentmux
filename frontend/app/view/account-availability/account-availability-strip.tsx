// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The persistent item for an account episode (docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md §6.2):
// a strip at the top of the Swarm (above its Supervisor, Broadcast and Kanban sections), one row per account whose agents are out of
// quota or signed out, with "Move these agents to another signed-in account" (§6.3). The move asks which
// account, the first signed-in one preselected (§13, decision 2); srv moves the idle agents and queues the rest for the end of their turn.

import { createSignal, For, Show, type Accessor, type JSX } from "solid-js";

import { Button, Select } from "@/app/element/ui";
import {
    episodeLine,
    moveEpisodeAgents,
    moveOutcome,
    preselectedCandidate,
    useAccountEpisodes,
} from "@/app/store/account-availability";
import { notifyLinksChanged } from "@/app/view/identity/account-agents";
import { openConnectors } from "@/app/view/section-pane/panes";
import type { AccountAvailability } from "@/types/rpc/AccountAvailability";

import "./account-availability-strip.scss";

function statusWord(status: string): string {
    if (status === "valid" || status === "ok") return "signed in";
    if (status === "unknown") return "not checked yet";
    return status.replace(/_/g, " ");
}

function EpisodeRow(props: { ep: Accessor<AccountAvailability> }): JSX.Element {
    const [picking, setPicking] = createSignal(false);
    const [to, setTo] = createSignal("");
    const [moving, setMoving] = createSignal(false);
    const [note, setNote] = createSignal<string | null>(null);
    const open = () => {
        setNote(null);
        setTo(preselectedCandidate(props.ep()));
        setPicking(true);
    };
    const move = async () => {
        setMoving(true);
        try {
            const moved = await moveEpisodeAgents(props.ep(), to());
            notifyLinksChanged(); // Connectors' account cards list agents from the links
            setNote(moveOutcome(moved));
            setPicking(false);
        } catch (e) {
            setNote(e instanceof Error ? e.message : String(e));
        } finally {
            setMoving(false);
        }
    };
    return (
        <div class="account-strip__row" data-account-state={props.ep().state}>
            <span class="account-strip__line">
                <i class="fa-solid fa-triangle-exclamation account-strip__icon" aria-hidden="true" />
                {episodeLine(props.ep())}
            </span>
            <Show
                when={picking()}
                fallback={
                    <Show
                        when={props.ep().agents.length > 0}
                        fallback={
                            // A login that expired: its agents moved by themselves, so what is left is signing in again.
                            <Button density="compact" tone="attention" icon="right-to-bracket" onClick={() => void openConnectors("accounts")}>
                                Sign in again
                            </Button>
                        }
                    >
                        <Show
                            when={props.ep().candidates.length > 0}
                            fallback={<span class="account-strip__dim">No other signed-in account to move them to</span>}
                        >
                            <Button density="compact" tone="attention" icon="arrow-right-arrow-left" onClick={open}>
                                Move these agents to another signed-in account
                            </Button>
                        </Show>
                    </Show>
                }
            >
                <span class="account-strip__picker">
                    <Select
                        density="compact"
                        ariaLabel="Account to move the agents to"
                        value={to()}
                        onChange={setTo}
                        options={props.ep().candidates.map((c) => ({
                            value: c.account_id,
                            label: `${c.label} (${statusWord(c.status)})`,
                        }))}
                    />
                    <Button density="compact" tone="accent" busy={moving()} disabled={!to()} onClick={() => void move()}>
                        Move
                    </Button>
                    <Button density="compact" tone="quiet" onClick={() => setPicking(false)}>
                        Cancel
                    </Button>
                </span>
            </Show>
            <Show when={note()}>{(n) => <span class="account-strip__note">{n()}</span>}</Show>
        </div>
    );
}

/** Every open account episode, or nothing. */
export function AccountAvailabilityStrip(): JSX.Element {
    const episodes = useAccountEpisodes();
    const ids = () => episodes().map((e) => e.account_id);
    return (
        <Show when={episodes().length > 0}>
            <div class="account-strip" role="status" aria-label="Accounts your agents can't run on">
                {/* Keyed by account, so a row keeps its open picker when the list refreshes. */}
                <For each={ids()}>
                    {(id) => (
                        <Show when={episodes().find((e) => e.account_id === id)}>{(ep) => <EpisodeRow ep={ep} />}</Show>
                    )}
                </For>
            </div>
        </Show>
    );
}
