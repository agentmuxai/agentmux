// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which agents' messages may be going astray, on the Swarm row (the one job of the old Warden
 * Host list that the Swarm did not already do):
 *
 * - a badge when another AgentMux instance on this host also claims the agent's id (a jekt may
 *   go to whichever instance registered last), or a recent jekt to it was refused by the identity
 *   guard. A healthy row shows nothing.
 * - "Stop receiving messages" in the row's menu, on a badged row only: it removes the agent's
 *   routing entry. The session keeps running and may register again on its next heartbeat.
 *
 * srv answers `reactive.registration_flags` with only the agents that have something to show, so
 * one batched read every 15 s while the Swarm is visible covers every row.
 */

import { ConfirmModal } from "@/app/element/confirm-modal";
import { RpcApi, type ReactiveMismatchSummary, type ReactiveRemoteRegistration } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createContext, createEffect, createMemo, createSignal, onCleanup, Show, useContext, type Accessor, type JSX } from "solid-js";

/** One agent srv flags: a claim by another instance, a refused jekt, or both. */
export interface RegistrationFlag {
    agent_id: string;
    block_id: string;
    remote: ReactiveRemoteRegistration[];
    recent_mismatch: ReactiveMismatchSummary | null;
}

export const REGISTRATION_POLL_MS = 15_000;

type ReadFlags = (client: typeof TabRpcClient, data: Record<string, never>) => Promise<{ flags?: RegistrationFlag[] } | null>;

/**
 * The flags srv holds now (`reactive.registration_flags`). Empty on any failure, including a srv
 * that does not know the command: the stub is looked up by name, so this window shows no badges
 * until the stub is in its build and srv serves it, and nothing breaks before then.
 */
export async function readRegistrationFlags(): Promise<RegistrationFlag[]> {
    try {
        const read = (RpcApi as unknown as { GetReactiveRegistrationFlagsCommand?: ReadFlags }).GetReactiveRegistrationFlagsCommand;
        const result = await read?.(TabRpcClient, {});
        return Array.isArray(result?.flags) ? result!.flags! : [];
    } catch {
        return [];
    }
}

/** The badge's words: the claim first, since it is the one a person can act on. */
export function registrationText(flag: RegistrationFlag): string {
    return flag.remote.length > 0 ? "Another instance claims this agent" : "A message to it was refused: identity mismatch";
}

function agoText(at: number, now: number): string {
    const s = Math.max(0, Math.round((now - at) / 1000));
    if (s < 60) return "just now";
    if (s < 3600) return `${Math.floor(s / 60)} min ago`;
    return `${Math.floor(s / 3600)} h ago`;
}

/** The detail behind the badge, one line per fact. */
export function registrationDetail(flag: RegistrationFlag, now: number): string {
    const lines: string[] = [];
    if (flag.remote.length > 0) {
        lines.push(
            "Also claimed by another instance on this host. A message for this agent may go to whichever instance registered last."
        );
        for (const r of flag.remote) lines.push(`  channel ${r.channel} (pid ${r.pid}), updated ${agoText(r.updated_at, now)}`);
    }
    const m = flag.recent_mismatch;
    if (m) {
        lines.push("A recent message to it was refused because the block it resolved to reported a different live identity.");
        lines.push(`  ${agoText(m.timestamp, now)}, block ${m.block_id.slice(0, 8)}${m.error_message ? `: ${m.error_message}` : ""}`);
    }
    return lines.join("\n");
}

type Flags = ReadonlyMap<string, RegistrationFlag>;

const FlagsContext = createContext<Accessor<Flags>>(() => new Map());
const [stopRequest, setStopRequest] = createSignal<RegistrationFlag | null>(null);

/** Reads the flags now and every 15 s while `visible()`; one reader for the whole Swarm. */
export function RegistrationFlagsProvider(props: { visible: Accessor<boolean>; children: JSX.Element }): JSX.Element {
    const [flags, setFlags] = createSignal<Flags>(new Map());
    let alive = true;
    onCleanup(() => (alive = false));
    const refresh = async () => {
        const list = await readRegistrationFlags();
        if (alive) setFlags(new Map(list.map((f) => [f.block_id, f])));
    };
    createEffect(() => {
        if (!props.visible()) return;
        void refresh();
        const timer = setInterval(() => void refresh(), REGISTRATION_POLL_MS);
        onCleanup(() => clearInterval(timer));
    });
    return (
        <FlagsContext.Provider value={flags}>
            {props.children}
            <StopReceivingConfirm onDone={() => void refresh()} />
        </FlagsContext.Provider>
    );
}

/** What srv flags for the block a row shows; undefined for a healthy row. */
export function useRegistrationFlag(blockId: Accessor<string | null | undefined>): Accessor<RegistrationFlag | undefined> {
    const flags = useContext(FlagsContext);
    return createMemo(() => {
        const id = blockId();
        return id ? flags().get(id) : undefined;
    });
}

/** The badge on a flagged row's header; its hover text is the detail. */
export function RegistrationBadge(props: { flag: Accessor<RegistrationFlag | undefined> }): JSX.Element {
    return (
        <Show when={props.flag()}>
            {(flag) => (
                <span class="swarm-reg-badge" title={registrationDetail(flag(), Date.now())}>
                    <i class="fa-solid fa-triangle-exclamation swarm-reg-badge-icon" />
                    <span class="swarm-reg-badge-label">{registrationText(flag())}</span>
                </span>
            )}
        </Show>
    );
}

/** The row-menu entry for a flagged row; there is none for a healthy one. */
export function stopReceivingItems(flag: RegistrationFlag | undefined): ContextMenuItem[] {
    if (!flag) return [];
    return [{ label: "Stop receiving messages", click: () => setStopRequest(flag) }];
}

/** The answer a person gives before an agent's routing entry is removed. */
function StopReceivingConfirm(props: { onDone: () => void }): JSX.Element {
    return (
        <ConfirmModal
            open={stopRequest() !== null}
            title="Stop receiving messages?"
            description="Messages for this agent stop being delivered to it from this instance."
            confirmLabel="Stop receiving"
            destructive
            onCancel={() => setStopRequest(null)}
            onConfirm={async () => {
                const flag = stopRequest();
                setStopRequest(null);
                if (!flag) return;
                // The block id with the name: a name can be held by several panes, and srv refuses an
                // ambiguous one without it. Sent over the window's own socket (the host answers only it).
                await RpcApi.AgentPaneUnregisterCommand(TabRpcClient, { agent_id: flag.agent_id, block_id: flag.block_id });
                props.onDone();
            }}
        >
            <p>
                The session keeps running, and it may register again on its next heartbeat. This only removes the
                routing entry it holds now.
            </p>
        </ConfirmModal>
    );
}
