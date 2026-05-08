// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ToggleIconButton } from "@/app/element/iconbutton";
import { getVoiceSession } from "@/app/hook/useVoiceInput";
import { Show, type JSX } from "solid-js";
import "./MicButton.scss";

export function MicButton(): JSX.Element {
    const voice = getVoiceSession();

    // Wrap toggleListening into the SignalAtom._set slot so ToggleIconButton
    // can call _set(!active()) to trigger the toggle.
    const activeAtom = Object.assign(() => voice.isListening(), {
        _set: (_v: boolean) => voice.toggleListening(),
    });

    return (
        <Show when={voice.isAvailable()}>
            <div class="mic-button-wrap" classList={{ "mic-active": voice.isListening() }}>
                <ToggleIconButton
                    decl={{
                        elemtype: "toggleiconbutton",
                        icon: "regular@microphone",
                        title: "Voice input (Ctrl+Shift+V)",
                        active: activeAtom,
                    }}
                />
            </div>
        </Show>
    );
}
