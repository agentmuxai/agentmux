// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The explanation under a brand's ways to connect: for each, the plans it works with, what it costs, the key's name
 * and where to make one, and when to pick it, then the vendor's pricing and limits pages. Every line is a fact from
 * provider-guide.ts and says its source on hover; a fact with no source is not drawn.
 */

import { tip } from "@/app/element/tip";
import { Button } from "@/app/element/ui";
import { openLink } from "@/app/store/global";
import { For, Show, type JSX } from "solid-js";
import { shown, sourceTip, type ConnectOption, type Fact, type ProviderGuide } from "./provider-guide";

function Line(props: { label: string; fact: Fact<string> | undefined }): JSX.Element {
    return (
        <Show when={shown(props.fact)}>
            {(f) => (
                <div class="accounts-guide-line">
                    <span class="accounts-guide-label">{props.label}</span>
                    <span class="accounts-guide-value" {...tip(sourceTip(f()))}>
                        {f().value}
                    </span>
                </div>
            )}
        </Show>
    );
}

function KeyLines(props: { option: ConnectOption }): JSX.Element {
    return (
        <Show when={props.option.key}>
            {(k) => (
                <div class="accounts-guide-line">
                    <span class="accounts-guide-label">The key</span>
                    <span class="accounts-guide-value">
                        Called "{k().calledOnVendorSite}" on the vendor's site
                        <Show when={shown(k().prefix)}>
                            {(p) => (
                                <span {...tip(sourceTip(p()))}>
                                    , starting <code>{p().value}</code>
                                </span>
                            )}
                        </Show>
                        .
                        <Show when={shown(k().createAt)}>
                            {(u) => (
                                <Button density="compact" tone="quiet" icon="arrow-up-right-from-square" onClick={() => openLink(u().value)}>
                                    Make one
                                </Button>
                            )}
                        </Show>
                    </span>
                </div>
            )}
        </Show>
    );
}

function OptionDetails(props: { option: ConnectOption }): JSX.Element {
    const o = () => props.option;
    return (
        <section class="accounts-guide-option">
            <h4 class="accounts-guide-title">{o().title}</h4>
            <Line label="What it is" fact={o().what} />
            <Show when={shown(o().plans)}>
                {(p) => (
                    <div class="accounts-guide-line">
                        <span class="accounts-guide-label">Works with</span>
                        <span class="accounts-guide-value" {...tip(sourceTip(p()))}>
                            {p().value.join(", ")}
                        </span>
                    </div>
                )}
            </Show>
            <Line label="Also" fact={o().plansNote} />
            <Line label="Costs" fact={o().billedAgainst} />
            <Show when={o().drives?.length}>
                <div class="accounts-guide-line">
                    <span class="accounts-guide-label">Runs</span>
                    <span class="accounts-guide-value">{o().drives?.join(", ")}</span>
                </div>
            </Show>
            <KeyLines option={o()} />
            <div class="accounts-guide-line">
                <span class="accounts-guide-label">Pick it when</span>
                <span class="accounts-guide-value">{o().pickWhen}</span>
            </div>
        </section>
    );
}

export function ProviderGuideDetails(props: { guide: ProviderGuide }): JSX.Element {
    const pointers = () => props.guide.pointers.filter((p) => shown(p.url));
    return (
        <details class="accounts-guide">
            <summary class="accounts-guide-summary">Which one should I pick?</summary>
            <For each={props.guide.options}>{(o) => <OptionDetails option={o} />}</For>
            <Show when={pointers().length > 0}>
                <div class="accounts-guide-pointers">
                    <For each={pointers()}>
                        {(p) => (
                            <Button
                                density="compact"
                                tone="quiet"
                                icon="arrow-up-right-from-square"
                                onClick={() => openLink(p.url.value)}
                                {...tip(sourceTip(p.url))}
                            >
                                {p.label}
                            </Button>
                        )}
                    </For>
                </div>
            </Show>
        </details>
    );
}

ProviderGuideDetails.displayName = "ProviderGuideDetails";
