// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryAdoptionPanel — "Earlier memory found under N accounts", above an
 * agent's file grid in Armory → Memory → Personal
 * (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.4).
 *
 * Lists the agent's memory folders under its earlier accounts (and files
 * held at first sighting), lets the human pick some, and asks the host to
 * confirm. The confirmation is a separate host window, never this panel: an
 * agent can reach this panel's buttons (they're Armory chrome), so this
 * panel can only open that window, not adopt.
 *
 * Renders nothing when there is nothing to offer.
 */

import { createEffect, createSignal, For, on, onCleanup, Show, type JSX } from "solid-js";
import { RpcApi, type NativeMemoryAdoptionCandidate, type NativeMemoryAdoptionList } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getApi } from "@/store/global";
import { accountLabel } from "@/app/view/memory-adoption-approval/MemoryAdoptionApprovalWindow";

interface MemoryAdoptionPanelProps {
    agentId: string;
    agentName: string;
    /** Called after an adoption so the file grid can refresh. */
    onAdopted?: () => void;
}

type Outcome = { status: "adopted" | "declined" | "failed"; text: string };

export function outcomeText(payload: any): Outcome | null {
    switch (payload?.status) {
        case "done":
        case "adopted": {
            const r = payload.report ?? {};
            const parts = [
                r.files_added ? `${r.files_added} file(s) added` : null,
                r.index_lines_added ? `${r.index_lines_added} index line(s) added` : null,
                r.versions_kept ? `${r.versions_kept} older version(s) kept in history` : null,
            ].filter(Boolean);
            return {
                status: "adopted",
                text: `${parts.length ? parts.join(", ") : "Nothing new to add"}. They appear in the folder at the agent's next launch.`,
            };
        }
        case "declined":
            return { status: "declined", text: "Adoption cancelled." };
        case "failed":
            return {
                status: "failed",
                text: String(payload.error ?? "").startsWith("changed")
                    ? "A folder changed since it was listed. The list has been refreshed; choose again."
                    : `Adoption failed: ${payload.error ?? "unknown error"}`,
            };
        default:
            return null;
    }
}

export const MemoryAdoptionPanel = (props: MemoryAdoptionPanelProps): JSX.Element => {
    const [list, setList] = createSignal<NativeMemoryAdoptionList | null>(null);
    const [open, setOpen] = createSignal(false);
    const [chosen, setChosen] = createSignal<Set<number>>(new Set());
    const [waiting, setWaiting] = createSignal(false);
    const [outcome, setOutcome] = createSignal<Outcome | null>(null);
    const [error, setError] = createSignal<string | null>(null);

    const load = (agentId: string) => {
        RpcApi.NativeMemoryAdoptionListCommand(TabRpcClient, { agent_id: agentId })
            .then((r) => {
                if (agentId !== props.agentId) return;
                setList(r.list ?? null);
                setChosen(new Set<number>());
            })
            .catch(() => setList(null));
    };
    createEffect(on(() => props.agentId, (id) => {
        setOutcome(null);
        setError(null);
        load(id);
    }));

    let unlisten: (() => void) | undefined;
    void getApi().listen<any>("memory-adoption-result", (payload) => {
        if (payload?.agent_id !== props.agentId) return;
        // The same window confirms releasing a folder; that isn't ours.
        if (payload?.kind && payload.kind !== "adopt") return;
        setWaiting(false);
        setOutcome(outcomeText(payload));
        load(props.agentId);
        if (payload?.status === "done" || payload?.status === "adopted") props.onAdopted?.();
    }).then((u) => {
        unlisten = u;
    });
    onCleanup(() => unlisten?.());

    const candidates = () => list()?.candidates ?? [];
    const toggle = (index: number) => {
        const next = new Set(chosen());
        next.has(index) ? next.delete(index) : next.add(index);
        setChosen(next);
    };

    const adopt = async () => {
        const l = list();
        const picked = candidates().filter((c) => chosen().has(c.index));
        if (!l || picked.length === 0 || waiting()) return;
        setError(null);
        setOutcome(null);
        setWaiting(true);
        try {
            await getApi().approvals.requestMemoryAdoption({
                window_label: await getApi().getWindowLabel(),
                agent_id: props.agentId,
                list_id: l.list_id,
                choices: picked.map((c) => ({ index: c.index, dir_hash: c.dir_hash })),
                summary: {
                    agentName: props.agentName,
                    folders: picked.map((c) => ({ account: c.account, files: c.files.map((f) => f.name) })),
                },
            });
        } catch (e) {
            setWaiting(false);
            setError(`Couldn't open the confirmation window: ${e}`);
        }
    };

    const count = () => candidates().filter((c) => c.account !== "held").length;
    const heldCount = () => candidates().filter((c) => c.account === "held").length;
    const headline = () => {
        const parts = [];
        if (count() > 0) parts.push(`Earlier memory found under ${count()} other account${count() === 1 ? "" : "s"}`);
        if (heldCount() > 0) parts.push("files held for review");
        return parts.join(", and ");
    };

    return (
        <Show when={candidates().length > 0 || outcome()}>
            <section class="memory-adoption-panel" aria-label="Earlier memory">
                <Show when={candidates().length > 0}>
                    <button type="button" class="memory-adoption-panel-toggle" onClick={() => setOpen(!open())}>
                        <i class={`fa-solid fa-chevron-${open() ? "down" : "right"}`} aria-hidden="true" />
                        {headline()}
                    </button>
                </Show>
                <Show when={open() && candidates().length > 0}>
                    <div class="memory-adoption-panel-list">
                        <For each={candidates()}>
                            {(c: NativeMemoryAdoptionCandidate) => (
                                <label class="memory-adoption-panel-candidate">
                                    <input
                                        type="checkbox"
                                        checked={chosen().has(c.index)}
                                        onChange={() => toggle(c.index)}
                                        disabled={waiting()}
                                    />
                                    <span class="memory-adoption-panel-account">{accountLabel(c.account)}</span>
                                    <span class="memory-adoption-panel-files">
                                        {c.files.map((f) => f.name).join(", ")}
                                    </span>
                                </label>
                            )}
                        </For>
                        <div class="memory-adoption-panel-actions">
                            <button
                                type="button"
                                class="memory-adoption-panel-adopt"
                                disabled={chosen().size === 0 || waiting()}
                                onClick={() => void adopt()}
                            >
                                {waiting() ? "Waiting for confirmation…" : "Adopt selected…"}
                            </button>
                        </div>
                    </div>
                </Show>
                <Show when={error()}>
                    <p class="memory-adoption-panel-error">{error()}</p>
                </Show>
                <Show when={outcome()}>
                    {(o) => <p class={`memory-adoption-panel-outcome is-${o().status}`}>{o().text}</p>}
                </Show>
            </section>
        </Show>
    );
};

MemoryAdoptionPanel.displayName = "MemoryAdoptionPanel";
