// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { motionAllowed, pulse } from "@/app/motion/primitives";
import { Button } from "@/app/element/ui";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { createEffect, createSignal, For, onCleanup, Show, type Accessor, type JSX } from "solid-js";
import type { Summary } from "./summary";
import { goToSummaryRow } from "./summary-go";
import type { PartId } from "./summary-kinds";
import { PART_ICON, partTip, summaryTip } from "./summary-text";
import { SummaryPopover } from "./SummaryPopover";
import { useSummary, type SummaryStatusProps } from "./use-summary";

/** One short highlight on the Vault part when a request arrives, where the Animations setting allows motion. */
function pulseOnNewVaultRequest(summary: Accessor<Summary>, root: () => HTMLElement | undefined): void {
    let seen: Set<string> | undefined;
    createEffect(() => {
        const keys = new Set(summary().parts.find((p) => p.id === "vault")?.rows.map((r) => r.key));
        const fresh = seen !== undefined && [...keys].some((k) => !seen!.has(k));
        seen = keys;
        const el = root()?.querySelector('[data-part="vault"]');
        if (fresh && el && motionAllowed("feedback")) pulse(el, "flash");
    });
}

/**
 * Everything that waits on the person, as one item: an icon and a number for agents running, Asks, Browser, Vault and
 * Kanban tasks, each hidden while it is zero (docs/specs/SPEC_STATUS_BAR_COMPACT_SUMMARY_AND_VAULT_REQUESTS_2026_11_15.md §2).
 * While anything waits a click opens one list of who, with a button to go to each; with nothing waiting it opens the
 * Swarm. A narrow bar sheds the agents, then the tasks, then all of it but one badge (StatusBar.scss); the item is never
 * dropped while anything waits (`slotDropAt`). The Vault part is operator-only: a widget cannot read or click it.
 */
export function SummaryStatus(props: SummaryStatusProps): JSX.Element {
    const summary = useSummary(props);
    const [open, setOpen] = createSignal<{ first?: PartId } | null>(null);
    let root: HTMLDivElement | undefined;
    // The list is for what waits: with nothing left it is closed for good, so the next request does not open it unasked.
    createEffect(() => {
        if (summary().waiting === 0) setOpen(null);
    });
    pulseOnNewVaultRequest(summary, () => root);
    // A narrow bar can hide the item while its list is open; the list has no anchor then and would jump to a corner.
    const watchAnchor = (el: HTMLDivElement) => {
        root = el;
        if (typeof ResizeObserver === "undefined") return;
        const watch = new ResizeObserver(() => {
            if (el.getClientRects().length === 0) setOpen(null);
        });
        watch.observe(el);
        onCleanup(() => watch.disconnect());
    };
    const openSwarm = () => void openOrFocusPaneByView("swarm");
    const click = (first?: PartId) => (summary().waiting > 0 ? setOpen(open() && open()!.first === first ? null : { first }) : openSwarm());
    const any = () => summary().parts.some((p) => p.count > 0);
    return (
        <Show when={any()}>
            <div ref={watchAnchor} class="status-bar-item status-summary" role="group" aria-label={summaryTip(summary())}>
                <For each={summary().parts}>
                    {(p) => (
                        <Show when={p.count > 0}>
                            <Button
                                variant="chrome"
                                icon={PART_ICON[p.id]}
                                class={p.id === "agents" ? "status-summary__part" : "status-summary__part status-summary__part--waits"}
                                data-part={p.id}
                                data-tip={partTip(p.id, p.count)}
                                aria-label={partTip(p.id, p.count)}
                                aria-haspopup={summary().waiting > 0 ? "dialog" : undefined}
                                onClick={() => click(p.id === "agents" ? undefined : p.id)}
                                data-operator-only={p.id === "vault" ? "" : undefined}
                            >
                                <span class="status-summary__count">{p.count}</span>
                            </Button>
                        </Show>
                    )}
                </For>
                <Show when={summary().waiting > 0}>
                    <Button
                        variant="chrome"
                        icon="bell"
                        class="status-summary__badge"
                        data-tip={summaryTip(summary())}
                        aria-label={summaryTip(summary())}
                        aria-haspopup="dialog"
                        onClick={() => click()}
                    >
                        <span class="status-summary__count">{summary().waiting}</span>
                    </Button>
                </Show>
            </div>
            <Show when={open() && summary().waiting > 0}>
                <SummaryPopover
                    anchor={root}
                    summary={summary}
                    first={open()!.first}
                    onClose={() => setOpen(null)}
                    onOpenSwarm={openSwarm}
                    go={(row) => void goToSummaryRow(row.click)}
                />
            </Show>
        </Show>
    );
}

SummaryStatus.displayName = "SummaryStatus";
