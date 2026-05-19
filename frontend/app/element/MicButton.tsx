// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ToggleIconButton } from "@/app/element/iconbutton";
import { getVoiceSession, type PaneVoiceHandle } from "@/app/hook/useVoiceInput";
import { getSettingsKeyAtom } from "@/app/store/global";
import { Show, type JSX } from "solid-js";
import "./MicButton.scss";

interface MicButtonProps {
    blockId: string;
    handle: PaneVoiceHandle;
    /** Optional context-aware tooltip (e.g. "Speak into this terminal").
     *  Falls back to a generic "Voice input" label when absent. */
    paneTitle?: string;
}

export function MicButton(props: MicButtonProps): JSX.Element {
    const voice = getVoiceSession();
    // `voice:enabled` is a global kill-switch (default true). Absent /
    // undefined ⇒ enabled; only an explicit `false` hides the button.
    const voiceEnabled = getSettingsKeyAtom("voice:enabled");

    // Active iff this pane owns the current voice session.
    const isActiveHere = () => voice.isListening() && voice.currentTargetId() === props.blockId;

    const handleClick = () => {
        // Snapshot pre-click state — registerPane below will overwrite
        // currentTargetId, so we need to know whether THIS pane already
        // owned the session before retargeting.
        const wasListening = voice.isListening();
        const wasMine = wasListening && voice.currentTargetId() === props.blockId;
        // Always bind this pane as the target; the session reads
        // `activeHandle` on every recognition event, so a click on
        // another pane's mic instantly retargets.
        voice.registerPane(props.blockId, props.handle);
        if (!wasListening) {
            voice.toggleListening(); // start
        } else if (wasMine) {
            voice.toggleListening(); // stop (same pane)
        }
        // else: was listening to another pane → registerPane already
        //       retargeted; do NOT toggle (would stop the session).
    };

    // Wrap our click handler into the SignalAtom._set slot so
    // ToggleIconButton can call _set(!active()) to dispatch the click.
    // We ignore the boolean it passes — we drive state ourselves.
    const activeAtom = Object.assign(() => isActiveHere(), {
        _set: (_v: boolean) => handleClick(),
    });

    return (
        <Show when={voice.isAvailable() && voiceEnabled() !== false}>
            <div class="mic-button-wrap" classList={{ "mic-active": isActiveHere() }}>
                <ToggleIconButton
                    decl={{
                        elemtype: "toggleiconbutton",
                        icon: "regular@microphone",
                        title: props.paneTitle ?? "Voice input (Ctrl+Shift+V)",
                        active: activeAtom,
                    }}
                />
            </div>
        </Show>
    );
}
