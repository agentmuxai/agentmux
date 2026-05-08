# Implementation Spec: Voice Input — Web Speech API (Phase 1)

**Date:** 2026-05-08  
**Status:** Draft  
**Parent:** `SPEC_VOICE_INPUT_2026_05_08.md`  
**Effort:** ~2–3 days, frontend-only, no Rust changes

---

## Scope

Implement Ctrl+Shift+M voice input using the browser-native `webkitSpeechRecognition` API. Transcript is injected into whichever agent pane was last focused. No auto-send; the user presses Enter as usual.

---

## Files

| Action | Path |
|--------|------|
| **Create** | `frontend/app/hook/useVoiceInput.ts` |
| **Create** | `frontend/app/view/agent/components/MicButton.tsx` |
| **Create** | `frontend/app/view/agent/components/MicButton.scss` |
| **Modify** | `frontend/app/view/agent/components/AgentFooter.tsx` |
| **Modify** | `frontend/app/view/agent/agent-view.tsx` |
| **Modify** | `frontend/app/app-init.ts` |

---

## 1. `frontend/app/hook/useVoiceInput.ts`

Module-level singleton. One `SpeechRecognition` instance exists for the entire app lifetime. Multiple `AgentFooter` instances register/deregister a handle; the last-focused one receives transcript.

### Type definitions

```ts
/** Provided by each AgentFooter on focus. Tells the voice session where to write. */
export interface PaneVoiceHandle {
    /** Append committed (final) transcript at current cursor position in the textarea. */
    appendFinal: (text: string) => void;
    /** Replace the in-progress interim suffix in the textarea. Pass "" to clear. */
    setInterim: (text: string) => void;
}

export interface VoiceSession {
    isListening: () => boolean;
    isAvailable: () => boolean;         // false when SpeechRecognition absent in this runtime
    toggleListening: () => void;
    /** Called by AgentFooter on focus to become the transcript target. */
    registerPane: (handle: PaneVoiceHandle) => void;
}
```

### Singleton implementation

```ts
import { createSignal } from "solid-js";

const RESTART_DELAY_MS = 100;   // pause before auto-restarting after onend

function createVoiceSession(): VoiceSession {
    const SR = (window as any).SpeechRecognition ?? (window as any).webkitSpeechRecognition;
    const [isListening, setIsListening] = createSignal(false);

    if (!SR) {
        return {
            isListening: () => false,
            isAvailable: () => false,
            toggleListening: () => {},
            registerPane: () => {},
        };
    }

    const recognition: SpeechRecognition = new SR();
    recognition.continuous = true;
    recognition.interimResults = true;
    recognition.lang = navigator.language;  // honour OS locale

    let activeHandle: PaneVoiceHandle | null = null;
    let interimActive = false;      // whether we've written interim text to the textarea

    recognition.onresult = (event: SpeechRecognitionEvent) => {
        let interim = "";
        let finals = "";

        // Only process results from this event batch (resultIndex onward).
        for (let i = event.resultIndex; i < event.results.length; i++) {
            const result = event.results[i];
            if (result.isFinal) {
                finals += result[0].transcript;
            } else {
                interim += result[0].transcript;
            }
        }

        if (finals) {
            // Clear any interim suffix first, then append final text with a
            // leading space (unless the textarea is empty).
            activeHandle?.setInterim("");
            interimActive = false;
            activeHandle?.appendFinal(finals);
        }

        if (interim) {
            activeHandle?.setInterim(interim);
            interimActive = true;
        } else if (!finals) {
            // No new content in this event — clear stale interim.
            activeHandle?.setInterim("");
            interimActive = false;
        }
    };

    recognition.onerror = (event: SpeechRecognitionErrorEvent) => {
        if (event.error === "not-allowed" || event.error === "service-not-allowed") {
            // Permission denied — surface to user via a toast (see §5).
            setIsListening(false);
            window.dispatchEvent(new CustomEvent("voice-input-error", { detail: event.error }));
        }
        // "no-speech" and "aborted" are non-fatal; onend will restart if still listening.
    };

    recognition.onend = () => {
        // Web Speech API stops automatically after a long pause or on some
        // browser builds after every utterance. Auto-restart if the user hasn't
        // toggled off.
        if (isListening()) {
            setTimeout(() => {
                if (isListening()) {
                    try { recognition.start(); } catch { /* already started race */ }
                }
            }, RESTART_DELAY_MS);
        }
    };

    const toggleListening = () => {
        if (isListening()) {
            recognition.stop();
            if (interimActive) {
                activeHandle?.setInterim("");
                interimActive = false;
            }
            setIsListening(false);
        } else {
            try {
                recognition.start();
                setIsListening(true);
            } catch {
                // Already started (race with onend auto-restart) — ignore.
            }
        }
    };

    return {
        isListening,
        isAvailable: () => true,
        toggleListening,
        registerPane: (handle) => { activeHandle = handle; },
    };
}

// Module-level singleton — created once on first import.
let _session: VoiceSession | null = null;

export function getVoiceSession(): VoiceSession {
    if (!_session) _session = createVoiceSession();
    return _session;
}
```

