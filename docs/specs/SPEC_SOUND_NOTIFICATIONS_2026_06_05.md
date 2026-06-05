# SPEC — Sound notifications subsystem

**Status:** Draft v1 — for review
**Date:** 2026-06-05
**Author:** agent2
**First use case:** play a "ding" when the agent pane's turn completes.
**Scope:** frontend-only (no Rust changes). Generalizes from the first use case to a registry-driven system that any pane / store / saga can opt into.

**Related:**
- `frontend/app/store/agent-pane-state/types.ts` (lines 480–605 — `AgentPaneEvent` union, source of the `turn-ended` event we react to)
- `frontend/app/store/agent-pane-state-store.ts` (lines 78–89, 172 — current single-sink event delivery; we extend it to multicast)
- `frontend/app/store/command-source.ts` (the global dispatch ring buffer we **do not** use; rationale in §6.2)
- `frontend/app/store/global.ts` (lines 91, 129, 459–470 — `settingsAtom`, `notifications`, `getSettingsKeyAtom` we plug into)
- `frontend/app/notification/usenotification.tsx` (the visual notification framework we coexist with — sound is additive, not a replacement)
- `frontend/app/store/focusManager.ts` + `frontend/util/focusutil.ts` (pane-focus signal used to suppress sound when the user is already looking at the pane)
- `agentmux-srv/src/backend/wconfig/types.rs` (lines 200–225 — backend settings struct we extend with the new `notify:sounds:*` keys)
- `frontend/types/gotypes.d.ts` (lines 1287–1351 — TS counterpart of the settings struct)
- `settings-template.jsonc` (the user-facing commented template we add a `-- Notification sounds --` section to)
- MDN — Web Audio API best practices (informs the `AudioBufferSourceNode`-based player choice)
- WCAG 2.2 SC 1.4.2 *Audio Control* (informs the mute / volume / per-event-disable controls)

---

## 0. TL;DR

Add a typed sound-event bus and a small `SoundService` that subscribes to reducer events, applies user settings + suppression rules, and plays a short SFX via the Web Audio API. First wired event: `agent.turn.complete` (fired on the existing `turn-ended` reducer event, classified by `TurnOutcome` so "completed" / "stopped" / "errored" can ship as distinct sounds in v2). The system is **registry-driven** — adding a new sound elsewhere in the app is a 3-line patch to `sounds.ts` plus one `notify()` call at the emit site. No churn through the existing 4-layer reducer stack; multicast plumbing is a small, additive change to the per-slice `setEventSink` contract.

The architecture choices are deliberate and small:

1. **Web Audio API over `<audio>`** for short SFX (lower latency, no `cloneNode` quirks, zero allocation per playback once decoded).
2. **Multicast event bus on top of the existing per-slice sinks** (additive — current callers keep working; no refactor of the reducer stack).
3. **Registry-driven sound IDs** so any code path can fire a sound without touching the player.
4. **Settings live in the same `wconfig` schema** as the rest of AgentMux — no parallel settings system.
5. **Visual fallback via the existing `notifications` atom** so deaf / muted users still get the signal (WCAG 1.4.2).
6. **Synthesized fallback** so the system is functional with zero shipped audio assets — the asset pack is purely a polish layer.

---

## 1. Why

Long-horizon agent runs are exactly the workflow where the user **leaves the app focused on something else** between turns (reading a doc, in another tab, in the kitchen). Today the only "turn done" signal is the visual one inside the pane: the working spinner stops, "Worked" appears in the composer strip. If the user isn't looking at the pane, the signal is invisible.

A small, ergonomic sound — *ding* when the turn finishes — closes that gap. The user can context-switch away and trust they'll be summoned back exactly when needed.

Once that exists, the same hook is useful for:
- agent error (`Done.errored`, stream stalled, submit timed out)
- pending-message acceptance / rejection (currently silent — the amber→accent color shift in the pane is the only feedback)
- background saga completions (package build done, deploy finished — future)
- drone-run state changes (currently a separate slice with its own reducer/sink — the bus is slice-agnostic, so this drops in for free)

We design for the general case from day one. A bespoke `playDingOnTurnEnd()` hack inside `useAgentStream` would solve the immediate ask but pay for itself zero times the next call.

---

## 2. Non-goals

- **No sound *generation*.** v1 ships either a small CC0 asset pack or a synthesized oscillator fallback (§7.4). No DSP, no procedural composition, no theme editor.
- **No backend sound dispatch.** The sidecar / launcher have no UI focus or audio device — sound is a renderer concern.
- **No system tray / OS notification integration.** That is `Notification`-API territory and orthogonal — pursued in a follow-on if user feedback wants it.
- **No multi-instance audio coordination.** Each AgentMux instance has its own renderer + AudioContext; nothing tries to dedupe "two windows both finished a turn at the same time." If the user finds this annoying, we'll add a window-leader gate later.
- **No backwards-compat shim for the existing single-sink callers.** §6.1 keeps them working as-is; only the new bus is added.
- **No background-music or long-form audio.** Strictly one-shot SFX.

