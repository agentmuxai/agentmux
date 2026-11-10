// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryAdoptionPanel — "Earlier memory found under N accounts", above an
 * agent's file grid in Memory → Personal
 * (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.4).
 *
 * Lists the agent's memory folders under its earlier accounts (and files
 * held at first sighting), lets the human pick some, and asks them to confirm
 * in a modal over this pane (`memory-confirms.tsx`). An agent can reach this
 * panel's buttons (they're Memory pane chrome), so they only ask: the modal
 * and srv's `agent:memory:adopt` are the person's alone.
 *
 * Renders nothing when there is nothing to offer.
 */

import { createEffect, createSignal, For, on, Show, type JSX } from "solid-js";
import { RpcApi, type NativeMemoryAdoptionCandidate, type NativeMemoryAdoptionList } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Button } from "@/app/element/ui";
import { accountLabel, AdoptConfirm, askInPane } from "./memory-confirms";

interface MemoryAdoptionPanelProps {
    agentId: string;
    agentName: string;
    /** Called after an adoption so the file grid can refresh. */
    onAdopted?: () => void;
}

type Outcome = { status: "adopted" | "declined" | "failed"; text: string };

/** Adopting reads the chosen folders and writes the record under the agent's reconcile lease. */
const ADOPT_TIMEOUT_MS = 30_000;

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

/** What the headline means, for a human who hasn't read the spec. */
export function adoptionHint(accounts: number, held: number): string {
    const parts = [];
    if (accounts > 0) {
        parts.push(
            "Memory is kept in a separate folder for each provider account an agent signs in with, so what this agent saved under an earlier account stayed there. " +
                "Adopting copies the files you pick into its current memory. Files it already has keep their current text, with the older text kept in history. " +
                "The one exception is the MEMORY.md index: the earlier entries are merged into it, so every adopted topic stays listed. " +
                "The earlier folders aren't changed.",
        );
    }
    if (held > 0) {
        parts.push(
            "Held files were already in its folder when AgentMux first saw it, but match another agent's memory, so they wait for you to adopt them.",
        );
    }
    return parts.join(" ");
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

    let root: HTMLElement | undefined;

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
        const agentId = props.agentId;
        const act = () =>
            RpcApi.NativeMemoryAdoptCommand(
                TabRpcClient,
                { agent_id: agentId, list_id: l.list_id, choices: picked.map((c) => ({ index: c.index, dir_hash: c.dir_hash })) },
                { timeout: ADOPT_TIMEOUT_MS }
            ).then(
                (report) => {
                    setOutcome(outcomeText({ status: "done", report }));
                    props.onAdopted?.();
                },
                (e) => void setOutcome(outcomeText({ status: "failed", error: e instanceof Error ? e.message : String(e) }))
            );
        const folders = picked.map((c) => ({ account: c.account, files: c.files.map((f) => f.name) }));
        const ended = await askInPane(root, AdoptConfirm, { agentName: props.agentName, folders }, act, {
            summary: `Adopt earlier memory into ${props.agentName}?`,
            text: "Adopting memory needs your answer",
            what: "asks before adopting memory",
        });
        setWaiting(false);
        if (ended === "no-pane") return void setError("Couldn't ask: Memory isn't open in a pane.");
        if (ended === "declined") setOutcome(outcomeText({ status: "declined" }));
        load(agentId);
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
            <section ref={root} class="memory-adoption-panel" aria-label="Earlier memory">
                <Show when={candidates().length > 0}>
                    <Button
                        density="compact"
                        icon={open() ? "chevron-down" : "chevron-right"}
                        class="memory-adoption-panel-toggle"
                        onClick={() => setOpen(!open())}
                    >
                        {headline()}
                    </Button>
                    <p class="memory-adoption-panel-hint">{adoptionHint(count(), heldCount())}</p>
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
                            <Button disabled={chosen().size === 0 || waiting()} onClick={() => void adopt()}>
                                {waiting() ? "Waiting for confirmation…" : "Adopt selected…"}
                            </Button>
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
