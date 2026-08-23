# SPEC — Agent tool-call tones (subliminal "talking" voice)

**Status:** implemented — core feature shipped in commit `4bda9ce88` ("per-tool-call subliminal tone voice"), with several follow-up settings/UI fixes since (#2668, #2669, #2689). Verified 2026-08-23: `frontend/app/notification/sound/tool-tones.ts` implements the exact G-major-pentatonic design this spec specifies; the `notify:tooltones:scope` setting (`all`/`focused`) matches §3.3's design.
**Date:** 2026-06-05
**Author:** agent2
**Builds on:** `docs/specs/SPEC_SOUND_NOTIFICATIONS_2026_06_05.md` (v1 sound subsystem — bus, player, settings plumbing)

**Related:**
- `frontend/app/store/agent-pane-state/types.ts` (line 535 — existing `tool-started` event we hook off)
- `frontend/app/store/agent-pane-state/reducer.ts` (line 451 — `ToolStart` arm that emits it)
- `frontend/app/view/agent/stream-parser.ts` (line 476 — `normalizeToolName` — the canonical name set the parser surfaces to us today)
- `frontend/app/notification/sound/` (the v1 subsystem we extend)
- `frontend/app/store/focusManager.ts` (per-pane focus signal — central to the §3.3 scoping decision)
- MDN — `BiquadFilterNode` lowpass + `OscillatorNode` envelope shape

---

## 0. TL;DR

Each agent tool call (`Read`, `Write`, `Bash`, `Edit`, …) plays a tiny synthesized tone — different per tool, deterministic, ~50ms, lowpass-filtered, very quiet. The effect is an ambient "voice" texture: when the agent strings together Read → Read → Edit → Write → Bash, you hear a recognizable little phrase. Over time the user learns the phrases without consciously trying — same path as keyboard sounds, RAID-controller ticks, or a Geiger counter.

**Talking, not morse.** The mapping is real — same tool always sounds the same — so it carries information, but the encoding is musical (pentatonic intervals, sine timbre) not literal (dot/dash). Pentatonic intervals can never produce a sour combination, which matters because tool calls can fire at >10 Hz during dense agent activity.

**Subliminal, not background music.** Default volume is ~0.15 of master, the chain is lowpassed at ~2.5 kHz, and individual tones are 40–80 ms. Loud enough to inform "the agent is moving"; quiet enough not to compete with anything else the user is doing.

**All panes by default — scope is a setting.** Tool tones are an ambient companion to *attending to the app at all*, not just to one pane. Default is `scope: "all"` so a user with two agents working in parallel hears both. Users who find the parallel chatter too dense can drop to `scope: "focused"` (only the focused pane in the focused window) — that's the single setting that turns "useful" into "useable" for a multi-agent power user. The `"focused"` mode is also the inverse of the v1 notification-sound focus rule (which *suppresses* when focused, because the user is already looking) — these are different UX surfaces and they get different defaults intentionally.

---

## 1. Why

The agent activity log is the visual channel for "what is the agent doing?" — every Read, every Bash, every Edit shows up as a row with the tool name and a one-line summary. When the user is watching the pane, this works. When they look away for a second to type elsewhere, they lose the thread.

A tiny per-call sound closes that gap as long as the user is looking AT the window. They don't have to keep their eyes on the activity log to feel the *cadence* of the work — Read-Read-Read-Bash means "looking around then ran something," Read-Edit-Read-Edit means "iterating on a file," and Bash-Bash-Bash means "this got stuck and is hammering the same thing." The user picks up patterns the way you pick up the rhythm of a colleague typing next to you.

This is **distinct from** the v1 sound notifications (turn-complete, error, etc.):

| | v1 notifications | tool-call tones |
|---|---|---|
| What it tells you | a discrete event happened | something is in progress |
| Attention model | summons your attention | adds texture under your attention |
| Volume | normal (0.6 default) | subliminal (~0.15 default) |
| Focus suppression | suppress when looking at it | only play when looking at it |
| Default | on | on (with scope and volume knobs) |
| Frequency | seconds-to-minutes apart | up to 10–20 Hz during dense activity |

We do not collapse the two — they are different UX surfaces with different defaults, different volumes, and different scope rules. They share the AudioContext and the bus, nothing else.

---

## 2. Non-goals

- **No literal morse code, no phoneme synthesis, no TTS.** Pentatonic intervals carry plenty of information and never dissonate.
- **No "voice themes" (major / minor / world scales).** Future surface, listed in §10 — v1 ships one curated palette.
- **No backend-side hint stream.** The reducer already fires `tool-started` with the tool name; we read it via the multicast. No new RPC, no new event.
- **No tool-end sound.** Doubles the rate, doesn't add information. (Tool *outcome* — error vs success — is interesting but lives in the activity log via the chunk-stream; revisit in v2 if user feedback wants it.)
- **No per-chunk streaming sound.** Way too dense — a Bash that streams 200 lines of stdout would be a buzz, not a tone.
- **No spatial / per-pane panning** in v1 — every pane mixes to the same mono channel. Stereo per-pane positioning is a v2 follow-up (§8.5).
- **No spatial / stereo / panning effects** in v1 — single mono channel. Stereo per-agent panning is fun (left agent vs right agent) but adds a knob that needs design.
- **No external asset shipping.** Synth only, per user direction. Same posture as v1.

---

## 3. Design — three decisions and the rest is mechanical

### 3.1 The mapping: pentatonic, two-tone "syllable" per call

Each tool call plays **two short tones in quick succession** (a "syllable") drawn from a pentatonic scale. Two tones because:
- One tone is too thin to feel like speech.
- Two tones make an interval — interval is the unit of *meaning* in this system. C→E is the Read syllable; E→C is the Write syllable; the user learns by inversion as much as by absolute pitch.
- Two short tones at 40–80 ms each fit inside a single tool-call without colliding with the next, even at peak agent cadence.

**Scale: G pentatonic** (G, A, B, D, E across two octaves). Pentatonic was chosen because:
- No semitone clashes — any two notes sound musically compatible, even when overlapping (which they will, when tools fire faster than the coalesce window).
- 8 distinct degrees in a 2-octave window — enough headroom to give 6–8 curated tools unique voices, plus a deterministic hash space for unknowns.
- G base lands the syllables in the middle of human pitch perception (~390 Hz) — clear without being shrill at low volume.

### 3.2 The mapping: curated for canonical tools, hashed for unknowns

The canonical set (from `stream-parser.ts:478`): `Read, Edit, Write, Bash, Grep, Glob, Task, Agent`. Each gets a hand-tuned syllable that *means* what the tool does. Unknown tools (MCP-provided, agent custom, etc.) get a deterministic hash → syllable so they're consistent within a session.

#### 3.2.1 Curated palette

| Tool | Syllable | Why |
|---|---|---|
| **Read** | B4 → A4 (gentle falling 2nd) | Soft, descending — "looking." Neutral, doesn't claim attention. |
| **Grep** | E5 → E5 (two same-pitch quick ticks) | "Search" — a question-mark feel, twin ticks for "scanning." |
| **Glob** | D5 → E5 (paired tick, rising) | "Listing" — Grep's sibling but rising (results coming in). |
| **Edit** | A4 → B4 → A4 (three-tone "fix") | Up-down-up — the auditory shape of a small correction. Three-tone, paired with Agent to mark high-importance gestures. |
| **Write** | G4 → D5 (rising 5th) | Bigger interval = "creation." Confident, not loud. |
| **Bash** | G3 → D4 (rising 5th, octave down, triangle wave) | Same musical shape as Write but darker — mechanical. Triangle gives the "machine" timbre. |
| **Task** | D5 → G5 (rising 4th, high) | "Delegation upward" — clear and brief. |
| **Agent** | G4 → D5 → G4 (returning 5th) | "Sub-agent dispatch" — out and back, three tones to mark importance. |
| `Other`/unknown | hashed (see §3.2.2) | Stable across the session. |

A user reading "Read Read Grep Read Edit Write Bash" hears: *falling, falling, twin-ticks, falling, three-tone-fix, rising-5th, mechanical-rising-5th*. The shape of work is audible.

#### 3.2.2 Hash fallback for unknowns

```ts
function hashToolToParams(tool: string): SyllableParams {
    // FNV-1a-ish hash for determinism + cheap to compute.
    let h = 2166136261 >>> 0;
    for (let i = 0; i < tool.length; i++) {
        h = (h ^ tool.charCodeAt(i)) >>> 0;
        h = Math.imul(h, 16777619) >>> 0;
    }
    // Pentatonic degree set (G major pentatonic, 2 octaves above G3).
    // 8 entries — each tool gets two distinct degrees.
    const DEGREES = [0, 2, 4, 7, 9, 12, 14, 16];
    const noteAt = (i: number) => 196 * Math.pow(2, DEGREES[i % 8] / 12); // G3 base
    return {
        tones: [noteAt(h), noteAt(h >>> 3)],
        durationMs: 45 + ((h >>> 6) & 0x1f), // 45–76ms
        wave: ((h >>> 11) & 1) === 0 ? "sine" : "triangle",
        gapMs: 12 + ((h >>> 12) & 7), // 12–19ms gap between tones
    };
}
```

Properties: deterministic, allocation-light, no string normalization beyond what the parser already does, no collision behavior to memorize.

### 3.3 The scoping rule: `notify:tooltones:scope`

Three modes, settings-driven, default `"all"`:

- **`"all"` (default)** — play for every pane in every window. The user hears all active agents. This is the "I want my desktop to be a chorus of agents I can listen to" mode. Default because it's the mode that delivers the spec's core promise (a 3-agent run sounds like 3 distinct voices) without requiring any setting flip.
- **`"window"`** — play for any pane in the *focused* window. Multi-window users who keep one foreground window per task get one window's chorus at a time. v1.5 follow-up (§8.5); not in the initial PR until we've seen actual user need.
- **`"focused"`** — play only when the source pane is focused AND the window has OS focus. A single agent's voice; everything else silent. This is the "I'm too overwhelmed by the chorus, let me hear one at a time" knob — and it's the *opposite* of v1's notification focus-suppression rule.

Default-on + `scope: "all"` does mean a user running 6 agents at once will hear all 6. That's the explicit design — it's what surfaces "the system is alive" most clearly. The pentatonic palette is bounded so 6 overlapping tools never produce a sour combination (§3.1 + §3.2). Users who find it dense have one setting flip (`scope: "focused"`) to recover quiet.

---

## 4. Module layout

```
frontend/app/notification/sound/
    tool-tones.ts            # NEW — tool name → syllable params, curated + hashed
    tool-tones-player.ts     # NEW — dedicated synth path with lowpass + indep gain
    sound-service.ts         # MODIFIED — route `tool-started` events to the player
    sounds.ts                # unchanged (tool tones do not use the registry — they're parametric)
    sound-player.ts          # unchanged
    synth-fallback.ts        # unchanged
    sound-events.ts          # unchanged
    index.ts                 # MODIFIED — re-export setToolTonesEnabled for tests
```

Adding a new curated tool is one entry in `tool-tones.ts`'s palette. Removing one falls back to hash. No settings change required to extend the palette.

---

## 5. The player chain — why a separate one

The v1 `SoundPlayer` does buffer-or-synth-fallback on the shared master bus. The tool-tones path has two specific needs the master bus doesn't provide:

1. **Lowpass filter.** A `BiquadFilterNode` at ~2.5 kHz cutoff smooths the attack of every tone, making the tones blend into ambient instead of poking through it. Notification sounds should NOT be lowpassed (we want them crisp); tool tones should.
2. **Independent volume.** The user wants notifications at one level and tool tones at another (much quieter, often). Two independent gain knobs.

Chain:

```
OscillatorNode → per-tone envelope (GainNode)
              → tool-tones gain (settings-bound)
              → BiquadFilter (lowpass ~2.5 kHz)
              → master gain (shared with v1)
              → AudioContext.destination
```

The master gain stays shared — turning the master to zero silences everything, which is the right semantics. The tool-tones gain layers below it.

```ts
// tool-tones-player.ts (sketch)
export class ToolTonesPlayer {
    private filter: BiquadFilterNode | null = null;
    private gain: GainNode | null = null;
    private toolGainValue = 0.15;
    private lastFiredAt = new Map<string, number>();

    attach(ctx: AudioContext, master: GainNode): void {
        this.filter = ctx.createBiquadFilter();
        this.filter.type = "lowpass";
        this.filter.frequency.value = 2500;
        this.filter.Q.value = 0.707;
        this.gain = ctx.createGain();
        this.gain.gain.value = this.toolGainValue;
        this.gain.connect(this.filter).connect(master);
    }

    setVolume(v: number): void {
        const clamped = Math.max(0, Math.min(1, v));
        this.toolGainValue = clamped;
        if (this.gain) this.gain.gain.value = clamped;
    }

    play(ctx: AudioContext, tool: string): void {
        const out = this.gain;
        if (!out) return;
        // Coalesce: same tool fired within 30ms = drop. The number
        // is tight on purpose; the user wants to hear "Read Read Read."
        const now = performance.now();
        const last = this.lastFiredAt.get(tool) ?? 0;
        if (now - last < 30) return;
        this.lastFiredAt.set(tool, now);
        const p = paramsForTool(tool);
        playSyllable(ctx, out, p);
    }
}

function playSyllable(ctx: AudioContext, out: AudioNode, p: SyllableParams): void {
    const startAt = ctx.currentTime;
    p.tones.forEach((freq, i) => {
        const osc = ctx.createOscillator();
        const env = ctx.createGain();
        const at = startAt + (i * (p.durationMs + p.gapMs)) / 1000;
        osc.type = p.wave;
        osc.frequency.setValueAtTime(freq, at);
        env.gain.setValueAtTime(0.0001, at);
        env.gain.exponentialRampToValueAtTime(0.4, at + 0.006);
        env.gain.exponentialRampToValueAtTime(0.0001, at + p.durationMs / 1000);
        osc.connect(env).connect(out);
        osc.start(at);
        osc.stop(at + p.durationMs / 1000 + 0.02);
    });
}
```

`paramsForTool` consults the curated map first; falls back to `hashToolToParams`.

---

## 6. Service wiring — additive to v1

`sound-service.ts` already installs a single multicast listener on the agent-pane-state store. The same listener gets one more case:

```ts
case "tool-started": {
    if (!shouldPlayToolTone(blockId)) return;
    const ctx = player.getAudioContext();
    if (!ctx) return; // not primed yet
    toolTones.play(ctx, event.name);
    return;
}
```

`shouldPlayToolTone(blockId)` reads `notify:tooltones:enabled` (default false) + `notify:tooltones:scope` and applies the focus rule:

```ts
function shouldPlayToolTone(blockId: string): boolean {
    if (replayMode) return false;
    // Master notification kill-switch covers tool tones too — they share
    // the master gain, but checking the setting up-front lets us skip
    // the AudioContext work entirely.
    if (getSettingsKeyAtom("notify:sounds:enabled")() === false) return false;
    const enabled = getSettingsKeyAtom("notify:tooltones:enabled")();
    if (enabled === false) return false; // default true — absence is on
    const scope = getSettingsKeyAtom("notify:tooltones:scope")() ?? "all";
    if (scope === "focused") {
        return focusManager.blockFocusAtom() === blockId && windowFocused();
    }
    // "window" mode lands in v1.5 — same predicate as "all" for now.
    return true; // "all" (or unknown — fail open to the default)
}
```

The volume thread:

```ts
createEffect(() => {
    const vol = getSettingsKeyAtom("notify:tooltones:volume")();
    toolTones.setVolume(typeof vol === "number" ? vol : 0.15);
});
```

`SoundPlayer` needs one new accessor — `getAudioContext(): AudioContext | null` — so the tool-tones player can hook into the same context. Tiny, additive.

The tool-tones player is attached on first prime (chain wired up at the moment the AudioContext is created):

```ts
// Inside installSoundService(), after prime():
toolTones.attach(player.getAudioContext()!, player.getMasterGain()!);
```

`SoundPlayer.getMasterGain(): GainNode | null` — also a tiny additive accessor.

---

## 7. Settings additions

Mirror of the v1 plumbing — Rust struct, TS type, user template, schema:

```jsonc
// -- Notification: tool-call tones (subliminal "voice") --
// "notify:tooltones:enabled":   true,
// "notify:tooltones:volume":    0.15,
// "notify:tooltones:scope":     "all",     // "all" or "focused"
```

Schema entries: `notify:tooltones:enabled` (bool, default true), `notify:tooltones:volume` (number 0–1, default 0.15), `notify:tooltones:scope` (enum `"all" | "focused"`, default `"all"`).

Rust struct: three new `Option<...>` fields with the same `#[serde(rename = ...)]` pattern as v1.

TypeScript `SettingsType`: three new optional keys.

**Default is `enabled: true`**. The point of the feature is the cadence under everything else — having users discover it under a settings flag means most never know it's there. Risks of on-by-default (hearing fatigue, surprise on update) are mitigated by:
- Low default volume (0.15 × master 0.6 ≈ 0.09 of full).
- Lowpass at 2.5 kHz softens every onset.
- v1 master `notify:sounds:enabled = false` silences tool tones too (shared chain).
- Changelog entry calls out the new sound + how to disable it (`notify:tooltones:enabled: false`) or quiet it (`notify:tooltones:volume: 0.05`).

---

## 8. v1 plan — files

### 8.1 Files added

```
frontend/app/notification/sound/tool-tones.ts
frontend/app/notification/sound/tool-tones-player.ts
frontend/app/notification/sound/__tests__/tool-tones.test.ts
```

### 8.2 Files changed

| File | Change |
|---|---|
| `frontend/app/notification/sound/sound-player.ts` | Add `getAudioContext()` and `getMasterGain()` accessors. |
| `frontend/app/notification/sound/sound-service.ts` | Wire `tool-started` event into `ToolTonesPlayer`; attach the player on prime; reactively thread the tool-tones volume + scope settings. |
| `frontend/app/notification/sound/index.ts` | Re-export `__getToolTonesPlayer` for tests. |
| `agentmux-srv/src/backend/wconfig/types.rs` | Three new `notify:tooltones:*` fields. |
| `frontend/types/gotypes.d.ts` | Three new TS keys. |
| `settings-template.jsonc` | Three new commented lines. |
| `schema/settings.json` | Three new schema entries. |

### 8.3 Files NOT changed

- `sounds.ts` — the registry is for fixed-id sounds; tool tones are parametric.
- `agent-pane-state-store.ts` — the multicast is already there.
- `useAgentStream.ts` — `tool-started` already emits from the reducer.
- `agent-pane-state/types.ts` — `tool-started.name` already carries the raw provider tool name (no `rawName` needed; see §9.6).
- `agent-pane-state/reducer.ts` — no event-shape change.

### 8.4 Test plan

- **Unit (vitest):** `tool-tones.test.ts` covers:
  - the curated palette resolves to the documented syllables (asserted on tone count, frequencies, wave shape, duration range)
  - the hash fallback is deterministic for a given string
  - the hash fallback never produces a frequency outside the pentatonic set
  - `Other`-normalized tools take the hash path
- **Unit (vitest):** `sound-service.test.ts` extension:
  - `tool-started` plays the tone by default (no settings set) — the on-by-default contract
  - `tool-started` does nothing when `notify:tooltones:enabled` is false
  - `tool-started` does nothing when the v1 master `notify:sounds:enabled` is false (shared kill-switch)
  - `tool-started` plays when scope=all regardless of focus (default behavior — both panes in a two-pane test)
  - `tool-started` plays when scope=focused and the pane is focused (+ window focused)
  - `tool-started` does NOT play when scope=focused and the pane is unfocused
  - 30ms coalesce per tool — second call within the window drops
  - Volume setting threads through to the player
- **Manual smoke (Playwright unhelpful for audio):**
  1. Enable, run a Claude session with multiple tools — confirm distinct sounds.
  2. Focus a different pane in the same window — confirm silence.
  3. Open a second window, give it focus — confirm silence from the first window.
  4. Set scope=all, run two agents in parallel — confirm both play.
  5. Set volume=0 — confirm silence (without disabling).
  6. Set master notifications off — confirm tool tones also silenced (shared master).

### 8.5 Out of scope, but easy v1.5 follow-ups

- `scope: "window"` option (every pane in the focused window, ignore per-pane focus). Wire is in place — `shouldPlayToolTone` accepts the literal already, just treats it the same as `"all"`. Promotion is a 3-line change once the predicate is decided (tab-level focus signal exists in the layout model).
- "Voice profile" picker (major / minor / pentatonic / wholetone) as a single string setting `notify:tooltones:scale`.
- Stereo panning by pane position (left pane → left ear). Adds spatial information at zero cognitive cost.
- Tool-outcome ornaments: a tiny "click up" on success vs "click down" on error appended to the syllable. Needs the tool-result event to carry success/failure — exists, would couple cleanly.
- First-launch one-shot toast on update — "we added tool-call sounds; here's where to disable them" — if telemetry shows a wave of users disabling immediately after the release.

---

## 9. Resolved decisions

1. **Default state of `notify:tooltones:enabled`** — **on**. The cadence is the feature; users who don't know it's there don't get it. Mitigation lives in the volume default + master kill-switch + changelog (§7).
2. **Default scope** — **`"all"`** (every pane in every window). Users who want a quieter mode flip to `"focused"`. The middle `"window"` option is wired but treated as `"all"` for now; promoted in v1.5 (§8.5).
3. **Default volume** — 0.15 of the master. Re-tune in v1.5 if users report it's loud at default OS volume.
4. **Coalesce window** — 30ms per tool. Tight on purpose so Read-Read-Read sounds like three discrete syllables. If reducer storms drop legitimate signals, raise to 60ms in a quick follow-up.
5. **Tool-ended sound** — **no**. Doubles the rate, doesn't add useful information. The next `tool-started` is the implicit "previous one finished" signal.
6. **Raw vs normalized tool name** — *no event change needed.* Code-reading confirmed `tool-started.name` already carries the raw provider tool name (`useAgentStream.ts:554` passes `event.tool` through verbatim). The `Other` normalization happens further downstream in the document parser for the activity-log render, not at the reducer boundary. The tool-tones module hashes / matches on the raw `name` directly — MCP tools like `mcp__myserver__my_tool` each hash to their own deterministic syllable.

---

## 10. Risks & mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Hearing fatigue during long sessions | Medium-High (on by default) | User disables, never re-enables | Low default volume (0.15 × master 0.6 ≈ 0.09 of full) + lowpass at 2.5 kHz. Master kill-switch documented. Changelog calls out volume + disable knobs. v2 follow-up: auto-fade after N minutes of continuous tool activity, or per-session diminishing returns. |
| Tool tones mask v1 notifications | Low | Important "turn-complete" missed | They share master; tool-tones gain is layered BELOW master at 0.15 — notifications stay relatively louder. |
| Polyphony explosion (many tools at once) | Low | Buzzing chord | Per-tool 30ms coalesce + the lowpass; pentatonic intervals don't dissonate even when overlapping. |
| Hashed unknowns sound bad together | Low | Cacophony from MCP-heavy users | The hash output lives in the same pentatonic set as curated tools — bounded by construction. |
| First-time users find it intrusive on update | Medium | Bad first impression | Low default volume; master kill-switch already familiar from v1; changelog highlights the new sound + the off knob. Telemetry — if disable rate spikes after release, ship a v1.1 with default-off + first-launch toast. |
| Headphone users find it loud at default level | Medium | Discomfort | Default 0.15 is low; documented in the changelog with a "set to 0.05 if on headphones" hint. |
| Replay mode plays the texture for every historical tool-start | Medium | Loud surprise during a replay scrub | The existing v1 `replayMode` gate covers `tool-started` too — same single check in `shouldPlayToolTone`. |
| BiquadFilter at low cutoff on some old CEFs produces artifacts | Low | Audio glitches | Cutoff 2.5 kHz is well within stable territory; Q 0.707 (Butterworth — no resonance peak). |

---

## 11. Why not just one knob per tool category?

A simpler design would be: "read-class tools = tone A, write-class = tone B, exec-class = tone C." Three sounds total. It's much easier to learn.

We reject it because it throws away the thing that makes the system useful: the *pattern* of calls. With three sounds, Read-Edit-Read-Edit-Write sounds nearly the same as Grep-Bash-Glob-Bash-Write — both reduce to "read-write-read-write-write". Distinct per-tool syllables make the *sequence* informative. The user learns the alphabet over an afternoon — which is *exactly* the morse-code-style learning curve they asked for — and from then on every dense agent run sounds like words.

Categorical mapping is a useful fallback if the curated palette feels too crowded. Easy v2 toggle: `notify:tooltones:granularity = "tool" | "category"`.

---

## 12. Appendix A — full curated table for reference

| Tool | Tones (Hz) | Wave | Duration / tone | Gap | Notes |
|---|---|---|---|---|---|
| Read | B4 (494) → A4 (440) | sine | 60ms | 14ms | Gentle descending major 2nd. |
| Grep | E5 (659) → E5 (659) | sine | 45ms | 18ms | Twin ticks, same pitch — "scanning." |
| Glob | D5 (587) → E5 (659) | sine | 45ms | 18ms | Rising minor 2nd — "results coming." |
| Edit | A4 (440) → B4 (494) → A4 (440) | sine | 50ms | 12ms | Three-tone up-down — the "fix" gesture. |
| Write | G4 (392) → D5 (587) | sine | 60ms | 18ms | Rising perfect 5th — "creation." |
| Bash | G3 (196) → D4 (294) | triangle | 70ms | 18ms | Same 5th, octave down + triangle = "machine." |
| Task | D5 (587) → G5 (784) | sine | 55ms | 14ms | Rising 4th, high — "delegation upward." |
| Agent | G4 (392) → D5 (587) → G4 (392) | sine | 55ms | 14ms | Out-and-back 5th — "sub-agent dispatch." |

(`Other`/unknown: hashed per §3.2.2 — deterministic, pentatonic, ≤ 2 tones, sine or triangle.)
