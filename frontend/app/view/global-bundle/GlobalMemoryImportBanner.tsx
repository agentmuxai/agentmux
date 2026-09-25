// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Bring Global Memory from…" — shown in Armory → Memory → Global on an
 * isolated channel (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.6).
 *
 * An isolated channel keeps its own Global Memory, so Armory testing there
 * can't touch the real one. That also means it starts without the entries
 * people added elsewhere; this offers them, from the other scopes' records,
 * and adds only what this channel lacks. Nothing is shared silently.
 *
 * Renders nothing on a shared channel, or when there's nothing to bring.
 */

import { createSignal, For, onMount, Show, type JSX } from "solid-js";
import { RpcApi, type GlobalMemoryImportSources } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

/** How a scope is named to the human. */
export function scopeLabel(scope: string): string {
    if (scope === "shared") return "your main Global Memory";
    if (scope.startsWith("channel:")) return `the "${scope.slice("channel:".length)}" channel`;
    return scope;
}

export const GlobalMemoryImportBanner = (props: { onImported?: () => void }): JSX.Element => {
    const [offer, setOffer] = createSignal<GlobalMemoryImportSources | null>(null);
    const [busy, setBusy] = createSignal<number | null>(null);
    const [message, setMessage] = createSignal<string | null>(null);

    const load = () =>
        RpcApi.GlobalMemoryImportSourcesCommand(TabRpcClient)
            .then(setOffer)
            .catch(() => setOffer(null));
    onMount(() => void load());

    const bring = async (index: number) => {
        const o = offer();
        if (!o || busy() !== null) return;
        setBusy(index);
        setMessage(null);
        try {
            const r = await RpcApi.GlobalMemoryImportCommand(TabRpcClient, { list_id: o.list_id, index });
            setMessage(
                `Added ${r.added} entr${r.added === 1 ? "y" : "ies"}` +
                    (r.renamed ? ` (${r.renamed} renamed: an entry here already had that name)` : "") +
                    ". Agents get them at their next launch.",
            );
            props.onImported?.();
        } catch (e) {
            setMessage(`Couldn't import: ${e}`);
        } finally {
            setBusy(null);
            void load();
        }
    };

    return (
        <Show when={(offer()?.sources.length ?? 0) > 0 || message()}>
            <section class="global-memory-import" aria-label="Bring Global Memory from elsewhere">
                <For each={offer()?.sources ?? []}>
                    {(src) => (
                        <div class="global-memory-import-source">
                            <span class="global-memory-import-text">
                                This channel keeps its own Global Memory. {scopeLabel(src.scope)} has{" "}
                                {src.missing.length} entr{src.missing.length === 1 ? "y" : "ies"} it doesn't:{" "}
                                <span class="global-memory-import-names">
                                    {src.missing.map((e) => e.name).join(", ")}
                                </span>
                            </span>
                            <button
                                type="button"
                                class="global-memory-import-button"
                                disabled={busy() !== null}
                                onClick={() => void bring(src.index)}
                            >
                                {busy() === src.index ? "Bringing…" : "Bring them here"}
                            </button>
                        </div>
                    )}
                </For>
                <Show when={message()}>
                    <p class="global-memory-import-message">{message()}</p>
                </Show>
            </section>
        </Show>
    );
};

GlobalMemoryImportBanner.displayName = "GlobalMemoryImportBanner";
