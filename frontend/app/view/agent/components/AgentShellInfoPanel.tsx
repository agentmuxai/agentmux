// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentShellInfoPanel — one line at the top of the Shell drawer answering the
 * two questions the drawer itself can't: *what is this shell?* and *what has
 * the agent left running on this machine?*
 *
 * Takes the slot `AgentControlBar` used to occupy with session-lifecycle UI
 * (moved out in #3435). See
 * `docs/specs/SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md` §4.
 *
 * Deliberately one row: the terminal below it is the point of the drawer, and
 * this must not eat its height. The process table (§4.2) expands from here in
 * a later phase.
 *
 * Nothing here is a liveness signal — the counts come from the per-block
 * tracker, which is real only on Windows today (`confidence`); elsewhere the
 * shell half still works, since it comes from `controllerstatus`.
 */

import { createEffect, createMemo, createSignal, onCleanup, Show, type JSX } from "solid-js";

import * as MOS from "@/app/store/mos";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";

import { useTrackedProcesses } from "../hooks/useTrackedProcesses";

interface AgentShellInfoPanelProps {
    /** The agent pane's own block — what the agent started lives here. */
    blockId: string;
    /** The drawer shell's sub-block; undefined until the shell is created. */
    shellSubBlockId?: string;
    /** The directory the shell was started in (`cmd:cwd`). */
    cwd?: string;
}

/** `312 MB`, `1.2 GB`, `—` for 0/unknown. */
function formatBytes(n: number): string {
    if (!n) return "—";
    const mb = n / 1024 / 1024;
    if (mb < 1024) return `${Math.round(mb)} MB`;
    return `${(mb / 1024).toFixed(1)} GB`;
}

/** `12m`, `3h 04m`, `2d 4h` — uptime, coarse on purpose. */
function formatUptime(ms: number): string {
    const m = Math.floor(ms / 60_000);
    if (m < 1) return "just started";
    if (m < 60) return `${m}m`;
    const h = Math.floor(m / 60);
    if (h < 24) return `${h}h ${String(m % 60).padStart(2, "0")}m`;
    return `${Math.floor(h / 24)}d ${h % 24}h`;
}

/** Keeps the tail: `…\agentmux-main-fresh\frontend`. */
function shortenPath(p: string, max = 34): string {
    if (p.length <= max) return p;
    return `…${p.slice(-(max - 1))}`;
}

/** Executable name from a full image path, without the `.exe`. */
function imageName(command: string): string {
    const base = command.split(/[\\/]/).pop() ?? command;
    return base.replace(/\.exe$/i, "");
}

/** Hover text listing what's running, until the expandable table (§4.2) lands. */
function procTitle(procs: { command: string; pid: number }[]): string {
    return procs.map((p) => `${imageName(p.command)} (${p.pid})`).join("\n");
}

export const AgentShellInfoPanel = (props: AgentShellInfoPanelProps): JSX.Element => {
    const [status, setStatus] = createSignal<string>("init");
    const [exitCode, setExitCode] = createSignal<number>(0);
    const [pid, setPid] = createSignal<number | undefined>(undefined);
    const [name, setName] = createSignal<string>("");
    const [spawnTs, setSpawnTs] = createSignal<number | undefined>(undefined);
    // Re-read on a slow tick so uptime advances without a status event.
    const [now, setNow] = createSignal(Date.now());

    const agentProcs = useTrackedProcesses(() => props.blockId);
    const shellProcs = useTrackedProcesses(() => props.shellSubBlockId);

    createEffect(() => {
        const id = props.shellSubBlockId;
        if (!id) return;
        // `controllerstatus` is published with `persist: 1`, so subscribing
        // replays the shell's current status immediately — this panel is
        // correct on a drawer reopen without waiting for the next change.
        const unsub = muxEventSubscribe({
            eventType: WpsEvent.ControllerStatus,
            scope: MOS.makeORef("block", id),
            handler: (event) => {
                const data = event?.data as
                    | {
                          shellprocstatus?: unknown;
                          shellprocexitcode?: unknown;
                          shellprocpid?: unknown;
                          shellprocname?: unknown;
                          spawn_ts_ms?: unknown;
                      }
                    | undefined;
                if (!data) return;
                if (typeof data.shellprocstatus === "string") setStatus(data.shellprocstatus);
                setExitCode(typeof data.shellprocexitcode === "number" ? data.shellprocexitcode : 0);
                if (typeof data.shellprocpid === "number") setPid(data.shellprocpid);
                if (typeof data.shellprocname === "string" && data.shellprocname) setName(data.shellprocname);
                if (typeof data.spawn_ts_ms === "number") setSpawnTs(data.spawn_ts_ms);
            },
        });
        onCleanup(() => unsub());
    });

    createEffect(() => {
        const t = setInterval(() => setNow(Date.now()), 30_000);
        onCleanup(() => clearInterval(t));
    });

    const running = () => status() === "running";
    const failed = () => status() === "done" && exitCode() !== 0;

    // Reported by the controller, not the tracker: the tracker omits each
    // block's root process, and here the root IS the shell.
    const shellName = createMemo(() => name() || "shell");

    const uptime = createMemo(() => {
        const ts = spawnTs();
        if (!ts || !running()) return null;
        return formatUptime(Math.max(0, now() - ts));
    });

    const totalRss = createMemo(() => agentProcs.list().reduce((sum, p) => sum + (p.rss_bytes || 0), 0));

    return (
        <div class="agent-shell-info">
            <div class="agent-shell-info-shell">
                <span
                    class="agent-shell-info-dot"
                    classList={{
                        "agent-shell-info-dot--running": running(),
                        "agent-shell-info-dot--failed": failed(),
                    }}
                    aria-hidden="true"
                />
                <span class="agent-shell-info-name">{shellName()}</span>
                <Show when={pid() != null}>
                    <span class="agent-shell-info-pid">pid {pid()}</span>
                </Show>
                <Show when={failed()}>
                    <span class="agent-shell-info-exit">exited {exitCode()}</span>
                </Show>
                <Show when={uptime()}>
                    <span class="agent-shell-info-uptime">{uptime()}</span>
                </Show>
                <Show when={props.cwd}>
                    <span class="agent-shell-info-cwd" title={`Started in ${props.cwd}`}>
                        {shortenPath(props.cwd!)}
                    </span>
                </Show>
            </div>

            <div class="agent-shell-info-procs">
                <Show
                    when={agentProcs.confidence() !== "none"}
                    fallback={
                        <span class="agent-shell-info-muted" title="Agent process tracking needs Windows Job Objects; other platforms report nothing yet.">
                            process tracking unavailable
                        </span>
                    }
                >
                    <Show when={agentProcs.list().length > 0}>
                        <span class="agent-shell-info-count" title={procTitle(agentProcs.list())}>
                            {agentProcs.confidence() === "best_effort" ? "≈" : ""}
                            {agentProcs.list().length} started by the agent
                        </span>
                        <Show when={totalRss() > 0}>
                            <span class="agent-shell-info-rss">{formatBytes(totalRss())}</span>
                        </Show>
                    </Show>
                    {/* Kept separate from the agent's own: these outlive the
                        drawer (closing it is a view toggle, never a kill), so
                        a backgrounded job here is exactly what scrolls out of
                        the terminal and gets forgotten. */}
                    <Show when={shellProcs.list().length > 0}>
                        <span class="agent-shell-info-count" title={procTitle(shellProcs.list())}>
                            {shellProcs.list().length} from this shell
                        </span>
                    </Show>
                </Show>
            </div>
        </div>
    );
};

AgentShellInfoPanel.displayName = "AgentShellInfoPanel";
