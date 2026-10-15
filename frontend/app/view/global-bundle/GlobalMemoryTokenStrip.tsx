// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// GlobalMemoryTokenStrip — above Memory → Global's tiles: what every agent of
// one kind starts with, in tokens, split into Operator Config and Workspace,
// with the Host / Container toggle and the tile order, the largest entries'
// share, and which startup files carry it (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md
// §5.2, §5.5).

import { For, Show, type JSX } from "solid-js";
import { SegmentedControl } from "@/app/element/ui";
import { topContributors, topContributorsLabel } from "@/app/view/native-memory/memory-aggregates";
import { shareLabel, tokensLabel, tokensTitle, TOKEN_ESTIMATE_NOTE } from "@/app/view/native-memory/memory-tokens";
import type { AgentKind, GlobalBundleViewModel } from "./global-bundle-model";

const KINDS: { value: AgentKind; label: string }[] = [
    { value: "host", label: "Host" },
    { value: "container", label: "Container" },
];

type Order = "injection" | "largest";
const ORDERS: { value: Order; label: string }[] = [
    { value: "injection", label: "Injection order" },
    { value: "largest", label: "Largest first" },
];

export function GlobalMemoryTokenStrip(props: { model: GlobalBundleViewModel }): JSX.Element {
    const m = props.model;
    const parts = () => {
        const t = m.deliveredTotalsAtom();
        if (!t) return [];
        return [
            { key: "system", label: "Operator Config", tokens: t.system },
            { key: "workspace", label: "Workspace", tokens: t.workspace },
        ].filter((p) => p.tokens > 0);
    };
    const top = () => topContributors((m.deliveredAtom() ?? []).map((s) => s.tokens_est));
    const files = () => m.filenameGroupsAtom();
    const filesTitle = () =>
        [
            ...files().map((g) => `${g.filename}: ${g.providerNames.join(", ")}`),
            ...(m.noFileProvidersAtom().length ? [`No startup file: ${m.noFileProvidersAtom().join(", ")}`] : []),
        ].join("\n");

    return (
        <div class="global-memory-token-strip" data-testid="global-memory-token-strip">
            <div class="global-memory-token-strip-head">
                <Show
                    when={m.deliveredTotalsAtom()}
                    fallback={<span class="global-memory-token-strip-total">Counting tokens…</span>}
                >
                    {(t) => (
                        <span class="global-memory-token-strip-total" title={tokensTitle(t().total)}>
                            Every {m.agentKindAtom()} agent starts with{" "}
                            <strong>{tokensLabel(t().total)}</strong> of Global Memory
                        </span>
                    )}
                </Show>
                <span class="global-memory-token-strip-toggles">
                    <SegmentedControl
                        density="compact"
                        ariaLabel="Agent kind"
                        options={KINDS}
                        value={m.agentKindAtom()}
                        onChange={(kind) => m.setAgentKind(kind)}
                    />
                    <SegmentedControl
                        density="compact"
                        ariaLabel="Order"
                        options={ORDERS}
                        value={m.largestFirstAtom() ? "largest" : "injection"}
                        onChange={(order) => m.setLargestFirst(order === "largest")}
                    />
                </span>
            </div>
            <Show when={parts().length > 0}>
                <div class="global-memory-token-strip-bar" aria-hidden="true">
                    <For each={parts()}>
                        {(p) => (
                            <span
                                class={`global-memory-token-strip-segment global-memory-token-strip-segment--${p.key}`}
                                style={{ "flex-grow": p.tokens }}
                            />
                        )}
                    </For>
                </div>
                <div class="global-memory-token-strip-legend" title={TOKEN_ESTIMATE_NOTE}>
                    <For each={parts()}>
                        {(p) => (
                            <span class={`global-memory-token-strip-key global-memory-token-strip-key--${p.key}`}>
                                {p.label} {tokensLabel(p.tokens)} ({shareLabel(p.tokens, m.deliveredTotalsAtom()?.total ?? 0)})
                            </span>
                        )}
                    </For>
                    <Show when={top()}>
                        {(t) => (
                            <span class="global-memory-token-strip-top" data-testid="global-memory-token-strip-top">
                                {topContributorsLabel(t(), "entries", "Global Memory")}
                            </span>
                        )}
                    </Show>
                </div>
            </Show>
            <Show when={files().length > 0}>
                <div class="global-memory-token-strip-files" title={filesTitle()} data-testid="global-memory-token-strip-files">
                    Sent in {files().map((g) => g.filename).join(", ")}
                    <Show when={m.noFileProvidersAtom().length > 0}>
                        {" "}· not sent to {m.noFileProvidersAtom().join(", ")} (no startup file)
                    </Show>
                </div>
            </Show>
        </div>
    );
}
