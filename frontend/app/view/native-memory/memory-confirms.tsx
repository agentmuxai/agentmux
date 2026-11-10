// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The person's confirmation for adopting an agent's earlier memory, and for releasing its claim on
 * a memory folder (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2 and §2.1.4): a modal over the
 * Memory pane whose button was pressed, and nothing else
 * (SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md §6 row 8).
 *
 * An agent can press Memory's own buttons, so they only ask. The answer is the person's: a pane
 * request's modal is operator-only (an agent can neither read nor press it), and srv takes
 * `agent:memory:adopt` and `agent:memory:release` only from the AgentMux window. Escape, Cancel and
 * a closed pane all leave the memory as it was.
 */

import { For, Show, type JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";
import { openPaneModal, type PaneRequestProps } from "@/app/store/pane-requests";

export type AdoptionFolder = { account: string; files: string[] };

/** `act` does the thing and never rejects; `cancel` says the person chose not to. */
type Answer = { act: () => Promise<void>; cancel: () => void } & PaneRequestProps;

/** How a folder is named to the person. */
export function accountLabel(account: string): string {
    if (account === "held") return "Files already in its folder that match another agent's memory";
    if (account === "default") return "The default (unlinked) account";
    return `Account ${account.slice(0, 8)}`;
}

function answers(props: Answer) {
    return {
        onConfirm: async () => {
            await props.act();
            props.close();
        },
        onCancel: () => {
            props.close();
            props.cancel();
        },
    };
}

export function AdoptConfirm(props: { agentName: string; folders: AdoptionFolder[] } & Answer): JSX.Element {
    return (
        <ConfirmModal
            open={true}
            scope="pane"
            title={`Adopt earlier memory into ${props.agentName}?`}
            confirmLabel="Adopt"
            attention
            {...answers(props)}
        >
            <p class="text-sm">
                These files become part of {props.agentName}'s memory. Files it already has keep their current
                content; older versions are kept in their history. They appear in its folder at its next launch.
            </p>
            <For each={props.folders}>
                {(folder) => (
                    <section class="mt-3 text-sm">
                        <h3 class="font-semibold">{accountLabel(folder.account)}</h3>
                        <ul class="mt-1 list-disc pl-5 break-all">
                            <For each={folder.files}>{(name) => <li>{name}</li>}</For>
                        </ul>
                    </section>
                )}
            </For>
            <Show when={props.folders.length === 0}>
                <p class="mt-3 text-sm">No folders were chosen.</p>
            </Show>
        </ConfirmModal>
    );
}

export function ReleaseConfirm(props: { agentName: string; dir?: string } & Answer): JSX.Element {
    return (
        <ConfirmModal
            open={true}
            scope="pane"
            title={`Release ${props.agentName}'s claim on this folder?`}
            confirmLabel="Release"
            attention
            {...answers(props)}
        >
            <p class="text-sm font-mono break-all">{props.dir ?? "(folder not recorded)"}</p>
            <p class="mt-3 text-sm">
                While {props.agentName} claims it, no other agent's memory is written to or read from this folder.
                Release it when {props.agentName} no longer uses it, so another agent that does can sync its memory
                there. If {props.agentName} still uses it, its next launch claims it again.
            </p>
        </ConfirmModal>
    );
}

/** The pane `el` is in, or "" when it is in none. */
function paneOf(el: Element | undefined): string {
    return el?.closest("[data-blockid]")?.getAttribute("data-blockid") ?? "";
}

/**
 * Ask in the pane `el` is in. On yes the modal stays, busy, while `act` runs, then closes. Resolves
 * when it is over: "done" after `act`, "declined" on Cancel, Escape or a closed pane, and "no-pane"
 * when `el` is in no pane, so nothing could be asked.
 */
export function askInPane<P extends object>(
    el: Element | undefined,
    Component: (props: P & Answer) => JSX.Element,
    props: P,
    act: () => Promise<void>,
    about: { summary: string; text: string; what: string }
): Promise<"done" | "declined" | "no-pane"> {
    const blockId = paneOf(el);
    if (!blockId) return Promise.resolve("no-pane");
    return new Promise((resolve) => {
        const answer = { act: () => act().then(() => resolve("done")), cancel: () => resolve("declined") };
        openPaneModal(
            blockId,
            Component as (props: Record<string, unknown> & PaneRequestProps) => JSX.Element,
            { ...props, ...answer },
            { source: "memory-confirm", onGone: () => resolve("declined"), ...about }
        );
    });
}