**Key decisions:**

- `continuous: true` — keeps the session alive across pauses without requiring the user to click again.
- `interimResults: true` — partial words appear as the user speaks.
- `registerPane` does not deregister on blur. If the user clicks the title bar or a scrollbar, the voice target stays on the last agent pane. It is only replaced when another agent pane gains focus.
- Auto-restart on `onend` with a 100 ms delay avoids an infinite restart loop when the user calls `recognition.stop()`.

---

## 2. `frontend/app/view/agent/components/MicButton.tsx`

Uses the existing `ToggleIconButton` from `frontend/app/element/iconbutton.tsx`, which already supports an `active()` signal.

```tsx
import { ToggleIconButton } from "@/app/element/iconbutton";
import { getVoiceSession } from "@/app/hook/useVoiceInput";
import { Show, JSX } from "solid-js";
import "./MicButton.scss";

export function MicButton(): JSX.Element {
    const voice = getVoiceSession();

    return (
        <Show when={voice.isAvailable()}>
            <div class="mic-button-wrap" classList={{ "mic-active": voice.isListening() }}>
                <ToggleIconButton
                    decl={{
                        elemtype: "toggleiconbutton",
                        icon: "regular@microphone",
                        title: "Voice input (Ctrl+Shift+M)",
                        active: voice.isListening,
                        click: voice.toggleListening,
                    }}
                />
            </div>
        </Show>
    );
}
```

---

## 3. `frontend/app/view/agent/components/MicButton.scss`

```scss
.mic-button-wrap {
    display: flex;
    align-items: center;

    &.mic-active .wave-iconbutton {
        color: var(--color-primary);
        position: relative;

        &::after {
            content: "";
            position: absolute;
            inset: -3px;
            border-radius: 50%;
            border: 1.5px solid var(--color-primary);
            opacity: 0.6;
            animation: mic-pulse 1.4s ease-in-out infinite;
        }
    }
}

@keyframes mic-pulse {
    0%, 100% { transform: scale(1);   opacity: 0.6; }
    50%       { transform: scale(1.3); opacity: 0;   }
}
```

---

## 4. `frontend/app/view/agent/components/AgentFooter.tsx`

### 4a. New prop

Add one optional prop to `AgentFooterProps` (line 187):

```ts
interface AgentFooterProps {
    agentId: string;
    onSendMessage?: (message: string) => void | Promise<void>;
    onTyping?: () => void;
    onStopAgent?: () => void;
    getCompletions?: (prefix: string) => SlashCommand[];
    // NEW:
    onFocused?: () => void;   // called when this pane becomes the voice target
}
```

### 4b. Voice handle registration

Inside `AgentFooter` component body, after `let textareaRef`:

```ts
import { getVoiceSession } from "@/app/hook/useVoiceInput";
import { MicButton } from "./MicButton";

// ── Voice input ───────────────────────────────────────────────────────────
const voice = getVoiceSession();

// Track committed text length so interim can be appended/replaced without
// disturbing text the user typed manually.
let voiceBaseLength = 0;  // length of textareaRef.value when this utterance started

const voiceHandle = {
    appendFinal: (text: string) => {
        if (!textareaRef) return;
        // Ensure there is a space separator unless the textarea is empty.
        const current = textareaRef.value;
        const separator = current.length > 0 && !current.endsWith(" ") ? " " : "";
        textareaRef.value = current + separator + text.trimStart();
        voiceBaseLength = textareaRef.value.length;
        props.onTyping?.();
    },
    setInterim: (text: string) => {
        if (!textareaRef) return;
        // Replace everything after voiceBaseLength with the interim text.
        const base = textareaRef.value.slice(0, voiceBaseLength);
        textareaRef.value = text ? base + " " + text : base;
        props.onTyping?.();
    },
};

const handleFocus = () => {
    voice.registerPane(voiceHandle);
    // Reset base to current length so interim appends after existing text.
    voiceBaseLength = textareaRef?.value.length ?? 0;
    props.onFocused?.();
};
```