---

## 3. What's in place today (verified against current `main`)

### 3.1 Reducer events are already the right shape

`AgentPaneEvent` (`frontend/app/store/agent-pane-state/types.ts:480–605`) is a discriminated union with stable type literals: `turn-ended`, `turn-started`, `stream-stalled`, `submit-timed-out`, `interrupt-timed-out`, `pending-accepted`, `pending-rejected`, `pending-expired`, `stream-stuck`, etc. These are **already** emitted on every state transition we'd want a sound for. The reducer is the single source of truth and the event is the audit-friendly form. We do not need a parallel "fire a sound" command; we just need a subscriber.

`agent-pane-state-store.ts:172` fans every emitted event into a single global `eventSink` function (default: warn-logger). The browser, editor, and drone slices follow the same pattern. The sink slot is "last-writer-wins" with idempotent install guards — see `browser-model.ts:installEventSinkOnce` and the comment block above it. This was fine when each slice had **one** consumer (the side-effecting bridge between reducer and DOM); it becomes a problem the moment a second consumer (us) wants in.

### 3.2 What does **not** carry the outcome today

The `turn-ended` event payload is `{ type, statsMerged, stoppingCleared }` — no `outcome` field, even though `state.turnPhase` is by that point `{ kind: "Done", outcome, finishedAt }` (see `reducer.ts:405–432`). The sound service needs the outcome to pick the right sound (`completed` vs `stopped` vs `errored` vs `interrupted`).

Two options:

| | snapshot the slot to read `turnPhase.outcome` | enrich the event payload |
|---|---|---|
| Coupling | sound service knows about the store API (`snapshot(blockId)`) | reducer carries one more field — no extra API surface |
| Reentrancy | safe — sink runs after state mutation (line 137 sets `slot.state = result.state` before line 172 fans events) | safe |
| Reusability | every other consumer pays the same coupling | every consumer reads the event directly |
| Audit | snapshot read is invisible in the dispatch ring | outcome is logged with the event in `recordDispatch` |

**Recommendation: enrich the event payload.** It is a one-line type extension and a one-line reducer change, and it makes the audit ring and any future telemetry pipeline strictly better. Concretely:

```ts
// types.ts — was
| { type: "turn-ended"; statsMerged: boolean; stoppingCleared: boolean }
// types.ts — becomes
| { type: "turn-ended"; outcome: TurnOutcome; statsMerged: boolean; stoppingCleared: boolean }
```

```ts
// reducer.ts TurnEnd arm — was
events: [{ type: "turn-ended", statsMerged: merged != null, stoppingCleared: stoppingWasSet }]
// becomes
events: [{ type: "turn-ended", outcome, statsMerged: merged != null, stoppingCleared: stoppingWasSet }]
```

The reducer already computes `outcome` two lines earlier (line 405). No new control flow.

### 3.3 No audio infrastructure exists

`/assets/`, `/public/`, `/frontend/assets/` — no `.mp3`, `.wav`, `.ogg`. No `AudioContext` / `HTMLAudioElement` / `playSound` / `howler` references in the frontend. No CEF audio flags in `agentmux-cef/src/`. The slate is clean and the design choice is unconstrained.

### 3.4 Settings plumbing is already turn-key

`getSettingsKeyAtom<T extends keyof SettingsType>(key)` (`global.ts:459`) returns a memoized SolidJS signal that re-renders on settings file change. Adding a new key is purely: edit `agentmux-srv/src/backend/wconfig/types.rs` + `frontend/types/gotypes.d.ts` (TS types) + `settings-template.jsonc` (the commented user-facing template). No frontend store wiring required.

### 3.5 Visual notifications exist; sound is additive

`atoms.notifications` + `setNotifications` (`global.ts:129`) drive the toast UI (`frontend/app/notification/notificationbubbles.tsx`). The sound service can optionally push a matching toast for any sound event when the user has muted audio or has accessibility settings on — that's our WCAG 1.4.2 visual-equivalent path. It is **not** required for v1 — the spinner stopping + "Worked" label is already a visual signal — but the hook is in place if we want belt-and-suspenders.

### 3.6 Pane focus tracking exists; window focus doesn't

`focusManager.blockFocusAtom()` (`focusManager.ts:11–19`) is a reactive accessor for "which blockId currently holds DOM focus." We can use this to **suppress** a sound when the originating pane is focused (the user is already watching — no audio summons needed). There is **no** existing app-level "is this window in the foreground" signal; `document.visibilityState` and `document.hasFocus()` are available but only consulted ad-hoc (one MyAgentsList refetch handler + one command-registry guard — `command-registry.ts:410`). v1 adds a small focus-state signal under `frontend/app/window/` for the sound service to read.

