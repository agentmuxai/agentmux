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
import { t } from "@/app/i18n/i18n";
import { openPaneModal, type PaneRequestProps } from "@/app/store/pane-requests";

export type AdoptionFolder = { account: string; files: string[] };

/** `act` does the thing and never rejects; `cancel` says the person chose not to. */
type Answer = { act: () => Promise<void>; cancel: () => void } & PaneRequestProps;

/** How a folder is named to the person. */
export function accountLabel(account: string): string {
    if (account === "held") return t("memoryConfirm.heldFolder");
    if (account === "default") return t("memoryConfirm.defaultAccount");
    return t("memoryConfirm.account", { id: account.slice(0, 8) });
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
            title={t("memoryConfirm.adoptTitle", { name: props.agentName })}
            confirmLabel={t("memoryConfirm.adopt")}
            {...answers(props)}
        >
            <p class="text-sm">
                {t("memoryConfirm.adoptBody", { name: props.agentName })}
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
                <p class="mt-3 text-sm">{t("memoryConfirm.noFolders")}</p>
            </Show>
        </ConfirmModal>
    );
}

export function ReleaseConfirm(props: { agentName: string; dir?: string } & Answer): JSX.Element {
    return (
        <ConfirmModal
            open={true}
            scope="pane"
            title={t("memoryConfirm.releaseTitle", { name: props.agentName })}
            confirmLabel={t("memoryConfirm.release")}
            {...answers(props)}
        >
            <p class="text-sm font-mono break-all">{props.dir ?? t("memoryConfirm.noDir")}</p>
            <p class="mt-3 text-sm">
                {t("memoryConfirm.releaseBody", { name: props.agentName })}
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
            { source: "memory-confirm", operatorOnly: true, onGone: () => resolve("declined"), ...about }
        );
    });
}
