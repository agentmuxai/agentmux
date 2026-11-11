// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Follows srv's `pane:confirm` events and asks each as a modal over the pane it is about
 * (docs/specs/SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md §6 rows 3 and 6): add a host to the
 * ssh config, remove or install AgentMux's helper, end a durable session, run a system tool's
 * install command. Each confirms the person's own click in that pane, so it covers that pane and
 * nothing else; it used to be an approval window over the whole AgentMux window.
 *
 * The answer goes to srv the one way it can, through `pane.confirm.decide`, which srv accepts only
 * from the AgentMux window, and a pane request's modal is operator-only, so an agent can neither
 * read nor press it. Nothing goes ahead without an answered yes: Escape, a closed pane and srv's
 * own time limit all leave the thing undone. The tone and notification come from srv's attention
 * event for the same pane.
 */

import { createSignal, Show, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { Switch } from "@/app/element/ui";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { openPaneModal, type PaneRequestHandle, type PaneRequestProps } from "@/app/store/pane-requests";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { PaneConfirmView } from "@/types/rpc/PaneConfirmView";

type Decide = (approve: boolean, checkbox: boolean) => Promise<void>;

const shown = new Map<string, PaneRequestHandle>();

function decide(q: PaneConfirmView, approve: boolean, checkbox: boolean): Promise<void> {
    return RpcApi.PaneConfirmDecideCommand(TabRpcClient, { request_id: q.id, approve, checkbox }).then(() => {});
}

export function PaneConfirm(props: { question: PaneConfirmView; decide: Decide } & PaneRequestProps): JSX.Element {
    const q = props.question;
    const [ticked, setTicked] = createSignal(false);
    const [failed, setFailed] = createSignal("");
    const answer = async (approve: boolean) => {
        try {
            await props.decide(approve, approve && ticked());
            props.close();
        } catch (e) {
            // The question stays: srv still waits, and the person can answer again.
            setFailed(e instanceof Error ? e.message : String(e));
        }
    };
    const boxId = `pane-confirm-box-${q.id}`;
    return (
        <ConfirmModal
            open={true}
            scope="pane"
            title={q.title}
            confirmLabel={q.ok_label}
            cancelLabel={q.cancel_label}
            destructive={q.destructive}
            onConfirm={() => answer(true)}
            onCancel={() => void answer(false)}
        >
            <p class="text-sm whitespace-pre-wrap break-words">{q.message}</p>
            <Show when={q.checkbox}>
                <div class="mt-3 flex items-center gap-2 text-sm">
                    <Switch id={boxId} checked={ticked()} onChange={setTicked} density="compact" />
                    <label for={boxId}>{q.checkbox}</label>
                </div>
            </Show>
            <Show when={failed()}>
                <p class="mt-3 text-sm" role="alert">
                    {failed()}
                </p>
            </Show>
        </ConfirmModal>
    );
}

function drop(id: string): void {
    const handle = shown.get(id);
    if (!handle) return;
    shown.delete(id);
    handle.close();
}

function show(q: PaneConfirmView): void {
    if (!q.block_id || shown.has(q.id)) return;
    const handle = openPaneModal(
        q.block_id,
        PaneConfirm,
        { question: q, decide: (approve: boolean, checkbox: boolean) => decide(q, approve, checkbox) },
        {
            source: "pane-confirm",
            deadlineMs: q.deadline_ms,
            summary: q.title,
            waits: false,
            // The pane was closed with the question open: the safe answer is no.
            onGone: () => {
                shown.delete(q.id);
                void decide(q, false, false).catch(() => {});
            },
        }
    );
    shown.set(q.id, handle);
}

let started = false;

/** Ask each of srv's pane questions in its pane. Call once per window. */
export function startPaneConfirms(): void {
    if (started) return;
    started = true;
    onMuxEvent(WpsEvent.PaneConfirm, (q) => {
        if (q.status === "cleared") drop(q.id);
        else show(q);
    });
}

/** Tests only. */
export function __resetPaneConfirms(): void {
    for (const id of [...shown.keys()]) drop(id);
    started = false;
}