---

## 4. Design — sound notifications subsystem

### 4.1 Module layout

```
frontend/app/notification/sound/
    sounds.ts             # The registry — sound IDs + default asset + classification
    sound-events.ts       # Typed multicast event bus + helpers
    sound-player.ts       # Web Audio API: AudioContext owner, AudioBuffer cache, play()
    sound-service.ts      # Orchestrator: subscribes, reads settings, suppresses, calls player
    synth-fallback.ts     # Oscillator-based fallback when asset missing or pack disabled
    index.ts              # Bootstrap — installed once at app init
```

Co-located with `frontend/app/notification/` because the user-mental-model of "what counts as a notification" is the same — they're going to expect to find sound settings near the existing notification settings.

### 4.2 The registry — `sounds.ts`

```ts
/** Stable string IDs. Keep kebab-case; namespaced by emitting subsystem. */
export type SoundId =
    | "agent.turn.complete"
    | "agent.turn.error"
    | "agent.turn.interrupted"
    | "agent.message.accepted"
    | "agent.message.rejected"
    | "agent.stream.stalled";

export type SoundCategory = "success" | "info" | "warning" | "error";

export interface SoundDef {
    id: SoundId;
    /** Default user-visible label in settings UI. */
    label: string;
    /** Used by the synth fallback when no asset is present. */
    category: SoundCategory;
    /** Path under `public/sounds/`. Optional — synth fallback kicks in if absent. */
    asset?: string;
    /** Setting key for per-event enable. Conventionally `notify:sound:<id>`. */
    settingKey: keyof SettingsType;
    /** Default volume, 0–1. Honored only if the user hasn't overridden it. */
    defaultVolume?: number;
    /**
     * Coalesce window. Two firings of the same sound within this many ms
     * play once. Default 300ms — covers double-dispatch storms during the
     * `turn-ended` ↔ `stream-unsubscribed` race without dropping legit
     * back-to-back signals.
     */
    coalesceMs?: number;
}

export const SOUNDS: Record<SoundId, SoundDef> = {
    "agent.turn.complete": {
        id: "agent.turn.complete",
        label: "Agent turn completed",
        category: "success",
        asset: "sounds/turn-complete.ogg",
        settingKey: "notify:sound:agent.turn.complete",
        coalesceMs: 300,
    },
    // …rest defined the same way.
};
```

The whole registry is a constant. Adding a new sound = one entry + one settings key in the schema. No imperative registration step.

### 4.3 The event bus — `sound-events.ts`

```ts
export interface SoundEvent {
    id: SoundId;
    /** Optional: blockId of the originating pane (for focus suppression). */
    sourceBlockId?: string;
    /** Optional: free-form override of the default sound — alternate asset id, gain, pan. */
    override?: { asset?: string; gain?: number };
}

type Listener = (ev: SoundEvent) => void;
const listeners = new Set<Listener>();

export function subscribeSoundEvents(listener: Listener): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
}

export function notify(id: SoundId, opts?: Omit<SoundEvent, "id">): void {
    const ev: SoundEvent = { id, ...opts };
    for (const l of listeners) {
        try { l(ev); } catch (e) { console.warn("[sound] listener threw", e); }
    }
}
```

That's the entire generalization surface. **Any code anywhere** in the frontend can call `notify("agent.turn.complete", { sourceBlockId })` and the sound service will (or won't, per settings) play it. No SolidJS coupling, no provider context, no DI graph.

### 4.4 Wiring the reducer events into the bus

Two integration paths, ordered cheapest-first:

