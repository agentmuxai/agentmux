// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The countdown on the composer's account chip while an agent gets a window to reach a safe point before
// a switch restarts its session (SPEC_AGENT_PREP_BEFORE_SESSION_RESTART_2026_11_14.md §3.6). srv writes
// the window to the pane's meta (`agent:prep`); this reads it.
import { createEffect, createMemo, createSignal, onCleanup, type Accessor } from "solid-js";

export interface PrepMeta {
    id: string;
    action: string;
    /** When the window ends, unless the agent asks for more time (epoch ms). */
    until_ms: number;
    /** The agent asked for the longer window. */
    extended: boolean;
    /** The agent's one line for why it needs longer. */
    why?: string | null;
}

/** "24 s", or "1:40" from a minute up. */
export function formatRemaining(ms: number): string {
    const s = Math.max(0, Math.ceil(ms / 1000));
    return s < 60 ? `${s} s` : `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** The chip's line while the window is open, or undefined when there is none (or it is not a well-formed one). */
export function prepCountdownText(prep: PrepMeta | null | undefined, nowMs: number, agentName?: string): string | undefined {
    if (!prep || typeof prep.until_ms !== "number") return undefined;
    const who = agentName || "the agent";
    const left = formatRemaining(prep.until_ms - nowMs);
    if (prep.extended) {
        const why = prep.why ? `: ${prep.why}` : "";
        return `${who} asks for more time${why} · ${left}`;
    }
    return `waiting for ${who} to reach a safe point · ${left}`;
}

/** The time, ticking once a second while a window is open and not at all otherwise. */
export function usePrepTicker(prep: Accessor<PrepMeta | null | undefined>): Accessor<number> {
    const [now, setNow] = createSignal(Date.now());
    createEffect(() => {
        if (!prep()) return;
        setNow(Date.now());
        const timer = setInterval(() => setNow(Date.now()), 1000);
        onCleanup(() => clearInterval(timer));
    });
    return now;
}

/** What the chip needs of a window: its countdown line, and the click that ends it. */
export function usePrepWindow(
    opts: { prep?: Accessor<PrepMeta | null | undefined>; agentName?: Accessor<string | undefined>; agentDefinitionId: Accessor<string | undefined> },
    deps: { prepNow: (agentId: string) => Promise<unknown>; notify: (type: "error", title: string, message: string) => void }
) {
    const prep = () => opts.prep?.();
    const now = usePrepTicker(prep);
    const prepCountdown = createMemo(() => prepCountdownText(prep(), now(), opts.agentName?.()));
    const onSwitchNow = () => {
        const agentId = opts.agentDefinitionId();
        if (!agentId) return;
        deps.prepNow(agentId).catch((e: any) => deps.notify("error", "Couldn't switch now", String(e?.message ?? e)));
    };
    return { prepCountdown, onSwitchNow };
}
