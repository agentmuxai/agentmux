# SPEC: Agent Waiting Ambient Sound
**Status:** Proposal  
**Date:** 2026-06-19  
**Area:** Notifications / Sound / Agent Pane  

---

## 1. Problem

When an agent asks the user a question and pauses to wait for an answer, there is no auditory signal. The user may be looking at another pane, another window, or a different app entirely. A one-shot notification chime (like `agent.turn.complete`) is insufficient here because the waiting state is *sustained* — the agent remains blocked until the user responds. A looping ambient tone gives the user a continuous but non-intrusive cue that their attention is needed.

---

## 2. Goal

Play a soft, looping ambient melody while an agent pane is in the **Idle** state and the most recent turn ended with a question directed at the user. Stop it immediately when the user begins typing or submits a message. Respect all existing sound settings (master switch, volume, focus suppression).

---

## 3. Scope

**In scope:**
- New sound event: `agent.waiting.for.input`
- New looping player mode in `SoundPlayer` / new `WaitingTonePlayer`
- New `TurnPhase` guard: entering `Idle` after a turn where the agent asked a question
- New settings keys: master toggle + volume for the waiting sound
- Settings template entry with user-facing comment
- Unit tests for trigger logic and player lifecycle

**Out of scope:**
- Detecting *which* question was asked (we don't parse transcript content)
- Visual indicator changes (separate concern)
- Mobile / non-Chromium platforms (CEF-only for now)
- Asset-file version of the sound (v1 is synth-only, same as rest of sound system)

---

## 4. Trigger Definition

The ambient loop starts when **all** of the following are true:

| # | Condition | Source |
|---|-----------|--------|
| A | `turnPhase.kind === "Done"` | `AgentPaneState.turnPhase` |
| B | `turnPhase.outcome === "completed"` | same |
| C | The agent's last assistant message ends with a question (heuristic: last text block ends with `?`) | transcript tail |
| D | The composer is enabled and empty (no pending messages) | `pending.length === 0` |
| E | The pane has not received a new user submission since condition A became true | reset signal |
| F | `notify:sounds:enabled !== false` | settings |
| G | `notify:sound:agent.waiting.for.input !== false` | settings |
| H | *(Optional suppression)* pane is not the focused pane while window is active, OR `notify:sounds:suppresswhenfocused === false` | same rule as existing events |

The loop **stops** when any of the following occur:

- User types in the composer (first `input` event / first keydown in composer textarea)
- User submits a message (`TurnPhase` moves to `Submitting`)
- Agent pane is closed or navigated away
- The ambient loop setting is toggled off at runtime
- 5 minutes elapse with no user interaction (safety cutoff — prevents infinite loop if user walks away)

**Heuristic rationale for condition C:** Parsing the full transcript for semantic question detection is heavyweight. A trailing `?` on the last text block is a strong-enough proxy for v1. The worst cases (false negative: question without `?`; false positive: rhetorical `?` mid-sentence) are acceptable polish misses, not correctness bugs. This can be upgraded to a smarter detector later without changing the rest of the spec.

---

## 5. Sound Design

### 5.1 Character

- **Type:** Looping ambient tone — *not* a one-shot notification
- **Feel:** Expectant, gentle, non-intrusive. Think soft marimba, music-box bells, or a slow pentatonic pad.
- **Loop:** Seamlessly crossfaded so there is no audible click or gap at the loop boundary
- **Duration per loop:** 4–8 seconds
- **Volume:** Independent gain node, default **0.25** (lower than turn-complete at 0.6 — the loop runs longer so it should sit further back)
- **Fade in:** 400 ms linear ramp to full gain on start
- **Fade out:** 600 ms linear ramp to zero on stop (prevents click on abrupt cut)

### 5.2 v1 Synthesis (synth fallback, no audio file)

Use the Web Audio API oscillator chain, consistent with `tool-tones-player.ts`:

```
OscillatorNode (sine, ~523 Hz — C5)
    → GainNode (envelope: 400ms attack, sustain, 600ms release)
    → BiquadFilterNode (lowpass, cutoff ~1200 Hz, Q 0.7 — keeps it mellow)
    → WaitingTonePlayer masterGain
    → AudioContext.destination
```

Play a 3-note ascending arpeggio (C5 → E5 → G5) at ~1 note/sec, pause 1 sec, repeat. Each note: 300ms on, 200ms off. This is a recognizable "waiting" pattern without being jarring.

### 5.3 Future Asset (post v1)

A polished audio file (`.mp3`, gapless loop, ~6 sec) can be dropped into `public/sounds/agent-waiting.mp3`. The `SoundPlayer` registry already supports asset-path loading — add `assetPath: "sounds/agent-waiting.mp3"` to the registry entry and the synth fallback is automatically bypassed when the file loads.

---

## 6. Architecture

### 6.1 New Files

```
frontend/app/notification/sound/
├── waiting-tone-player.ts     ← new: looping oscillator chain
└── __tests__/
    └── waiting-tone-player.test.ts   ← new
```

### 6.2 Changed Files

| File | Change |
|------|--------|
| `sound-events.ts` | Add `"agent.waiting.for.input"` to `SoundEventName` union |
| `sounds.ts` | Add registry entry for `agent.waiting.for.input` |
| `sound-service.ts` | Add `mapAgentPaneEvent` case for waiting trigger; manage loop lifecycle |
| `sound-player.ts` | Add `playLooping(name)` / `stopLooping(name)` methods alongside existing one-shot `play()` |
| `agent-pane-state/types.ts` | Add `lastTurnHadQuestion: boolean` field to `AgentPaneState` |
| `agent-pane-state/reducer.ts` | Set `lastTurnHadQuestion` on `turn-ended` by inspecting transcript tail |
| `agent-pane-state-store.ts` | Emit new `AgentPaneEvent` `"waiting-for-input"` / `"waiting-ended"` |
| `frontend/types/gotypes.d.ts` | Add `"notify:sound:agent.waiting.for.input"` and `"notify:sounds:waiting:volume"` to `SettingsType` |
| `settings-template.jsonc` | Add new keys with user-facing comments |

### 6.3 New AgentPaneEvents

```typescript
// In agent-pane-state/types.ts — add to AgentPaneEvent union:
| { type: "waiting-for-input"; blockId: string }
| { type: "waiting-ended";    blockId: string; reason: "submitted" | "typing" | "timeout" | "closed" }
```

These events flow through the existing multicast extraListeners fan-out — the sound service picks them up exactly like existing events.

### 6.4 WaitingTonePlayer

```typescript
// frontend/app/notification/sound/waiting-tone-player.ts

export class WaitingTonePlayer {
    private ctx: AudioContext;
    private masterGain: GainNode;
    private running = false;
    private timeoutHandle: ReturnType<typeof setTimeout> | null = null;

    constructor(ctx: AudioContext) { ... }

    /** Start the looping arpeggio. Idempotent if already playing. */
    start(volume: number): void { ... }

    /** Fade out and stop. Returns a Promise that resolves after the fade. */
    stop(): Promise<void> { ... }

    private scheduleArpeggio(): void { ... }   // Recursive Web Audio scheduling
}
```

The player is **instantiated once** inside `sound-service.ts` alongside the existing `SoundPlayer` and `ToolTonesPlayer`, sharing the same `AudioContext`.

### 6.5 Sound Service Changes

```typescript
// sound-service.ts — additions to mapAgentPaneEvent():

case "waiting-for-input": {
    if (!shouldPlayWaiting(blockId)) break;
    const vol = getSettingsKeyAtom("notify:sounds:waiting:volume")() ?? 0.25;
    waitingPlayer.start(vol);
    // Auto-stop after 5 min
    waitingStopTimeout = setTimeout(() => waitingPlayer.stop(), 5 * 60 * 1000);
    break;
}

case "waiting-ended": {
    clearTimeout(waitingStopTimeout);
    waitingPlayer.stop();   // graceful fade-out
    break;
}
```

`shouldPlayWaiting(blockId)` reuses the existing `shouldPlay()` logic (master switch, per-event setting, focus suppression) — no new gating logic needed.

### 6.6 Reducer Changes

```typescript
// agent-pane-state/reducer.ts — on turn-ended with outcome "completed":

const lastTurnHadQuestion = detectsQuestion(state); 
// detectsQuestion: reads last text content block from transcript,
// returns true if it ends with "?"

nextState = { ...nextState, lastTurnHadQuestion };
```

And in `agent-pane-state-store.ts` dispatch fan-out:

```typescript
// After reducer returns, before notifying extraListeners:
if (
    newState.turnPhase.kind === "Idle" &&
    newState.lastTurnHadQuestion &&
    newState.pending.length === 0
) {
    extraEvents.push({ type: "waiting-for-input", blockId });
}

// When transitioning OUT of Idle (Submitting or new turn):
if (
    prevState.turnPhase.kind === "Idle" &&
    newState.turnPhase.kind !== "Idle" &&
    prevState.lastTurnHadQuestion
) {
    extraEvents.push({ type: "waiting-ended", blockId, reason: "submitted" });
}
```

The `"typing"` reason for `waiting-ended` is fired from the **composer component** directly (not the reducer), since keydown events are not in the state machine:

```typescript
// AgentComposerStrip.tsx — onInput handler:
onInput={() => {
    if (waitingActive()) {   // local signal set when waiting-for-input fires
        dispatchSoundEvent({ type: "waiting-ended", blockId, reason: "typing" });
    }
    // ... existing input handling
}}
```

---

## 7. Settings

### 7.1 New Keys

```typescript
// frontend/types/gotypes.d.ts — add to SettingsType:
"notify:sound:agent.waiting.for.input"?: boolean;   // default: true
"notify:sounds:waiting:volume"?: number;            // default: 0.25 (0–1)
```

### 7.2 Settings Template Entry

```jsonc
// settings-template.jsonc — add under notify:sound:* block:

// Play a soft looping tone while an agent is waiting for your reply.
// Set to false to disable, or adjust "notify:sounds:waiting:volume" (0–1, default 0.25).
"notify:sound:agent.waiting.for.input": true,
"notify:sounds:waiting:volume": 0.25,
```

---

## 8. Edge Cases & Invariants

| Case | Behavior |
|------|----------|
| Multiple panes simultaneously waiting | Each pane manages its own loop independently. Sound service tracks one `waitingPlayer` per blockId (Map). |
| User switches focus to waiting pane | `suppresswhenfocused` kicks in: loop fades out while pane is focused, resumes (restart from top) when focus leaves — only if `notify:sounds:suppresswhenfocused === true`. |
| Settings toggled off mid-loop | `sound-service` subscribes to the settings signal; on change to `false`, calls `waitingPlayer.stop()` for all active blockIds. |
| Agent pane closed mid-loop | Pane teardown emits `"closed"` event → `waiting-ended` with reason `"closed"` → `waitingPlayer.stop()`. |
| 5-minute safety cutoff fires | `waitingPlayer.stop()` called, loop does not restart unless a new `waiting-for-input` event is emitted. |
| Question ends in `?"` or `?'` or `?)` | `detectsQuestion` should trim trailing punctuation/quotes before checking the final char. |
| Last assistant block is a tool result, not text | `detectsQuestion` reads only `type === "text"` blocks — tool result blocks are skipped. |
| AudioContext suspended (browser autoplay policy) | WaitingTonePlayer checks `ctx.state` and calls `ctx.resume()` before scheduling; same pattern as existing sound system. |
| Loop already playing when new `waiting-for-input` fires (same pane) | `WaitingTonePlayer.start()` is idempotent — no-op if already running. |

---

## 9. Tests

### 9.1 `waiting-tone-player.test.ts`
- `start()` schedules oscillator nodes on AudioContext mock
- `stop()` triggers gain ramp to 0 and resolves after fade
- `start()` is idempotent (calling twice doesn't double-schedule)
- 5-minute auto-stop fires `stop()` (fake timers)

### 9.2 `sound-service.test.ts` additions
- `waiting-for-input` event → `waitingPlayer.start()` called with correct volume
- `waiting-ended` event → `waitingPlayer.stop()` called
- Master `notify:sounds:enabled = false` → `waitingPlayer.start()` not called
- `notify:sound:agent.waiting.for.input = false` → not called
- Focus suppression: pane focused + `suppresswhenfocused = true` → not called

### 9.3 `reducer.test.ts` additions
- `turn-ended` with `outcome = "completed"` + last text block ends with `?` → `lastTurnHadQuestion = true`
- `turn-ended` with last text block not ending with `?` → `lastTurnHadQuestion = false`
- `turn-ended` with `outcome = "errored"` → `lastTurnHadQuestion = false` regardless

---

## 10. Implementation Order

1. **`waiting-tone-player.ts`** — self-contained, no dependencies, testable in isolation
2. **Settings keys** — `gotypes.d.ts` + `settings-template.jsonc`
3. **`sound-events.ts` + `sounds.ts`** — add event name to registry
4. **`agent-pane-state/types.ts`** — add `lastTurnHadQuestion`, new event types
5. **`agent-pane-state/reducer.ts`** — `detectsQuestion` + set field on `turn-ended`
6. **`agent-pane-state-store.ts`** — emit `waiting-for-input` / `waiting-ended` events
7. **`sound-player.ts`** — `playLooping` / `stopLooping` stubs (or delegate fully to `WaitingTonePlayer`)
8. **`sound-service.ts`** — wire new event cases + manage player lifecycle
9. **`AgentComposerStrip.tsx`** — fire `waiting-ended` on first keydown
10. **Tests** for each layer

---

## 11. Non-Goals (Explicit)

- **Voice / speech synthesis** — out of scope; this is purely a tone.
- **Per-agent sound customization** — all agents share the same waiting sound in v1.
- **Transcript semantic parsing** — the `?` heuristic is intentionally dumb; NLP is a follow-up.
- **Notification badge / visual** — orthogonal, tracked separately.
- **Desktop OS notification** — the existing notification system covers that; this spec is in-app audio only.