**Path A — explicit `notify()` at the dispatch call site** (used for v1's `turn-ended`):

```ts
// frontend/app/view/agent/useAgentStream.ts, inside finalizeTurn(), AFTER the
// dispatchPane({ type: "TurnEnd", stats }) call:
notify("agent.turn.complete", { sourceBlockId: blockId });
```

Pros: zero plumbing. Cons: each emit site must remember to call it, and the dispatch may be replayed in a session-replay context that **shouldn't** play sound (we only want sounds for live events).

**Path B — multicast on the reducer event sink** (used for everything else):

Extend `agent-pane-state-store.ts` (and any other slice we want bus integration for) so it can have N listeners in addition to the existing single sink. Minimal diff:

```ts
// agent-pane-state-store.ts — additive, no callsite changes
const extraListeners = new Set<EventSink>();
export function addEventListener(sink: EventSink): () => void {
    extraListeners.add(sink);
    return () => extraListeners.delete(sink);
}
// inside dispatch(), after the existing single-sink fan-out:
for (const ev of result.events) {
    eventSink(blockId, ev);
    for (const l of extraListeners) {
        try { l(blockId, ev); } catch (e) { console.warn("[pane-state] listener threw", e); }
    }
}
```

Sound service then installs a single listener that maps event types → `notify()` calls:

```ts
addEventListener((blockId, ev) => {
    switch (ev.type) {
        case "turn-ended":
            if (ev.outcome === "completed") notify("agent.turn.complete", { sourceBlockId: blockId });
            else if (ev.outcome === "errored") notify("agent.turn.error", { sourceBlockId: blockId });
            else if (ev.outcome === "interrupted" || ev.outcome === "stopped")
                notify("agent.turn.interrupted", { sourceBlockId: blockId });
            return;
        case "submit-timed-out":
        case "stream-stalled":
            notify("agent.turn.error", { sourceBlockId: blockId });
            return;
        case "pending-accepted":
            notify("agent.message.accepted", { sourceBlockId: blockId });
            return;
        case "pending-rejected":
            notify("agent.message.rejected", { sourceBlockId: blockId });
            return;
    }
});
```

**For v1 we use Path B**, hooked off the multicast sink. It naturally covers every outcome and other future event types without per-callsite changes. Path A stays in the toolbox for "fire from a hand-coded saga, not a reducer event" cases.

The `outcome` field on `turn-ended` is the only reducer change needed for this whole spec (see §3.2).

### 4.5 The player — `sound-player.ts`

Modeled directly on the MDN best-practices: one `AudioContext` per renderer, decoded `AudioBuffer`s per sound ID, fresh `AudioBufferSourceNode` per play. Synth fallback (`synth-fallback.ts`) runs on the same context.

```ts
class SoundPlayer {
    private ctx: AudioContext | null = null;
    private buffers = new Map<SoundId, AudioBuffer>();
    private masterGain: GainNode | null = null;

    /** Call inside a user gesture handler (any keydown / pointerdown in the app shell). */
    async prime(): Promise<void> {
        if (this.ctx) return;
        this.ctx = new (window.AudioContext || (window as any).webkitAudioContext)();
        this.masterGain = this.ctx.createGain();
        this.masterGain.connect(this.ctx.destination);
        await Promise.all(
            Object.values(SOUNDS)
                .filter((s) => s.asset)
                .map((s) => this.loadAsset(s)),
        );
    }

    async play(def: SoundDef, gain = 1): Promise<void> {
        const ctx = this.ctx;
        if (!ctx || !this.masterGain) return;
        if (ctx.state === "suspended") await ctx.resume();
        const buf = this.buffers.get(def.id);
        if (buf) {
            const src = ctx.createBufferSource();
            src.buffer = buf;
            const g = ctx.createGain();
            g.gain.value = gain;
            src.connect(g).connect(this.masterGain);
            src.start();
            return;
        }
        // Synth fallback — small, deterministic, no asset required.
        playSynthFallback(ctx, this.masterGain, def.category, gain);
    }

    setMasterGain(value: number) {
        if (this.masterGain) this.masterGain.gain.value = value;
    }

    private async loadAsset(def: SoundDef) {
        try {
            const resp = await fetch(`/${def.asset}`);
            const bytes = await resp.arrayBuffer();
            const buf = await this.ctx!.decodeAudioData(bytes);
            this.buffers.set(def.id, buf);
        } catch (e) {
            console.warn(`[sound] failed to load asset for ${def.id}, will use synth fallback`, e);
        }
    }
}
```

The synth fallback is a 4-line oscillator (`OscillatorNode` + `GainNode` with a short attack/decay envelope) per category. See `synth-fallback.ts` sketch in Appendix A.

### 4.6 The orchestrator — `sound-service.ts`

```ts
const player = new SoundPlayer();
const lastFiredAt = new Map<SoundId, number>();
let focusedBlock: () => string | null;     // bound from focusManager
let isWindowFocused: () => boolean;         // bound from new window-focus signal

export function installSoundService() {
    focusedBlock = focusManager.blockFocusAtom;
    isWindowFocused = makeWindowFocusSignal();   // §4.7

    // Prime AudioContext on first user gesture (autoplay policy).
    const primeOnce = () => { player.prime(); cleanup(); };
    const cleanup = () => {
        document.removeEventListener("pointerdown", primeOnce, true);
        document.removeEventListener("keydown", primeOnce, true);
    };
    document.addEventListener("pointerdown", primeOnce, { capture: true, once: true });
    document.addEventListener("keydown", primeOnce, { capture: true, once: true });

    // Master gain reactive to settings.
    createEffect(() => {
        const vol = getSettingsKeyAtom("notify:sounds:volume")() ?? 0.6;
        player.setMasterGain(vol);
    });

    subscribeSoundEvents((ev) => {
        if (!shouldPlay(ev)) return;
        const def = SOUNDS[ev.id];
        if (!def) return;
        const now = performance.now();
        const last = lastFiredAt.get(ev.id) ?? 0;
        if (now - last < (def.coalesceMs ?? 300)) return;
        lastFiredAt.set(ev.id, now);
        player.play(def, ev.override?.gain ?? 1);
    });
}

function shouldPlay(ev: SoundEvent): boolean {
    if (!getSettingsKeyAtom("notify:sounds:enabled")()) return false;
    const def = SOUNDS[ev.id];
    const perEvent = getSettingsKeyAtom(def.settingKey)();
    if (perEvent === false) return false;                            // explicit opt-out
    if (perEvent == null && !defaultsOnFor(def)) return false;       // defaults-off events (none in v1)

    // Focus suppression: if the originating pane is focused AND the window
    // has OS focus, the user can already see the visual change — no need
    // to nag them with sound.
    const suppressWhenFocused = getSettingsKeyAtom("notify:sounds:suppresswhenfocused")() ?? true;
    if (suppressWhenFocused && ev.sourceBlockId
        && focusedBlock() === ev.sourceBlockId
        && isWindowFocused()) {
        return false;
    }
    return true;
}
```

Three knobs (master enable, master volume, suppress-when-focused), one per-event override map (`notify:sound:<id>`), and a coalesce window. That is all the policy.

### 4.7 Window focus signal

```ts
// frontend/app/window/window-focus.ts
export function makeWindowFocusSignal(): () => boolean {
    const [focused, setFocused] = createSignal(document.hasFocus());
    window.addEventListener("focus", () => setFocused(true));
    window.addEventListener("blur", () => setFocused(false));
    document.addEventListener("visibilitychange", () => setFocused(document.visibilityState === "visible" && document.hasFocus()));
    return focused;
}
```

Small, self-contained, no dependency on the rest of the focus manager. Used only by the sound service in v1, but the signal itself is general — surface it via `window/window-focus.ts` so future consumers can read it without reinventing.

### 4.8 Bootstrap — `index.ts`

```ts
// frontend/app/notification/sound/index.ts
export { installSoundService } from "./sound-service";
export { notify, subscribeSoundEvents, type SoundEvent } from "./sound-events";
export { SOUNDS, type SoundId, type SoundDef, type SoundCategory } from "./sounds";
```

`installSoundService()` is called once from `frontend/app-init.ts` after the settings atom is hydrated (so the AudioContext-prime listener is installed before the first user click) and after the agent-pane store is constructed (so we can add the multicast listener).

---

## 5. Settings — schema additions

### 5.1 Backend struct (`agentmux-srv/src/backend/wconfig/types.rs`)

Add to `SettingsType` near the existing `voice_enabled` field (~line 218):

```rust
// -- Notification sounds --
//
// Master switch. Default: true (sounds on). User sets to false in
// settings.json to fully silence the app.
#[serde(rename = "notify:sounds:enabled", default, skip_serializing_if = "Option::is_none")]
pub notify_sounds_enabled: Option<bool>,

// 0.0 to 1.0; default 0.6 if unset.
#[serde(rename = "notify:sounds:volume", default, skip_serializing_if = "Option::is_none")]
pub notify_sounds_volume: Option<f32>,

// Suppress a pane's sound when that pane is focused AND the window is in
// foreground. Default: true.
#[serde(rename = "notify:sounds:suppresswhenfocused", default, skip_serializing_if = "Option::is_none")]
pub notify_sounds_suppress_when_focused: Option<bool>,

// Per-event opt-out. Absence = default (on for v1's sound set).
#[serde(rename = "notify:sound:agent.turn.complete", default, skip_serializing_if = "Option::is_none")]
pub notify_sound_agent_turn_complete: Option<bool>,

#[serde(rename = "notify:sound:agent.turn.error", default, skip_serializing_if = "Option::is_none")]
pub notify_sound_agent_turn_error: Option<bool>,

#[serde(rename = "notify:sound:agent.turn.interrupted", default, skip_serializing_if = "Option::is_none")]
pub notify_sound_agent_turn_interrupted: Option<bool>,

#[serde(rename = "notify:sound:agent.message.accepted", default, skip_serializing_if = "Option::is_none")]
pub notify_sound_agent_message_accepted: Option<bool>,

#[serde(rename = "notify:sound:agent.message.rejected", default, skip_serializing_if = "Option::is_none")]
pub notify_sound_agent_message_rejected: Option<bool>,

#[serde(rename = "notify:sound:agent.stream.stalled", default, skip_serializing_if = "Option::is_none")]
pub notify_sound_agent_stream_stalled: Option<bool>,
```

### 5.2 TypeScript types (`frontend/types/gotypes.d.ts`)

Mirror the above into the `SettingsType` block (~line 1287):

```ts
"notify:*"?: boolean;
"notify:sounds:enabled"?: boolean;
"notify:sounds:volume"?: number;
"notify:sounds:suppresswhenfocused"?: boolean;
"notify:sound:agent.turn.complete"?: boolean;
"notify:sound:agent.turn.error"?: boolean;
"notify:sound:agent.turn.interrupted"?: boolean;
"notify:sound:agent.message.accepted"?: boolean;
"notify:sound:agent.message.rejected"?: boolean;
"notify:sound:agent.stream.stalled"?: boolean;
```

### 5.3 User template (`settings-template.jsonc`)

Add a section after the existing dnd block:

```jsonc
// -- Notification sounds --
// "notify:sounds:enabled":                  true,
// "notify:sounds:volume":                   0.6,
// "notify:sounds:suppresswhenfocused":      true,
// "notify:sound:agent.turn.complete":       true,
// "notify:sound:agent.turn.error":          true,
// "notify:sound:agent.turn.interrupted":    true,
// "notify:sound:agent.message.accepted":    true,
// "notify:sound:agent.message.rejected":    true,
// "notify:sound:agent.stream.stalled":      true,
```

### 5.4 JSON schema (`schema/settings.json`)

Same shape — adds the keys with `type: "boolean"` / `type: "number"` and a `description` per key matching the comments above.

No settings-UI work in v1. Users edit `settings.json` directly via Hamburger → Settings (same as today's settings model). A settings-UI panel is fair game for v2 once the sound set stabilizes.

---

## 6. Open design questions + chosen answers

### 6.1 Why not just call `setEventSink` from the sound service?

Because the existing `setEventSink` is "last-writer-wins" and `browser-model.ts` already owns the slot to handle `pane-clicked`. The sound service would either clobber the existing handler (silent break of the click-focus path) or have to manually re-implement the multicast itself — at which point it might as well live in the store. We do the latter, additively (§4.4 Path B). No existing single-sink call site changes.

### 6.2 Why not subscribe to `dispatchRecordsAtom` instead?

`command-source.ts:65` keeps a 500-entry ring buffer of every dispatch, exposed as a SolidJS signal. A naïve `createEffect(() => dispatchRecordsAtom())` listener would fire on every dispatch *anywhere in the app* — agent-document StreamFlush, agent-pane TokensIn / TokensOut delta, etc. — at the cadence of every typed character and every parsed event. The signal-write fan-out is exactly what `recordDispatch`'s `untrack` block was added to defend against (see `command-source.ts:78` and the "3000× runaway and renderer V8-stack crash" comment). Using it as a sound-event feed would mean re-walking the ring buffer per dispatch, allocating filtered arrays, and chaining a reactive effect through the most-hot dispatch paths in the app. Not viable.

The multicast event-listener path is O(1) per fan-out (`for (const l of extraListeners)`), runs in the dispatch's existing critical section, and does **not** establish reactive dependencies.

### 6.3 Asset format and sourcing

| Format | Pros | Cons |
|---|---|---|
| **OGG Vorbis** | Smallest size, Apache-compatible decoders ship in CEF Chromium | None for our use case |
| MP3 | Universal | Slightly larger, patent history is moot under modern Chromium but not great optics |
| WAV | Zero decode cost, dead simple | 5–10× the size — bundle bloat |
| Synth only (no assets) | Zero shipped bytes, no licensing question | Tinnier, less polished |

**Recommendation:** ship OGG at 64kbps mono, target ≤200ms duration per SFX, ~3KB per file. Six sounds → ~18KB total. Tracked under `public/sounds/` (Vite copies it verbatim into the dev server and the package build). Source from CC0 packs only — short-list in Appendix B. Synth fallback (§4.5) covers the dev / unbundled case and any asset load failure.

### 6.4 Per-window vs per-instance audio

CEF gives each AgentMux process one renderer per top-level window. `AudioContext`s are per-renderer. If the user has two windows of the same instance, each window's sound service primes its own `AudioContext` and plays independently. If the same `turn-ended` event lands in both windows because both render the same workspace, both will play. v1 accepts this as cheap and rare (workspaces typically display in one window). If it bites, a follow-up gates by `tabAtom() === pane.tabid`.

### 6.5 CEF autoplay policy quirks

Chromium's autoplay policy blocks new `AudioContext`s from playing until a user gesture lands. The first-gesture prime listener in `installSoundService()` covers this. The dev server (`task dev`) opens AgentMux into a CEF window that registers a user gesture as soon as the user clicks anywhere in the chrome — typically before they could have a turn complete. Packaged builds are identical. If we somehow get a `turn-ended` before any user click (extremely unlikely — the user had to click *something* to launch the agent), we drop the sound and log it; we do not queue.

### 6.6 Session-replay safety

`SPEC_AGENT_PANE_SESSION_REPLAY_2026_05_12.md` describes a replay mode where past commands are dispatched into the reducer to rebuild state. If we're naively multicast-subscribed, replay would fire sound events for every historical `turn-ended`. Guard: the sound service's `subscribeSoundEvents` handler checks a `replayMode` flag (set by the replay infrastructure) and no-ops while it's true. Plumbing: a single `setReplayMode(bool)` export from `sound-service.ts`; replay flips it around its dispatches. Two-line patch; documented in this spec, owned by the replay infrastructure to call.

### 6.7 Where does the user-facing label "Notification sounds" live?

Nowhere in v1 — there is no settings UI in v1. The only user-facing strings are the comments in `settings-template.jsonc` and the `description` fields in `schema/settings.json`. A future settings panel reads `SOUNDS[id].label` for the per-event toggle row.

---

## 7. Concrete v1 plan — turn complete only

This is the smallest end-to-end implementation. Subsequent sound events drop in by adding entries to `SOUNDS` and switch cases to the multicast listener — no further plumbing.

### 7.1 Files added

```
frontend/app/notification/sound/sounds.ts
frontend/app/notification/sound/sound-events.ts
frontend/app/notification/sound/sound-player.ts
frontend/app/notification/sound/sound-service.ts
frontend/app/notification/sound/synth-fallback.ts
frontend/app/notification/sound/index.ts
frontend/app/notification/sound/__tests__/sound-events.test.ts
frontend/app/notification/sound/__tests__/sound-service.test.ts
frontend/app/window/window-focus.ts
public/sounds/turn-complete.ogg           # CC0 — see Appendix B
```

### 7.2 Files changed

| File | Change |
|---|---|
| `frontend/app/store/agent-pane-state/types.ts` | Add `outcome: TurnOutcome` to the `turn-ended` event variant. |
| `frontend/app/store/agent-pane-state/reducer.ts` | Include `outcome` when emitting `turn-ended` (one-line change inside the existing TurnEnd arm). |
| `frontend/app/store/agent-pane-state-store.ts` | Add `addEventListener(sink): () => void` (§4.4 Path B). Fan emitted events to both the single sink and the listener set. |
| `frontend/app/store/agent-pane-state-store.test.ts` | Test the multicast: existing single-sink behavior unchanged; an added listener also receives events; unsubscribing stops delivery; a throwing listener does not break the single sink. |
| `frontend/app-init.ts` | Call `installSoundService()` once after settings hydrate + the agent-pane store is up. |
| `agentmux-srv/src/backend/wconfig/types.rs` | Add the 9 `notify:*` fields per §5.1. |
| `frontend/types/gotypes.d.ts` | Mirror the type additions per §5.2. |
| `settings-template.jsonc` | Add the "Notification sounds" section per §5.3. |
| `schema/settings.json` | Add the same keys with type + description. |

### 7.3 Files NOT changed

- `useAgentStream.ts` — even though the first use case lives here, we drive the sound via the multicast off `turn-ended`. No call-site instrumentation.
- Any other reducer / view — the bus is the integration point.

### 7.4 Test plan

- **Unit (vitest):** `sound-events.test.ts` covers `subscribe / notify / unsubscribe` and a throwing-listener-doesn't-break-the-rest invariant.
- **Unit (vitest):** `sound-service.test.ts` mocks `getSettingsKeyAtom` and the player, then asserts:
  - master-off → no play
  - per-event-off → no play
  - coalesce window drops a second fire within 300ms
  - focus-suppression drops play when the source pane is focused **and** window has focus
  - focus-suppression does **not** drop play when the pane is focused but the window is blurred
  - volume setting threads through to `player.setMasterGain`
- **Integration (vitest, jsdom):** `agent-pane-state-store.test.ts` extension — dispatch a `TurnEnd` from a registered slot, assert the multicast listener received an event with `type: "turn-ended"` and `outcome: "completed"`.
- **E2E (Playwright, deferred):** turn-complete sound deferred to manual smoke since headless audio playback validation is wasteful effort. Manual smoke checklist:
  1. With master enabled, run a Claude turn, confirm sound plays at session_end.
  2. Focus a different pane while the agent runs → confirm sound plays.
  3. Focus the agent pane itself → confirm sound is suppressed.
  4. Open a second window, give it OS focus while the agent pane is focused → confirm sound plays again (window not focused).
  5. Set `notify:sounds:enabled = false` in settings.json, save, repeat 1 → confirm silence.
  6. Set `notify:sound:agent.turn.complete = false`, repeat → confirm silence (per-event off).
  7. Delete the asset file before app start → confirm synth fallback plays a polite tone.

### 7.5 Migration / rollout

- No DB / state migration.
- New settings keys are all `Option<…>` with `None` defaults — existing `settings.json` files are unaffected.
- Behavior on a brand-new install: sounds are **on**. (Decision point — see §8.)

---

## 8. Decisions to confirm before implementing

1. **Default for `notify:sounds:enabled`** — on (proposed) or off-until-opted-in?
2. **Asset pack vs synth-only for v1** — ship a CC0 OGG pack (~18KB) or ship synth-only (zero bytes) and add the pack in v2?
3. **Per-window dedupe** — accept v1's "both windows play" or gate to active window from day one?
4. **Where does the `outcome` enrichment of `turn-ended` ride** — as part of this PR or as a separate prep PR? (Recommend: same PR; the change is two lines and removes the only awkward coupling in this design.)
5. **Multicast on other slices** — `agent-document-store`, `browser-pane-state-store`, `editor-pane-state-store`, `drone-run-state-store` all have the same single-sink pattern. v1 only modifies `agent-pane-state-store`. Do we extend the others now (so future sounds drop in for free) or later (when the first one is wanted)? Recommend later — YAGNI on store API changes.

---

## 9. Risks & mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `AudioContext` never primed (autoplay block) | Low | Sound silently no-ops | Log a one-shot warning to console; user clicking anywhere primes it. |
| Asset bundle bloat | Low | +18KB to package | OGG + mono + 64kbps + ≤200ms = bounded; checked in PR. |
| Replay-mode regression (sound plays during session replay) | Medium until §6.6 lands | Annoying noise | §6.6 `setReplayMode(true)` gate. Test added. |
| Multicast listener throws and crashes the dispatch | Low | Other listeners + state mutation unaffected (already happens before the fan-out) | Try/catch per listener in `dispatch()` and in `sound-events`. |
| Multiple panes / windows all firing on one `turn-ended` | Medium | Several rapid dings | Coalesce window per sound ID (300ms default). Per-window dedupe — open question §8.3. |
| Volume too loud on first launch | Medium | Bad first impression | Default `volume = 0.6`. Synth fallback is also gentle (sine + short envelope). |
| CC0 asset turns out to be mis-licensed | Low | Legal | Source from a vetted CC0-only pack (Appendix B); record provenance in a `LICENSES.md` alongside the asset. |
| User on muted speakers misses the only feedback | Always | Equivalent to today | Visual signal is already there (spinner stops, "Worked" appears). Optionally surface a passive notification toast as a belt-and-suspenders WCAG path in v2. |

---

## 10. Appendix A — synth fallback sketch

```ts
// frontend/app/notification/sound/synth-fallback.ts
export function playSynthFallback(
    ctx: AudioContext,
    out: GainNode,
    category: SoundCategory,
    gain: number,
): void {
    const now = ctx.currentTime;
    const osc = ctx.createOscillator();
    const env = ctx.createGain();
    const params = paramsForCategory(category);
    osc.type = params.wave;
    osc.frequency.setValueAtTime(params.freq, now);
    // Short attack, exponential decay — ~150ms total, polite envelope.
    env.gain.setValueAtTime(0.0001, now);
    env.gain.exponentialRampToValueAtTime(gain * params.peak, now + 0.01);
    env.gain.exponentialRampToValueAtTime(0.0001, now + 0.15);
    osc.connect(env).connect(out);
    osc.start(now);
    osc.stop(now + 0.16);
}

function paramsForCategory(c: SoundCategory) {
    switch (c) {
        case "success": return { wave: "sine" as const,     freq: 880, peak: 0.5 };
        case "info":    return { wave: "sine" as const,     freq: 660, peak: 0.4 };
        case "warning": return { wave: "triangle" as const, freq: 440, peak: 0.5 };
        case "error":   return { wave: "square" as const,   freq: 220, peak: 0.4 };
    }
}
```

Deterministic, allocation-light, requires no assets, sounds polite.

---

## 11. Appendix B — CC0 asset shortlist

- **Kenney UI Audio** (kenney.nl/assets/interface-sounds) — CC0, multiple short "ding" / "confirm" / "error" variants suitable for the v1 sound set.
- **Material Sound Effects** — Google's released set under Apache 2.0; high-quality, identifiable, well-balanced.
- **Freesound CC0 packs** — vetted per-asset CC0 dedications only (avoid Attribution licenses to keep the bundle Apache-clean).

The PR will include `public/sounds/LICENSES.md` listing provenance + license for every shipped clip.

---

## 12. Out of scope, tracked elsewhere

- Backend-originated sound events (sagas, runtime alerts) — needs a backend → frontend audio-event channel; design after sagas-execution-plan stabilizes.
- OS-level notifications (Windows toast, macOS notification center, libnotify) — separate spec.
- Settings UI panel for the sound controls — separate UX-driven design once the sound set stabilizes.
- Customizable sound themes / user-supplied SFX — interesting follow-on; gate behind a "Sound theme" setting that points at a user-data-dir asset folder.
- Per-agent / per-provider sound differentiation — drop-in via a `notify("agent.turn.complete", { override: { asset: …, gain: … } })` call from the originating site; design surface already exists.
