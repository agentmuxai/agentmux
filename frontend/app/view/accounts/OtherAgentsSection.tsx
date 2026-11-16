// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Other agents" under the Connectors tiles (SPEC_CONNECTORS_EVERY_WAY_TO_CONNECT_2026_11_14.md §3, §5): the agents
// that have no tile of their own, one line each on how they sign in or take a key, with their docs link. Nothing here
// connects anything: each agent asks inside its own pane. The lines are facts from provider-guide.ts (each says its
// source on hover); the names and docs links come from the agent catalog.

import { tip } from "@/app/element/tip";
import { Button } from "@/app/element/ui";
import { openLink } from "@/app/store/global";
import { PROVIDERS } from "@/app/view/agent/providers/catalog";
import { ProviderLogo } from "@/element/ProviderLogo";
import { For, Show, type JSX } from "solid-js";
import { OTHER_AGENTS, shown, sourceTip, type AgentKeyPointer, type OtherAgent } from "./provider-guide";

/** The agents of the list the catalog knows, in the list's order. */
export function otherAgentsShown(): OtherAgent[] {
    return OTHER_AGENTS.filter((a) => PROVIDERS[a.id] && shown(a.line));
}

function KeyPointer(props: { pointer: AgentKeyPointer }): JSX.Element {
    const notes = () => props.pointer.notes.map((n) => shown(n)).filter((n) => n !== undefined);
    return (
        <Show when={shown(props.pointer.createAt)}>
            {(url) => (
                <div class="accounts-other-key">
                    <Button density="compact" tone="quiet" icon="key" onClick={() => openLink(url().value)} {...tip(sourceTip(url()))}>
                        {props.pointer.label}
                    </Button>
                    <For each={notes()}>
                        {(n) => (
                            <span class="accounts-other-note" {...tip(sourceTip(n))}>
                                {n.value}
                            </span>
                        )}
                    </For>
                </div>
            )}
        </Show>
    );
}

function AgentRow(props: { agent: OtherAgent }): JSX.Element {
    const def = () => PROVIDERS[props.agent.id];
    return (
        <li class="accounts-other-row" data-agent={props.agent.id}>
            <span class="accounts-other-logo">
                <ProviderLogo provider={props.agent.id} size={16} />
            </span>
            <div class="accounts-other-body">
                <div class="accounts-other-head">
                    <span class="accounts-other-name">{def().displayName}</span>
                    <Button
                        density="compact"
                        tone="quiet"
                        icon="arrow-up-right-from-square"
                        onClick={() => openLink(def().docsUrl)}
                        {...tip(`Docs: ${def().docsUrl}`)}
                    >
                        Docs
                    </Button>
                </div>
                <Show when={shown(props.agent.line)}>
                    {(line) => (
                        <div class="accounts-other-line" {...tip(sourceTip(line()))}>
                            {line().value}
                        </div>
                    )}
                </Show>
                <Show when={props.agent.key}>{(k) => <KeyPointer pointer={k()} />}</Show>
            </div>
        </li>
    );
}

export function OtherAgentsSection(): JSX.Element {
    const agents = otherAgentsShown;
    return (
        <Show when={agents().length > 0}>
            <details class="accounts-other">
                <summary class="accounts-other-summary">Other agents ({agents().length})</summary>
                <p class="accounts-other-intro">These have no tile here: you set each one up inside its own pane.</p>
                <ul class="accounts-other-list">
                    <For each={agents()}>{(a) => <AgentRow agent={a} />}</For>
                </ul>
            </details>
        </Show>
    );
}

OtherAgentsSection.displayName = "OtherAgentsSection";
