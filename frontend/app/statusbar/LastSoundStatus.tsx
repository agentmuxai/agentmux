// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { useTick } from "@/app/hook/useTick";
import { recentSounds, subscribePlayedSounds, type PlayedSound } from "@/app/notification/sound/sound-history";
import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { revealBlock } from "@/app/util/reveal-block";
import { lastSound, soundText } from "@/app/view/audit/audit-sound";
import { openAuditAtSound } from "@/app/view/audit/open-audit";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createEffect, createMemo, createSignal, on, onCleanup, Show, type Accessor, type JSX } from "solid-js";
import { refitStatusBar } from "./status-bar-fit";

/** The item redraws for a new sound at most this often. */
export const SOUND_REFRESH_MS = 1000;

/**
 * What this window played, as a signal that changes at most once a `SOUND_REFRESH_MS`: the first sound after a quiet
 * spell at once, the rest in one update when the interval is over. A window with many agents at work plays dozens of
 * sounds a second, and redrawing the item for each one cost a fifth of the renderer's CPU in the performance lab
 * (the 11-09 to 11-10 step of task #47680), for a line that says "4s ago" and is read once a second.
 */
function useRefreshedSounds(): Accessor<readonly PlayedSound[]> {
    const [sounds, setSounds] = createSignal(recentSounds());
    let last = Number.NEGATIVE_INFINITY;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const refresh = (): void => {
        timer = undefined;
        last = performance.now();
        setSounds(recentSounds());
    };
    const stop = subscribePlayedSounds(() => {
        if (timer !== undefined) return;
        const wait = SOUND_REFRESH_MS - (performance.now() - last);
        if (wait <= 0) refresh();
        else timer = setTimeout(refresh, wait);
    });
    onCleanup(() => {
        stop();
        if (timer !== undefined) clearTimeout(timer);
    });
    return sounds;
}

export const LAST_SOUND_TIP = "Click to see this in the Audit. Shift-click goes to the agent's pane";

/**
 * "Who was that": the last sound this window played, "♪ agent · Bash · 4s ago", for a minute after it. A click
 * opens the Audit at the row it came from (SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md §3.3);
 * a shift-click goes to the agent's pane.
 */
export function LastSoundStatus(props: { rows?: Accessor<AgentRuntime[]> }): JSX.Element {
    const sounds = useRefreshedSounds();
    const tick = useTick(1000);
    const now = createMemo(() => (tick(), Date.now()));
    const sound = createMemo(() => lastSound(sounds(), now()));
    // It comes and goes while a narrow bar may hide its slot, which changes no size the bar watches.
    createEffect(on(() => !!sound(), refitStatusBar, { defer: true }));
    const line = (s: PlayedSound) => soundText(s, who(), now());
    const who = () => (props.rows ?? sharedAgentRuntime().rows)().find((r) => r.block_id === sound()?.blockId)?.name;
    return (
        <Show when={sound()}>
            {(s) => (
                <div
                    class="status-bar-item clickable status-last-sound"
                    onClick={(e) => (e.shiftKey ? void revealBlock(s().blockId, { source: "audit" }) : void openAuditAtSound(s()))}
                    data-tip={LAST_SOUND_TIP}
                    aria-label={`${line(s())}. ${LAST_SOUND_TIP}`}
                    role="button"
                >
                    {line(s())}
                </div>
            )}
        </Show>
    );
}

LastSoundStatus.displayName = "LastSoundStatus";