### 4c. Wire focus and MicButton into the JSX

The current JSX (lines 373–393):

```tsx
<div class="agent-input-container">
    <Show when={autocompletePrefix() !== null && completions().length > 0}>
        <SlashAutocomplete ... />
    </Show>
    <textarea
        ref={textareaRef}
        class="agent-input"
        placeholder={`Send message to ${props.agentId}...`}
        onKeyDown={handleKeyDown}
        onInput={handleInput}
        rows={1}
    />
    <div class="agent-input-hint">
        <span>Enter to send • Shift+Enter for newline • Esc to clear / stop</span>
    </div>
</div>
```

Replace with:

```tsx
<div class="agent-input-container" onFocusIn={handleFocus}>
    <Show when={autocompletePrefix() !== null && completions().length > 0}>
        <SlashAutocomplete ... />
    </Show>
    <div class="agent-input-row">
        <MicButton />
        <textarea
            ref={textareaRef}
            class="agent-input"
            classList={{ "voice-active": voice.isListening() }}
            placeholder={`Send message to ${props.agentId}...`}
            onKeyDown={handleKeyDown}
            onInput={(e) => {
                // User typed: update voiceBaseLength so interim appends correctly.
                voiceBaseLength = (e.target as HTMLTextAreaElement).value.length;
                handleInput(e);
            }}
            rows={1}
        />
    </div>
    <div class="agent-input-hint">
        <span>Enter to send • Shift+Enter for newline • Esc to clear / stop</span>
    </div>
</div>
```

**`onFocusIn`** bubbles from any child click (including the textarea and the mic button), so a single handler covers the whole footer area.

**`classList={{ "voice-active": voice.isListening() }}`** on the textarea lets CSS add a subtle pulse border while the mic is active:

```scss
.agent-input.voice-active {
    border-color: var(--color-primary);
    box-shadow: 0 0 0 1px var(--color-primary);
    transition: border-color 0.2s, box-shadow 0.2s;
}
```

Add this to the existing agent SCSS file.

### 4d. `voiceBaseLength` reset on send

In `handleSend` (line 291), after clearing the textarea, reset the base:

```ts
const handleSend = () => {
    if (!textareaRef) return;
    const message = textareaRef.value;
    if (!message.trim()) return;
    if (props.onSendMessage) {
        props.onSendMessage(message);
        textareaRef.value = "";
        voiceBaseLength = 0;    // ← add this
        // ... rest unchanged
    }
};
```

---

## 5. `frontend/app/view/agent/agent-view.tsx`

No structural changes needed. The `onFocusIn` handler inside `AgentFooter` handles pane registration automatically.

However, wire the `onFocused` prop if any parent-level behaviour is needed (currently none required). Leave the `AgentFooter` call at line 719 as-is for now — `onFocused` is optional.

---

## 6. `frontend/app/app-init.ts`

Install the global Ctrl+Shift+M keybinding once at startup:

```ts
import { getVoiceSession } from "@/app/hook/useVoiceInput";

// Inside initApp() or at module level after imports:
document.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.ctrlKey && e.shiftKey && e.key === "M") {
        e.preventDefault();
        getVoiceSession().toggleListening();
    }
});
```

Also listen for the `voice-input-error` event dispatched from the session on permission denial:

```ts
window.addEventListener("voice-input-error", (e: Event) => {
    const error = (e as CustomEvent<string>).detail;
    if (error === "not-allowed" || error === "service-not-allowed") {
        // Replace with whatever toast/notification system the app uses.
        console.warn("[voice] Microphone permission denied");
        // TODO: surface toast: "Microphone access denied — allow it in your OS sound settings"
    }
});
```

---

## 7. Interim text display strategy

**Decision: write-through to textarea value, no overlay.**

Rationale: a sibling overlay `<div>` must match the textarea's exact font, line-height, padding, scrollTop, and wrapping — fragile and requires synchronous layout reads (which the existing footer deliberately avoids; see comment at `AgentFooter.tsx:219–233`). The write-through approach writes directly to `textareaRef.value`:

```
"fix the bug in " ← voiceBaseLength = 17 (user typed)
"fix the bug in main.rs" ← interim (main.rs = in-progress word)
"fix the bug in main.rs" ← final — voiceBaseLength advances to 23
```

If the user types while interim is showing, `onInput` updates `voiceBaseLength` to the new `.value.length`, effectively committing any interim text as-is (it becomes part of the base). This is correct — the user's editing intent takes precedence.

