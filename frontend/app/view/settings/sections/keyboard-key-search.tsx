// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Search by key in Settings → Keyboard shortcuts (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §4.5):
// "Record keys" captures a press and keeps the list to the commands that have it, as VS Code's Record Keys does.

import { createSignal, Show, type JSX } from "solid-js";

import { Button, Pill, TextInput } from "@/app/element/ui";
import { captureKey } from "@/app/keybindings/capture";
import { formatKey, type KeyPlatform } from "@/app/keybindings/keys";
import { useKeyCapture } from "./keyboard-capture-field";
import { MAX_STEPS } from "./keyboard-recorder-state";
import type { KeyboardFilters } from "./keyboard-section-state";

/** The field that takes the presses: a bound key is looked up, not run. */
function KeyField(p: { f: KeyboardFilters; platform: KeyPlatform; onStop: () => void }): JSX.Element {
    let field: HTMLInputElement | undefined;
    const onKeyDown = (e: KeyboardEvent) => {
        const bare = !e.ctrlKey && !e.altKey && !e.metaKey;
        // A bare Tab still moves on, so the keyboard is not trapped here.
        if (e.key === "Tab" && bare) return;
        e.preventDefault();
        e.stopPropagation();
        if (bare && !e.shiftKey && e.key === "Escape") return p.onStop();
        const got = captureKey(e);
        if (!("key" in got)) return;
        const steps = p.f.key() ? p.f.key().split(" ") : [];
        p.f.setKey((steps.length >= MAX_STEPS ? [got.key] : [...steps, got.key]).join(" "));
    };
    useKeyCapture(() => field, onKeyDown);
    return (
        <TextInput
            detached
            readOnly
            ref={(el) => (field = el)}
            class="kb-key-field"
            aria-label="Keys to look for"
            placeholder="Press the keys to look for"
            value={p.f.key() ? formatKey(p.f.key(), p.platform) : ""}
        />
    );
}

/** Record keys: press a key to see the commands that have it. */
export function KeySearch(p: { f: KeyboardFilters; platform: KeyPlatform }): JSX.Element {
    const [recording, setRecording] = createSignal(false);
    return (
        <>
            <Button aria-pressed={recording()} onClick={() => setRecording(!recording())}>
                {recording() ? "Stop recording" : "Record keys"}
            </Button>
            <Show when={recording()}>
                <KeyField f={p.f} platform={p.platform} onStop={() => setRecording(false)} />
            </Show>
            <Show when={!recording() && p.f.key()}>
                <Pill tone="accent">Keys: {formatKey(p.f.key(), p.platform)}</Pill>
                <Button aria-label="Clear the key filter" onClick={() => p.f.setKey("")}>
                    Clear
                </Button>
            </Show>
        </>
    );
}
