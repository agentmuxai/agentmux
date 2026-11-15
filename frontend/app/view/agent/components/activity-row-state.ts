// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { formatElapsedCompact, formatTimeLeft } from "@/util/format-time";
import { createDispatchDetail, type DispatchDetail } from "../../swarm/swarm-model";
import { EXIT_FLASH_MS, KIND_SIGIL, type PinnedActivity } from "../../../agent-model/activity-types";
import { usePulseWhen } from "@/app/motion/solid";
import { toolOutputChunks } from "../../../agent-model/tool-adapter";
import type { ToolLogChunk } from "../../../agent-model/types";

export interface ActivityRowProps {
    /** Reactive accessor — returns undefined if the activity just left. */
    activity: () => PinnedActivity | undefined;
    expanded: () => boolean;
    /** True once this row is past its retention window and only still
     *  mounted for the departure flash — see ActivityDock's `isLeaving`. */
    leaving: () => boolean;
    onToggle: () => void;
    onStop: () => void;
    onDismiss: () => void;
}

/** The landing flash: `entering` is true until the row has finished landing, and the row's element is the one pulsed. */
function createLanding() {
    // Landing flash on first mount: the same bounce a tab plays when it lands after a
    // drag/drop. `entering` is cleared after EXIT_FLASH_MS so the departure flash waits it out.
    const [entering, setEntering] = createSignal(true);
    let rowRef: HTMLDivElement | undefined;
    usePulseWhen(() => rowRef, entering);
    onMount(() => {
        const timer = setTimeout(() => setEntering(false), EXIT_FLASH_MS);
        onCleanup(() => clearTimeout(timer));
    });

    return {
        entering,
        setRowRef: (el: HTMLDivElement) => {
            rowRef = el;
        },
    };
}

/** The figures the collapsed row shows, and the full command the expanded header shows. */
function createFigures(props: ActivityRowProps, tick: () => number) {
    /**
     * "~Ns left" for a whole-command sleep, while it's still running — the one
     * activity whose remaining time is actually known (`sleepMs`, set only by
     * `sleep-detect.ts`'s narrow matcher). Everything else shows elapsed only,
     * because a guess here would be worse than nothing.
     *
     * Clamped at 0: the process is reaped slightly after its own deadline, so
     * the last tick before `ToolEnd` would otherwise render a negative number.
     * Empty once terminal — a finished row shows its final elapsed, not a
     * countdown to a moment that has passed.
     */
    const remaining = createMemo(() => {
        const a = props.activity();
        if (!a || a.sleepMs == null || a.status !== "running") return "";
        tick();
        return formatTimeLeft(a.startedAt + a.sleepMs - Date.now());
    });

    const elapsed = createMemo(() => {
        const a = props.activity();
        if (!a) return "";
        const end = a.endedAt ?? (tick(), Date.now());
        return formatElapsedCompact(end - a.startedAt);
    });

    // Terminal statuses override the kind sigil with a result glyph.
    const sigil = createMemo(() => {
        const a = props.activity();
        if (!a) return "";
        switch (a.status) {
            case "running": return KIND_SIGIL[a.kind];
            case "unknown": return "?";
            case "done": return "✓";
            case "error": return "✗";
            case "stopped": return "■";
        }
    });

    const tail = createMemo((): string | undefined => {
        const a = props.activity();
        if (!a) return undefined;
        // A shell's or a tool's last output line is not shown on the row: the expanded log has it.
        if (a.subagent) {
            // Cheap tail from data already on ActiveSubagent — no extra
            // fetch/subscribe just to render a collapsed row's tail (that
            // cost is reserved for the expanded view, below).
            const n = a.subagent.event_count;
            return n > 0 ? `${n} event${n === 1 ? "" : "s"}` : undefined;
        }
        if (a.subagentGroup) {
            const members = a.subagentGroup.members;
            const active = members.filter((m) => m.status === "active").length;
            return active > 0 ? `${active}/${members.length} active` : `${members.length} subagents`;
        }
        return undefined;
    });

    // Full command for the expanded header — the collapsed row's title is
    // ellipsis-truncated by CSS, so a long command (a heartbeat-wrapped
    // dev launch, a multi-flag one-liner) is unidentifiable without this.
    // Shell rows: `cmd` is the real command even when the row carries a
    // custom display title. Tool rows: BashParams.command, falling back to
    // the title (which toolToActivity derives from the same field).
    const fullCommand = createMemo((): string | undefined => {
        const a = props.activity();
        if (!a) return undefined;
        if (a.shell) return a.shell.cmd;
        if (a.tool) return (a.tool.params as { command?: string } | undefined)?.command ?? a.fullTitle ?? a.title;
        return undefined;
    });

    return { remaining, elapsed, sigil, tail, fullCommand };
}

/** What the expanded row shows below its header: a shell's or a tool's log, a subagent's transcript. */
function createTranscript(props: ActivityRowProps) {
    // Expanded shell/tool log — the same streamed-output preview as
    // PersistentShellBlock and a running tool (ChunkPreview.tsx). A promoted
    // tool call's log is optional (only populated for tools that stream
    // partials — see ToolNode.log's own doc comment), unlike a ShellNode's,
    // which always has one.
    const logChunks = createMemo((): ToolLogChunk[] => {
        const a = props.activity();
        return (a?.shell?.log.chunks ?? (a?.tool ? toolOutputChunks(a.tool) : undefined) ?? []) as ToolLogChunk[];
    });

    // Expanded subagent transcript — created only while this row is actually
    // expanded (mirrors SwarmViewModel's own lazy `getDispatchDetail` cache),
    // disposed on collapse/unmount so an idle dock isn't holding N live
    // event subscriptions for subagents nobody is looking at.
    const [dispatchDetail, setDispatchDetail] = createSignal<DispatchDetail | null>(null);
    createEffect(() => {
        const sub = props.activity()?.subagent;
        if (!props.expanded() || !sub) {
            setDispatchDetail(null);
            return;
        }
        const detail = createDispatchDetail(sub.dispatch_id, sub.agent_id);
        setDispatchDetail(detail);
        onCleanup(() => detail.dispose());
    });

    return { logChunks, dispatchDetail };
}

/** The state of one dock row: its clock, its landing flash, the figures it shows and the transcript it opens. */
export function createActivityRow(props: ActivityRowProps) {
    const tick = useTick(1000);
    const landing = createLanding();
    const figures = createFigures(props, tick);
    const transcript = createTranscript(props);

    return { tick, ...landing, ...figures, ...transcript };
}

export type ActivityRowState = ReturnType<typeof createActivityRow>;
