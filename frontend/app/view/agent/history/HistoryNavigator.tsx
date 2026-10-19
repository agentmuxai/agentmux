// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Agent History reader's navigation (SPEC_WARDEN_HISTORY_SECTION_2026_10_18.md
 * phase 2): which conversation it shows (the current one, or one archived by
 * "New conversation"), how many sessions that conversation holds, and a jump to
 * a session's end or a date. A jump shows the conversation up to that point,
 * at the bottom as the reader always opens; "Show newer" pages forward from
 * there, and "Back to latest" returns to following the conversation.
 */

import { createEffect, createMemo, createSignal, on, Show, type Accessor, type JSX } from "solid-js";

import { Button, Select, TextInput } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentArchiveRow } from "@/types/rpc/AgentArchiveRow";
import type { HistorySession } from "@/types/rpc/HistorySession";

import "./history-navigator.scss";

export type HistoryMode = "live" | "jumped" | "archive";

export interface HistoryNavigatorProps {
    /** The agent's definition id; without one there is nothing to list. */
    agentId?: string;
    /** The block the reader reads through (its `agentId` names the agent). */
    readBlockId: string;
    mode: Accessor<HistoryMode>;
    archiveZone: Accessor<string | null>;
    /** Lines below what is loaded: "Show newer" has something to show. */
    hasNewer: Accessor<boolean>;
    onOpenArchive: (zone: string | null) => void;
    /** Show the conversation up to line `line`. */
    onJump: (line: number) => void;
    onShowNewer: () => void;
    onLatest: () => void;
}

const CURRENT = "";
/** The first read of a long record scans it: minutes for the largest. */
export const SESSIONS_TIMEOUT_MS = 300_000;

/** "Oct 17, 4:27 PM". */
export function shortWhen(ms: number): string {
    if (!ms) return "unknown time";
    return new Date(ms).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

/** The end of a local calendar day, `yyyy-mm-dd`, in Unix ms; null for anything else. */
export function endOfDayMs(day: string): number | null {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
    if (!m) return null;
    const end = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]) + 1).getTime() - 1;
    return Number.isFinite(end) ? end : null;
}

/** A session in the "Jump to" list. */
export function sessionLabel(s: HistorySession): string {
    const id = `Session ${s.session_id.slice(0, 8)}`;
    // Sessions older than the receive-time sidecar have no times.
    if (!s.first_ms && !s.last_ms) return `${id} · no time recorded`;
    return `${id} · ${shortWhen(s.first_ms)} – ${shortWhen(s.last_ms)}`;
}

export function HistoryNavigator(props: HistoryNavigatorProps): JSX.Element {
    const [archives, setArchives] = createSignal<AgentArchiveRow[]>([]);
    const [sessions, setSessions] = createSignal<HistorySession[] | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [jumping, setJumping] = createSignal(false);
    const [day, setDay] = createSignal("");
    // The jump list goes back to its prompt after each jump, so the same
    // session can be picked again.
    const [picked, setPicked] = createSignal("");
    const [counting, setCounting] = createSignal(false);

    createEffect(
        on(
            () => props.agentId,
            async (id) => {
                if (!id) return;
                // Separately: the archive list is quick, while the first read of
                // a long record scans it (a 1 GB one takes well over the default
                // RPC timeout); later reads only what was appended.
                RpcApi.AgentSessionListArchivesCommand(TabRpcClient, { definition_id: id, limit: 100 })
                    .then((rows) => setArchives(rows ?? []))
                    .catch((e) => setError(String(e)));
                setCounting(true);
                try {
                    const record = await RpcApi.HistoryAgentSessionsCommand(TabRpcClient, { definition_id: id }, { timeout: SESSIONS_TIMEOUT_MS });
                    setSessions(record?.sessions ?? []);
                } catch (e) {
                    setError(String(e));
                } finally {
                    setCounting(false);
                }
            }
        )
    );

    /** The zone the reader shows: the archive, or the agent's current conversation. */
    const zone = (): string => props.archiveZone() ?? `agent:${props.agentId}:current`;
    /** Its sessions, oldest first. */
    const shown = createMemo(() =>
        (sessions() ?? []).filter((s) => s.zone === zone()).sort((a, b) => a.first_ms - b.first_ms)
    );

    const locate = async (at: { at_ms?: number; at_byte?: number }) => {
        setJumping(true);
        try {
            const loc = await RpcApi.HistoryLocateCommand(TabRpcClient, {
                block_id: props.readBlockId,
                ...(props.archiveZone() ? { archive_zone: props.archiveZone()! } : {}),
                ...at,
            });
            setError(null);
            props.onJump(loc.line);
        } catch (e) {
            setError(String(e));
        } finally {
            setJumping(false);
            setPicked("");
        }
    };

    const conversationOptions = createMemo(() => [
        { value: CURRENT, label: "Current conversation" },
        ...archives().map((a) => ({
            value: a.archive_zoneid,
            label: `Archived ${shortWhen(a.archived_at_ms)}${a.preview ? ` · ${a.preview.slice(0, 60)}` : ""}`,
        })),
    ]);
    const jumpOptions = createMemo(() => [
        { value: "", label: shown().length ? "Jump to a session…" : "No sessions in the record" },
        ...shown().map((s) => ({ value: String(s.end_byte), label: sessionLabel(s) })),
    ]);

    return (
        <div class="agent-history-nav">
            <Show when={archives().length > 0}>
                <Select
                    options={conversationOptions()}
                    value={props.archiveZone() ?? CURRENT}
                    onChange={(v) => props.onOpenArchive(v === CURRENT ? null : v)}
                    density="compact"
                    ariaLabel="Conversation"
                />
            </Show>
            <Show when={counting()}>
                <span class="agent-history-nav-count">counting sessions…</span>
            </Show>
            <Show when={sessions() !== null}>
                <span class="agent-history-nav-count">
                    {shown().length} {shown().length === 1 ? "session" : "sessions"}
                </span>
                <Select
                    options={jumpOptions()}
                    value={picked()}
                    onChange={(v) => {
                        setPicked(v);
                        if (v) void locate({ at_byte: Number(v) });
                    }}
                    density="compact"
                    disabled={jumping() || shown().length === 0}
                    ariaLabel="Jump to a session"
                />
            </Show>
            <TextInput
                type="date"
                density="compact"
                value={day()}
                aria-label="Jump to a date"
                disabled={jumping()}
                onChange={(e) => {
                    const value = e.currentTarget.value;
                    setDay(value);
                    const end = endOfDayMs(value);
                    if (end !== null) void locate({ at_ms: end });
                }}
            />
            <Show when={props.mode() !== "live" && props.hasNewer()}>
                <Button tone="quiet" density="compact" onClick={() => props.onShowNewer()}>
                    Show newer
                </Button>
            </Show>
            <Show when={props.mode() !== "live"}>
                <Button tone="quiet" density="compact" icon="arrow-down" onClick={() => props.onLatest()}>
                    {props.mode() === "archive" ? "Back to current" : "Back to latest"}
                </Button>
            </Show>
            <Show when={error()}>
                <span class="agent-history-nav-error">⚠ {error()}</span>
            </Show>
        </div>
    );
}