**Visual cue:** The `voice-active` class on the textarea and the pulse ring on the MicButton are sufficient for the user to know voice is active. Dimming the interim text is deferred to a future iteration.

---

## 8. CEF microphone permission

CEF prompts the user for microphone access via the standard Chromium permission UI on first use. The permission is stored in the CEF user-data profile per version directory (`~/.agentmux/versions/<v>/`).

No additional CEF configuration is required. `navigator.mediaDevices` and `webkitSpeechRecognition` are available in CEF by default.

If access is denied, `recognition.onerror` fires with `error: "not-allowed"`. The session catches this, emits `voice-input-error`, and the keybinding stops responding until the app restarts (CEF caches the denial; the user must clear it in OS sound settings).

---

## 9. State machine

```
IDLE
 │  Ctrl+Shift+M  /  MicButton click
 ▼
LISTENING
 │  onresult (interim)  →  setInterim(text) on active pane
 │  onresult (final)    →  appendFinal(text), setInterim("")
 │  onend               →  auto-restart after 100 ms (recognition.onend handler)
 │  Ctrl+Shift+M  /  MicButton click
 ▼
IDLE (recognition.stop(); setInterim("") to clear any dangling interim)
```

Error transitions:

```
LISTENING
 │  onerror "not-allowed"   →  IDLE + emit voice-input-error
 │  onerror "no-speech"     →  (stay LISTENING; onend will auto-restart)
 │  onerror "aborted"       →  (stay LISTENING; onend will auto-restart)
```

---

## 10. Edge cases

| Scenario | Handling |
|----------|----------|
| User switches to terminal pane | Terminal has no `onFocusIn` → `registerPane` not called → `activeHandle` stays on last agent pane → transcript still goes there |
| User clicks title bar / scrollbar | `onFocusIn` not triggered → handle unchanged → correct |
| Two agent panes side-by-side | Whichever was clicked last holds `activeHandle`; the other pane is unaffected |
| User types mid-utterance | `onInput` updates `voiceBaseLength` to current length; interim is appended after what the user typed |
| User presses Esc to clear | `handleKeyDown` sets `textareaRef.value = ""`, then `voiceBaseLength = 0` (add this reset in the Esc branch alongside the existing clear) |
| Send while listening | `handleSend` clears textarea and resets `voiceBaseLength = 0`; voice session keeps running; next words go into the now-empty textarea |
| No `SpeechRecognition` API | `isAvailable()` returns `false`; `MicButton` is hidden; Ctrl+Shift+M is a no-op |
| Rapid pane switches during an utterance | The final `onresult` fires on whichever pane is registered at that moment (the newest focused one) |

---

## 11. TypeScript — `SpeechRecognition` types

The Web Speech API types are not in `lib.dom.d.ts` by default in all TypeScript versions. Add a declaration file:

**`frontend/types/speech.d.ts`** (new file):

```ts
interface SpeechRecognitionEvent extends Event {
    readonly resultIndex: number;
    readonly results: SpeechRecognitionResultList;
}

interface SpeechRecognitionErrorEvent extends Event {
    readonly error: string;
    readonly message: string;
}

interface SpeechRecognition extends EventTarget {
    continuous: boolean;
    interimResults: boolean;
    lang: string;
    start(): void;
    stop(): void;
    abort(): void;
    onresult: ((event: SpeechRecognitionEvent) => void) | null;
    onerror: ((event: SpeechRecognitionErrorEvent) => void) | null;
    onend: (() => void) | null;
}

declare var SpeechRecognition: { new(): SpeechRecognition };
declare var webkitSpeechRecognition: { new(): SpeechRecognition };
```

---

## 12. Implementation order

1. **`speech.d.ts`** — types first so the rest compiles.
2. **`useVoiceInput.ts`** — singleton with stub `registerPane` (no real handle wired yet). Verify Ctrl+Shift+M toggles `isListening` in devtools console.
3. **`MicButton.tsx` + `MicButton.scss`** — render in footer, verify pulse animation.
4. **`AgentFooter.tsx`** — `onFocusIn`, `voiceHandle`, `voiceBaseLength` tracking, `voice-active` class on textarea.
5. **`app-init.ts`** — global keybinding + error listener.
6. **Manual test:**
   - Open two agent panes.
   - Ctrl+Shift+M → mic button pulses.
   - Speak → words appear in the focused pane.
   - Click the other pane → speak → words appear there.
   - Press Enter → sends, textarea clears, mic stays on.
   - Ctrl+Shift+M → stops.
