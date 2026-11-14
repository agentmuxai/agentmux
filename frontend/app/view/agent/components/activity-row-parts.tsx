// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { IconButton } from "@/app/element/ui/Button";
import { CloseButton } from "@/app/element/ui/CloseButton";
import { For, Show, type Accessor, type JSX } from "solid-js";
import { Tooltip } from "@/app/element/tooltip";
import { subagentDisplayLabel, type ActiveSubagent, type SubagentEvent } from "../../swarm/swarm-model";
import { KIND_SIGIL, type PinnedActivity } from "../../../agent-model/activity-types";
import { ActivitySigil } from "./ActivitySigil";
import { ChunkPreview } from "./ChunkPreview";
import type { ActivityRowProps, ActivityRowState } from "./activity-row-state";

/** One-line text summary per subagent event kind — deliberately simpler than
 *  the Swarm pane's own `DispatchActivityFeedEntry` (no per-event expand/
 *  collapse, no dedicated CSS): the dock reuses the existing shell-log line
 *  chrome (`.agent-tool-log-line`) instead of depending on `swarm-view.scss`,
 *  which only loads once the user has opened a Swarm pane this session. */
function subagentEventLine(e: SubagentEvent): string {
    const t = e.event_type;
    switch (t.type) {
        case "text": return t.content;
        case "result": return t.content;
        case "progress": return t.output;
        case "tool_use": return `→ ${t.name}`;
        case "tool_result": return t.is_error ? `✗ ${t.preview}` : t.preview;
    }
}

/** Terminal-status glyph for one member row inside an expanded group roster
 *  — mirrors ActivityRow's own top-level `s.sigil()` (running/done/stopped),
 *  just scoped to a single `ActiveSubagent` instead of a `PinnedActivity`. */
function memberSigil(status: ActiveSubagent["status"]): string {
    switch (status) {
        case "active": return KIND_SIGIL.subagent;
        case "completed": return "✓";
        case "abandoned": return "■";
    }
}

interface PartProps {
    s: ActivityRowState;
    a: Accessor<PinnedActivity>;
    props: ActivityRowProps;
}

/** The collapsed row: the sigil, the title, the stats on the right, and the stop or dismiss button. */
export const ActivitySummary = ({ s, a, props }: PartProps): JSX.Element => {
    return (
        <div class="agent-activity-summary" onClick={props.onToggle}>
            <span class="agent-activity-sigil">
                <ActivitySigil activity={a()} glyph={s.sigil()} tick={s.tick} />
            </span>
            <Tooltip content={a().fullTitle} disable={!a().fullTitle} divClassName="agent-activity-title">{a().title}</Tooltip>
            {/* The stats, on the right like the working row's: what it counted, a known wait's time left, how long it has run. */}
            <span class="agent-activity-stats">
                <Show when={s.tail()}>
                    <span class="agent-activity-tail">{s.tail()}</span>
                </Show>
                <Show when={s.remaining()}>
                    <span class="agent-activity-remaining">{s.remaining()}</span>
                </Show>
                <span class="agent-activity-elapsed">{s.elapsed()}</span>
            </span>
            <Show when={a().canStop}>
                <IconButton
                    icon="stop"
                    label="Stop"
                    tooltip={false}
                    tone="danger"
                    density="compact"
                    class="agent-activity-stop"
                    onClick={(e) => { e.stopPropagation(); props.onStop(); }}
                />
            </Show>
            <Show when={!a().canStop && a().status === "error"}>
                <CloseButton
                    size="sm"
                    label="Dismiss"
                    tooltip={false}
                    class="agent-activity-dismiss"
                    onClick={(e) => { e.stopPropagation(); props.onDismiss(); }}
                />
            </Show>
        </div>
    );
};

/** The expanded log of a shell or a tool call. */
export const ShellLog = ({ s, a }: PartProps): JSX.Element => {
    return (
        <div
            class="agent-activity-log agent-tool-overlay-log"
            onClick={(e) => e.stopPropagation()}
        >
            {/* Full command, word-wrapped — the collapsed
                title truncates, so this header is how the
                user identifies WHAT is running (user report
                2026-08-10: long-running process in the dock
                was unidentifiable when expanded). */}
            <Show when={s.fullCommand()}>
                <div class="agent-activity-command">{s.fullCommand()}</div>
            </Show>
            <ChunkPreview chunks={s.logChunks()} />
            <Show when={s.logChunks().length === 0 && a().tool}>
                <pre class="agent-tool-log-line">No output yet</pre>
            </Show>
            <Show when={a().shell?.log.open ?? a().tool?.log?.open ?? false}>
                <div class="agent-shell-streaming-indicator" />
            </Show>
        </div>
    );
};

/** The expanded transcript of one subagent. */
export const SubagentLog = ({ s }: PartProps): JSX.Element => {
    return (
        <div
            class="agent-activity-log agent-tool-overlay-log"
            onClick={(e) => e.stopPropagation()}
        >
            <Show when={s.dispatchDetail() && s.dispatchDetail()!.entriesAtom().length === 0}>
                <pre class="agent-tool-log-line">No activity yet</pre>
            </Show>
            <For each={s.dispatchDetail()?.entriesAtom() ?? []}>
                {(entry) => <pre class="agent-tool-log-line">{subagentEventLine(entry.event)}</pre>}
            </For>
        </div>
    );
};

/** Group roster — a compact per-member summary, not a live
 *  transcript: opening one dock row's expand shouldn't
 *  subscribe to N members' event streams at once (a
 *  single Task/Workflow run can spawn dozens). Full
 *  per-member transcripts remain the Swarm pane's job. */
export const GroupRoster = ({ a }: PartProps): JSX.Element => {
    return (
        <div
            class="agent-activity-log agent-tool-overlay-log"
            onClick={(e) => e.stopPropagation()}
        >
            <For each={a().subagentGroup!.members}>
                {(member) => (
                    <pre class="agent-tool-log-line">
                        {memberSigil(member.status)} {subagentDisplayLabel(member)}
                        {member.event_count > 0
                            ? ` — ${member.event_count} event${member.event_count === 1 ? "" : "s"}`
                            : ""}
                    </pre>
                )}
            </For>
        </div>
    );
};
