// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The line above the composer that says voice input is not set up, shown when this pane's mic was asked
 * for and the transcription engine has nothing to transcribe with (`VoiceSession.notSetUp`). The mic does
 * not record in that case, so without the line a click on it did nothing a person could read. It stays
 * until it is dismissed or the mic next records, and takes the person to Settings > Recording.
 */
import { Show, type JSX } from "solid-js";
import { Button, Notice } from "@/app/element/ui";
import { getVoiceSession, type VoiceSetupGap } from "@/app/hook/useVoiceInput";
import { openSettingsAt } from "@/app/view/settings/open-settings";

/** What is missing, in the person's words. srv's own sentence names settings keys, which only help in a file. */
export function voiceSetupText(gap: Pick<VoiceSetupGap, "missing">): string {
    switch (gap.missing) {
        case "whisper-cli":
            return "Local Whisper is chosen but its program isn't set: give the path to whisper-cli, or add a Groq key.";
        case "whisper-model":
            return "Local Whisper is chosen but its model file wasn't found: pick a model, or add a Groq key.";
        default:
            return "It needs a transcription engine: add a Groq key, or choose local Whisper.";
    }
}

export function VoiceSetupNotice(props: { blockId: string }): JSX.Element {
    const voice = getVoiceSession();
    const gap = () => {
        const g = voice.notSetUp?.();
        return g && g.blockId === props.blockId ? g : undefined;
    };
    const dismiss = () => {
        voice.notSetUp?._set(null);
        // The mic goes back to its plain look: the next click asks again.
        if (voice.lastError() === "not-configured") voice.lastError._set(null);
    };
    return (
        <Show when={gap()}>
            {(g) => (
                <Notice
                    tone="warning"
                    icon="microphone-slash"
                    class="agent-voice-setup-notice"
                    title="Voice input isn't set up"
                    actions={
                        <Button tone="accent" onClick={() => void openSettingsAt("recording")}>
                            Open Recording settings
                        </Button>
                    }
                    onDismiss={dismiss}
                >
                    {voiceSetupText(g())}
                </Notice>
            )}
        </Show>
    );
}
