// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { useTick } from "@/app/hook/useTick";
import type { PlayedSound } from "@/app/notification/sound/sound-history";
import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { revealBlock } from "@/app/util/reveal-block";
import { lastSound, soundText, usePlayedSounds } from "@/app/view/audit/audit-sound";
import { openAuditAtSound } from "@/app/view/audit/open-audit";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { createEffect, createMemo, on, Show, type Accessor, type JSX } from "solid-js";
import { refitStatusBar } from "./status-bar-fit";

export const LAST_SOUND_TIP = "Click to see this in the Audit. Shift-click goes to the agent's pane";

/**
 * "Who was that": the last sound this window played, "♪ agent · Bash · 4s ago", for a minute after it. A click
 * opens the Audit at the row it came from (SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md §3.3);
 * a shift-click goes to the agent's pane.
 */
export function LastSoundStatus(props: { rows?: Accessor<AgentRuntime[]> }): JSX.Element {
    const sounds = usePlayedSounds();
    const tick = useTick(1000);
    const now = createMemo(() => (tick(), Date.now()));
    const sound = createMemo(() => lastSound(sounds(), now()));
    // It comes and goes while a narrow bar may hide its slot, which changes no size the bar watches.
    createEffect(on(() => !!sound(), refitStatusBar, { defer: true }));
    const line = (s: PlayedSound) => soundText(s, who(), now());
    const who = () => (props.rows ?? sharedAgentRuntime().rows)().find((r) => r.block_id === sound()?.blockId)?.agent_id;
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
