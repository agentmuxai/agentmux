# Activity flash matches its sound: same strikes, same timing, same relative intensity

**Status:** active — Phase 1 (tool tones) shipped in PR #3717; Phase 2 (event
sounds) in PR #3740; Phase 3 (waiting loop) not started, pending Q3.
See §8–§9 for what shipped and where it departs from §3.
**Date:** 2026-09-24.
**Requested by:** repo owner (asafebgi).
**Author:** Clamk.
**Builds on:** `docs/specs/SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md` (PR #3625, Revisions 2–3).
It replaces that spec's single fixed envelope and its 100 ms throttle. Everything
else stays: targets (window tab and pane pill), colors, overlay mechanics, the
setting, and the gates.
**Related:** `SPEC_AGENT_TOOL_CALL_TONES_2026_06_05.md` (tool syllables),
`SPEC_SOUND_NOTIFICATIONS_2026_06_05.md` (event sounds),
`SPEC_AGENT_WAITING_AMBIENT_SOUND_2026_06_19.md` (waiting loop).

---

## 1. The ask

Owner, 2026-09-24: the flash should match the sound in **intensity, frequency and
timing**. They hear roughly three sounds: one is a **single hard knock**, another is
**three smaller knocks**. Both must be recognizable in the flash.

Today every tool tone produces the same flash: an instant jump to full strength, a
40 ms hold, then a 300 ms `ease-out` fade (`frontend/app/notification/activity-flash.ts:69-81`).
Every syllable looks identical, whether it has two strikes or three.

## 2. Exact timing of every sound

Every sound in the app is synthesized in code with Web Audio. There are no audio
files (`sounds.ts:15-20`; no `.wav`/`.ogg`/`.mp3` in the repo). The timing below
is therefore exact: it is read straight from the synth parameters, not measured
from a recording. The only extra latency is the audio output device's (§5.3).

Envelope shape for every strike: an exponential ramp from 0.0001 up to the peak,
then an exponential ramp back down to 0.0001. "−6 dB" and "−20 dB" are times from
the strike's onset until it drops to half and a tenth of its peak amplitude.
Strike level is computed from each strike's energy (peak × RMS of the waveform,
× decay length) at default volumes, relative to the loudest sound.

### 2.1 Tool-call tones (flash today)

Source: `tool-tones.ts` (params), `tool-tones-player.ts:98-119` (synthesis).
Onset of tone *i* = *i* × (durationMs + gapMs). Attack is 6 ms. Envelope peak is
0.4, then × tool-tones volume 0.25, × master 0.6, giving **0.060 at the output
(−24.4 dBFS)**. There is a 2.5 kHz lowpass.

| Tool | Wave | Notes (Hz) | Strikes | Strike onsets (ms) | Ends (ms) | −6 dB | −20 dB |
|---|---|---|---|---|---|---|---|
| Read | sine | 494 → 440 | 2 | 0, 74 | 134 | 10.5 | 21.0 |
| Grep | sine | 659 → 659 | 2 | 0, 63 | 108 | 9.3 | 16.8 |
| Glob | sine | 587 → 659 | 2 | 0, 63 | 108 | 9.3 | 16.8 |
| **Edit** | sine | 440 → 494 → 440 | **3** | **0, 62, 124** | 174 | 9.7 | 18.2 |
| Write | sine | 392 → 587 | 2 | 0, 78 | 138 | 10.5 | 21.0 |
| Bash | triangle | 196 → 294 | 2 | 0, 88 | 158 | 11.3 | 23.8 |
| Task | sine | 587 → 784 | 2 | 0, 69 | 124 | 10.1 | 19.6 |
| **Agent** | sine | 392 → 587 → 392 | **3** | **0, 69, 138** | 193 | 10.1 | 19.6 |
| *any other tool* (hashed) | sine/tri | 2 pentatonic notes | 2 | 0, 57–95 | 102–171 | 9–12 | 17–26 |

Common hashed names, computed from the real hash: TodoWrite 0/66 (tri), WebFetch
0/61, WebSearch 0/82 (tri), ToolSearch 0/71 (tri), Skill 0/64 (tri),
`mcp__agentmux__SendMessage` 0/79, `mcp__agentmux__Shell` 0/74 (tri), Codex `shell`
0/88, `exec_command` 0/80, `apply_patch` 0/67, `update_plan` 0/75 (tri).

**Every tool tone has 2 or 3 strikes. None is a single strike.** Only Edit and Agent
have 3. Curated tool strikes land within 1.4 dB of each other in level (−18.1 to
−19.5 dB), so they sound equally loud; what tells them apart is the rhythm and the
pitch. Hashed syllables span a little wider, from −17.0 dB (76 ms sine) to −21.3 dB
(45 ms triangle).

### 2.2 Event sounds (no flash before Phase 2)

Source: `synth-fallback.ts`. Attack is 10 ms, and each strike decays to 0.0001 at
150 ms. The chain is × master 0.6 only; there is **no** 0.25 tool-tones stage, so
these are about 4–5× the amplitude of a tool tone. No lowpass.

| Sound (event) | Wave | Notes (Hz) | Strikes | Onsets (ms) | Peak at output | −6 dB | −20 dB | Strike level vs. loudest |
|---|---|---|---|---|---|---|---|---|
| **info**: pending message accepted | sine | 660 | **1** | 0 | 0.24 (−12.4 dBFS) | 21.7 | 48.9 | −1.9 dB |
| **warning**: turn interrupted, message rejected | triangle | 440 | **1** | 0 | 0.30 (−10.5 dBFS) | 21.4 | 47.8 | −1.9 dB |
| success: turn complete | sine | 784 → 1046 | 2 | 0, 70 | 0.30 | 21.4 | 47.8 | −0.1 dB |
| error: turn errored, submit timed out | square | 440 → 294 | 2 | 0, 90 | 0.21 | 21.9 | 50.1 | 0.0 dB |

A tool strike's level is **−18 to −19.5 dB** on this same scale. **Each event-sound
strike is about 17–19 dB louder than a tool strike** and decays about 3× more
slowly.

### 2.3 Waiting loop (no flash today)

Source: `waiting-tone-player.ts`. The C5 → E5 → G5 sine arpeggio has onsets at
**0, 500, 1000 ms** and **repeats every 2500 ms** while the agent waits (5 min
cap). Each note: 20 ms attack, 300 ms long, peak 0.35 × 0.25 × 0.6 = 0.052, 1.2 kHz
lowpass, 400 ms fade-in on start. −6 dB at 43.8 ms, −20 dB at 99 ms. Strike level
is −12 dB.

### 2.4 Which sound is which knock

The code doesn't match "one single knock among the tool tones". The most likely
reading of the owner's description:

- **Single hard knock:** an event sound, either **message accepted** (660 Hz sine) or
  **turn interrupted / message rejected** (440 Hz triangle). They are the only
  single-strike sounds, and they are about 18 dB harder than a tool strike.
  **Neither flashed before Phase 2** (§9), so this knock had no visual at all.
- **Three smaller knocks:** the **Edit** syllable (or Agent: same shape, lower
  notes, slightly slower). It is quiet (−24 dBFS peak), fast (62 ms apart) and
  strikes three times. A less likely alternative is the waiting arpeggio (3 notes,
  500 ms apart, looping).
- The rest (the third sound) is most likely the two-strike tool syllables (Read,
  Bash, Grep, …), which are the most common sound in a session.

**Owner to confirm (Q1).** The design below doesn't depend on the answer, because
every flash is derived from its own sound's parameters. The answer does decide
whether Phase 2 (event sounds) is required or optional.

## 3. Design

### 3.1 One source of truth: flash patterns are derived from synth params

A sound's flash is a **strike pattern**:

```ts
interface FlashStrike { atMs: number; intensity: number } // intensity 0–1
interface FlashPattern { strikes: FlashStrike[] }
```

It is computed by pure functions from the **same params the synth plays**:

- `flashPatternForSyllable(p: SyllableParams)`: one strike per tone, at
  `i × (p.durationMs + p.gapMs)`.
- `flashPatternForCategory(c: SoundCategory)`: one strike at 0, plus one at
  `second.delayMs` if present. Built from `synth-fallback.ts`'s `paramsFor`, which
  must be exported (or its timing lifted into a shared table).
- (Phase 3) `flashPatternForWaitingCycle()`: strikes at 0/500/1000, built from
  `waiting-tone-player.ts`'s constants, which must be exported.

No timing number is written twice. Changing a syllable in `tool-tones.ts` changes
its flash automatically.

### 3.2 Intensity: from strike level, compressed

Each strike's intensity comes from its level in §2 (energy at default volumes,
relative to the loudest sound, ΔL dB ≤ 0):

```
intensity = clamp(2^(ΔL / 20), FLASH_MIN_INTENSITY = 0.5, 1)
```

`2^(ΔL/10)` is perceived loudness (+10 dB ≈ twice as loud). The extra halving of
the exponent reflects that brightness perception compresses more strongly than
loudness does, and it keeps quiet sounds visible. Result:

| Sound | ΔL | Flash intensity |
|---|---|---|
| success, error | 0 dB | 1.00 |
| info (accepted), warning (interrupted/rejected) | −1.9 dB | 0.94 |
| waiting-loop note | −12 dB | 0.66 |
| curated tool strike (Read … Agent) | −18.1 to −19.5 dB | 0.51–0.53 |
| hashed tool strike | −17.0 to −21.3 dB | 0.50–0.56 (only the shortest triangle syllables reach the 0.5 floor) |

(Corrected during Phase 1: this table first said every tool strike sits at the
0.5 floor. The formula puts them just above it; the tests caught the mismatch.)

The levels are computed **once, from default volumes**, by a pure helper that reads
the same constants as the players (`ENVELOPE_PEAK`, `DEFAULT_*_VOLUME`, the
category peaks). They are not recomputed from the user's live volume sliders.
The flash has to keep working at volume 0 and before the AudioContext is primed
(existing requirement, flash spec §2.2), and a volume slider shouldn't dim it (Q4).

The per-strike intensity scales the overlay's opacity. Each stylesheet still owns
"full strength" (pill 0.9 alpha, tab 0.12 alpha, flash spec Revision 3). Intensity
1.0 is today's full-strength click.

**Consequence the owner must accept or tune (Q2):** tool-tone flashes get dimmer
than today, from full strength to about half (pill ≈ 0.46–0.50 alpha peak). That is the honest
result: tool tones really are about 18 dB quieter than event sounds. The hard knock
then reads as roughly twice as bright as each small knock. To keep tool flashes
brighter, raise `FLASH_MIN_INTENSITY`. The ratio between the two knocks narrows
accordingly.

### 3.3 Envelope per strike

The audio strikes are percussive: about 10 ms to −6 dB, about 20 ms to −20 dB
(tool). At 60 Hz a frame is 16.7 ms, so the visual can't copy the audio decay
literally: it would last one frame. Each strike instead gets:

| Phase | Value | Why |
|---|---|---|
| Attack | instant jump to `intensity` | Audio attack is 6–10 ms (20 ms for waiting), under one frame |
| Hold | **20 ms** | Guarantees at least one 60 Hz frame at peak, whatever the phase relative to vsync |
| Decay between strikes | `ease-out` down to **15 % of the strike's intensity**, reached exactly at the next strike's onset | The dip is what makes 2 or 3 strikes read as separate pulses. Gaps are 41–75 ms (2.5–4.5 frames) for tool tones, 50–70 ms for event doubles |
| Final tail (after the last strike) | `ease-out` to 0 over **280 ms** | Same 300 ms visible length as today's owner-approved single click |

Resulting shapes:

- **Single hard knock** (info/warning): one bright pulse (0.94), then a 300 ms fade.
  Looks like today's click, slightly stronger.
- **Three small knocks** (Edit): three half-strength pulses at 0/62/124 ms, dipping
  to 15 % between them, then a 280 ms tail from the third.
  Total length 124 + 300 = 424 ms.
- **Two-strike tool syllable** (Read): two half-strength pulses at 0/74 ms, then the
  tail.
- **Waiting loop** (Phase 3): three pulses at 0.66, 500 ms apart. Each fully decays
  before the next (the 280 ms tail is shorter than the 500 ms gap), repeating every
  2.5 s.

Keyframes are produced by **sampling a pure `envelopeAt(patterns, t)` function**
every 8 ms (≥ 120 Hz), with a duplicate-offset keyframe pair at each onset for the
instant jump. WAAPI allows equal offsets. Every pattern then goes through one code
path, including merged ones (§3.4), and the tests assert on the pure function. A
500 ms pattern is about 70 keyframes of one `opacity` property; it still runs on
the compositor, and nothing runs on the main thread per frame.

### 3.4 Overlapping sounds: merge, don't restart

Audio overlaps: two syllables started 40 ms apart both play in full. The current
flash instead **restarts** on each tone and **drops** any restart within 100 ms
(`FLASH_THROTTLE_MS`). That throttle would delete the 2nd and 3rd pulses of
every syllable, so it has to go.

New rule: each element keeps its live patterns (absolute start time + pattern).
When a new one arrives, drop the finished patterns, set the envelope to the
**per-instant maximum** of all live patterns, cancel the running animation, and
start a new one from "now" (sampled per §3.3). The visual then matches the sum of
the sounds at every instant, up to its brightest component. A burst of parallel
tool calls shows as a dense cluster of pulses, which is what it sounds like.

### 3.5 Only strikes that actually sound

The flash must mirror the audio's own dedup, so it never shows a pulse you didn't
hear:

- **Tool tones:** `ToolTonesPlayer.play` drops a repeat of the same tool within
  30 ms (`tool-tones-player.ts:29,86-88`). The flash is emitted **before** that
  check today (`sound-service.ts:332-338`), so a coalesced repeat flashes without
  sounding. Lift the coalesce decision into a method both paths use
  (`toolTones.claim(tool, now): boolean`). Emit the flash and play the tone only
  when it returns true. This works unprimed too, since the coalesce map lives on the
  player, not the context.
- **Event sounds:** emit the flash inside the bus subscriber (`sound-service.ts:172-189`)
  **after** `shouldPlay` and the per-id `coalesceMs` check, and **before**
  `player.play`. It inherits the gates (master, per-event toggle, focus
  suppression, replay mode). Only emit when `ev.sourceBlockId` is set.

### 3.6 Timing alignment with what you hear

Current behavior: the flash starts on the next animation frame (about 0–17 ms). The
tone starts at `ctx.currentTime`, the start of the next audio render quantum, and
reaches your ears after the output device's latency. On Windows that is typically
tens of ms (unmeasured here). So today the flash usually **leads** the sound.

Change:

1. `toolTones.play` / `player.play` return the context time the first strike was
   scheduled at (`startAt`).
2. Convert it to page time with `ctx.getOutputTimestamp()`:
   `audibleAtPerf = ts.performanceTime + (startAt − ts.contextTime) × 1000`.
   If that returns zeros (context just resumed), use
   `now + (ctx.baseLatency + ctx.outputLatency) × 1000`.
3. Start the flash animation with `delay = max(0, audibleAtPerf − performance.now() − FLASH_VISUAL_LEAD_MS)`.
   `FLASH_VISUAL_LEAD_MS` defaults to **16** (about one frame of compositor and
   display latency) and gets calibrated in §6.
4. Unprimed context, or no audio played: `delay = 0`. There's no sound to align to.

**Target:** each visual pulse peaks within **±20 ms** of its strike's audible
onset. For reference, ITU-R BT.1359 puts the detection threshold for A/V offset at
about 45 ms (sound early) and 125 ms (sound late), so ±20 ms is comfortably
invisible, and it is about the resolution a 60 Hz display allows anyway.

### 3.7 Targets and settings

- Targets are the same as today: the source's window tab and its pane pill, both for
  every sound that flashes. Event sounds use the same `{ blockId }` routing.
- `FlashTarget` becomes `{ blockId, pattern, delayMs }`. `tab.tsx:234-235` and
  `PaneTabStrip.tsx:571-572` pass them through to `flashElement`.
- Setting: rename `notify:tooltones:flash`'s **label** to "Flash the tab and pane
  when a sound plays". Keep the key; event sounds and the waiting loop honor it too.
  This saves a new schema key for what users see as one feature. The alternative
  (a separate `notify:sounds:flash`) is listed in Q5.
- Reduced motion: unchanged (flash spec Revision 2, item 4, owner decision). This is
  still opacity in place, nothing moves.

### 3.8 Photosensitivity

A 3-strike syllable is three flashes within 124 ms, so any target can now exceed
WCAG 2.3.1's "three flashes in one second". The design relies on the **small-area
exemption**, the same argument already recorded at `activity-flash.ts:57-62`. Now
it carries the whole case, so update that comment:

- Per agent, the flashing area is a pill (~120×20 px) plus a tab (~200×33 px),
  about 9,000 px². The general-flash area threshold is about 341×256 ≈ 87,000 px²
  (25 % of a 10° field at the reference viewing distance). It would take about 9
  agents flashing **in the same instant** to reach it. Parallel strikes across
  agents are rare and brief.
- The window-tab tint is 0.12 alpha, which is probably below WCAG's 10 %
  relative-luminance change that defines a "flash" at all. The pill at 0.45–0.9
  alpha is the one that counts.
- Saturated-red pane colors fall under the red-flash rule, and the same small-area
  exemption applies.
- The waiting loop (Phase 3) averages 1.2 pulses/s in 0.5 s spacing, under 3/s
  even without the exemption.

## 4. Phases

1. **Tool-tone patterns.** §3.1 (syllables), §3.2, §3.3, §3.4, §3.5 (tool path),
   §3.6. This makes "three smaller knocks" visible as three pulses, and every
   two-strike syllable as two.
2. **Event-sound flashes.** `flashPatternForCategory` and the bus hook (§3.5).
   This gives the single hard knock its visual. **Required if Q1 confirms the hard
   knock is an event sound**, which the code strongly suggests.
3. **Waiting loop** (owner call, Q3). One pattern per cycle, emitted from
   `WaitingTonePlayer.scheduleNextCycle` through a callback the service passes in.
   It stops with the loop, including when the focus-suspend path pauses it.

## 5. Files

| File | Change |
|---|---|
| `frontend/app/notification/activity-flash.ts` | `FlashPattern`, `envelopeAt`, keyframe sampler, merge-per-element, `delayMs`, remove `FLASH_THROTTLE_MS`, new constants (`FLASH_STRIKE_HOLD_MS` 20, `FLASH_INTER_STRIKE_FLOOR` 0.15, `FLASH_TAIL_MS` 280, `FLASH_MIN_INTENSITY` 0.5, `FLASH_VISUAL_LEAD_MS` 16, sample step 8 ms), updated photosensitivity comment |
| new `frontend/app/notification/sound/flash-patterns.ts` | `flashPatternForSyllable`, `flashPatternForCategory`, (P3) `flashPatternForWaitingCycle`, `strikeLevelDb` / `intensityFor`; imports player constants only |
| `tool-tones-player.ts` | export `ENVELOPE_PEAK`, the 6 ms attack; `claim(tool)`; `play` accepts params and returns `startAt` |
| `synth-fallback.ts` | export category params and the 10 ms / 150 ms envelope constants; return `startAt` |
| `sound-player.ts` | `play` returns `startAt \| null` |
| `waiting-tone-player.ts` | (P3) export constants; optional per-cycle callback |
| `sound-service.ts` | reorder the tool path (claim → flash → play); event-sound flash in the bus subscriber; audible-time conversion helper |
| `tab.tsx`, `PaneTabStrip.tsx` | pass `pattern` and `delayMs` through |
| `sounds-section.tsx` | relabel the toggle |
| `SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md` | add a Revision 4 pointer to this spec |

## 6. Tests and verification

Unit tests (vitest):

- `flash-patterns`: every curated tool yields onsets exactly `i × (d + g)` and one
  strike per tone. Edit → `[0, 62, 124]`, Read → `[0, 74]`. A hashed tool yields
  2 strikes. info/warning → 1 strike, success → `[0, 70]`, error → `[0, 90]`.
  Intensities match the §3.2 table within 0.01.
- `envelopeAt`: equals `intensity` from each onset through onset + 20 ms. It
  reaches 0.15 × intensity at the next onset, jumps back to peak at that onset, and
  hits 0 at last onset + 300 ms. The merge of two patterns equals the per-instant max.
- Keyframes: they contain a duplicate-offset pair at every onset, the offsets are
  non-decreasing, and the duration is `last onset + 300`.
- Sound service: a coalesced tool repeat (same tool within 30 ms) produces neither a
  tone nor a flash. An event sound blocked by focus suppression doesn't flash, and
  one that passes flashes with its category pattern. Unprimed → `delayMs` 0.
- Update the existing throttle tests in `activity-flash.test.ts` (they encode the
  behavior being removed).

Live verification in a dev build, both required before calling it done:

1. **CDP keyframe check:** trigger Edit, Read, and a turn interrupt in a background
   pane. Read `el.getAnimations({ subtree: true })` on the pill and tab and confirm
   the pulse count, spacing, and peak ratios against §2.
2. **A/V sync capture:** record screen plus system audio at ≥ 120 fps (OBS). Step
   frames and compare each overlay peak with its waveform onset. Every pulse must be
   within ±20 ms of its strike. Calibrate `FLASH_VISUAL_LEAD_MS` from the measured
   mean offset, and log `ctx.baseLatency` / `ctx.outputLatency` on the owner's
   machine in the PR description.
3. The owner can tell the single hard knock from the three small knocks **with the
   sound muted**, by the flash alone.

## 7. Open questions for the owner

1. **Which sound is the "single hard knock"?** Code says no tool tone is a single
   strike. The only single-strike sounds are *message accepted* (660 Hz sine) and
   *turn interrupted / message rejected* (440 Hz triangle), and neither flashes
   today. If that's the one, Phase 2 is required. Is "three smaller knocks" Edit, or
   the waiting arpeggio?
2. **Tool flashes at half strength** (§3.2) so the hard knock can be twice as
   bright. OK, or raise `FLASH_MIN_INTENSITY`?
3. **Waiting loop flashes** (Phase 3)? It would pulse the tab 3× every 2.5 s for as
   long as the agent waits, which overlaps the taskbar attention badge (#3662).
4. **Volume sliders:** keep flash intensity independent of them (recommended), or
   dim a sound's flash when its volume is turned down?
5. **Window tab contrast:** at 0.12 alpha, the dips between Edit's three pulses are
   0.12 → 0.018 → 0.12. That may be too subtle to read as three on the tab; the pill
   will read clearly. Leave it, or raise the tab's full strength (e.g. 0.2)?
6. **One toggle or two:** reuse `notify:tooltones:flash` for all sounds (proposed),
   or add a separate `notify:sounds:flash`?

## 8. What shipped: Phase 1 (2026-09-24)

Code:

- `frontend/app/notification/activity-flash.ts`: `FlashStrike`/`FlashPattern`,
  `FlashTarget = { blockId, pattern, delayMs }`, the pure envelope
  (`patternEnvelopeAt`, `envelopeAt` for merges, `easeOut` = CSS
  `cubic-bezier(0, 0, 0.58, 1)`), `buildFlashKeyframes` (8 ms samples + hold ends
  + a same-offset pair per strike), and `flashElement(el, { pattern, delayMs },
  baseColor?)` with per-element merging. `FLASH_THROTTLE_MS` and the fixed
  40/300 ms envelope are gone. The photosensitivity comment now rests on the
  small-area exemption (§3.8).
- New `frontend/app/notification/sound/flash-patterns.ts`:
  `flashPatternForSyllable`, `intensityForLevel`, `syllableStrikeLevelDb`,
  `categoryStrikeLevelDb` (the level reference, and Phase 2's input),
  `audibleFlashDelayMs`, `FLASH_MIN_INTENSITY`, `FLASH_MAX_AUDIO_DELAY_MS`.
- `tool-tones-player.ts`: exports its envelope constants and
  `TOOL_TONE_COALESCE_MS`; `play()` returns the scheduled context time, or null
  when nothing played. `synth-fallback.ts`: exports `synthParamsFor` and its
  envelope constants (playback unchanged).
- `sound-service.ts`: the tool path plays first, then emits the flash with the
  tool's pattern and the audible-time delay. `tab.tsx` and `PaneTabStrip.tsx`
  pass the target through.

Departures from §3:

1. **Flash dedup is per pane, not a shared `claim()` (§3.5).** The tone coalesces
   by tool name alone, so Read from two panes 5 ms apart plays one syllable. Under
   §3.5 as written, the second pane would get no flash, although it did make that
   sound; its tab and pill are the only way to see that. The flash coalesces by
   (pane, tool) over the same `TOOL_TONE_COALESCE_MS` instead. A pane's own
   repeats (parallel tool calls) still show as one strike, like the audio, and
   every strike shown is one you heard. The player's own coalesce is unchanged,
   but a coalesced `play()` now returns the start time of the syllable it was
   folded into (not null). Otherwise the second pane's flash would fire
   undelayed, ahead of the sound both panes share (ReAgent P1 on #3717).
   The delay can also be **negative**: when that shared syllable is already
   audible, `flashElement` places the pattern in the past so it resumes where
   the sound is, instead of restarting up to ~46 ms behind it (Codex P2 on
   #3717). The cap is ±500 ms, and a pattern that is already over is skipped
   without interrupting the element's current flash.
2. **The animation's start time is pinned** to the moment `flashElement` runs
   (`anim.startTime = performance.now()`). Otherwise a new animation starts on the
   next frame, up to ~17 ms late, and every strike lands that much after its
   sound. `FLASH_VISUAL_LEAD_MS` therefore covers only compositor and scan-out
   latency.
3. **A 500 ms cap on the audio delay** (`FLASH_MAX_AUDIO_DELAY_MS`), not in §3.6.
   Bluetooth output legitimately runs 150–300 ms; a larger reported latency is
   more likely wrong than real. Non-finite values fall back to 0.
4. **Tool intensities are 0.50–0.56, not all at the floor** (see the corrected §3.2
   table).

Verification:

- Unit tests: `__tests__/activity-flash.test.ts` (envelope holds, dips, tail, merge
  maximum, keyframe pairs, delay, merge/cancel, base color),
  `sound/__tests__/flash-patterns.test.ts` (every §2.1 onset, level range,
  intensity mapping, audible delay with output timestamps, the latency fallback,
  and the cap), `sound-service.test.ts` (pattern per tool, per-pane dedup,
  delay when primed, 0 when not), and the tab and pill suites (pattern and delay
  reach the animation). Typecheck (`tsconfig.citypecheck.json`) is clean.
- **Real-Chromium check** (headless Chrome, the real modules bundled with
  esbuild): Edit's pattern on a real `::before` overlay produced one pseudo-element
  animation with 61 keyframes, the pinned start time was honored, and Chromium's
  computed opacity matched `patternEnvelopeAt` within 0.0002 at 0, 10, 19, 40,
  61.9, 62, 70, 123.9, 124, 200, 300 and 423 ms. That confirms three pulses at
  0/62/124 ms (0.518 peak), each dipping to 0.078 (15 %) just before the next,
  then the tail.
- **Not yet done:** §6 items 1–3 in a dev build (CDP keyframe check in the app, the
  A/V capture, and calibrating `FLASH_VISUAL_LEAD_MS`), and the owner's muted
  side-by-side comparison. Those need the running app and someone listening.
- **Owner check (2026-09-25):** the owner ran a `task dev` build of the rebased
  branch and approved the result ("it is good"). The A/V capture and
  `FLASH_VISUAL_LEAD_MS` calibration were not done.

## 9. What shipped: Phase 2, event sounds (2026-09-25)

The single hard knock gets its visual. Every event sound that has a source pane
(turn complete, errored, interrupted; message accepted, rejected) now flashes that
pane's pill and window tab with its own pattern:

| Sound | Strikes | Intensity |
|---|---|---|
| info: message accepted | 1 at 0 ms | 0.94 |
| warning: turn interrupted, message rejected | 1 at 0 ms | 0.94 |
| success: turn complete | 0, 70 ms | 0.997 |
| error: turn errored, submit timed out | 0, 90 ms | 1.00 |

That is about 1.8× a tool knock (0.51–0.56), per §3.2. Q1 did not need answering
first: whichever of these is the owner's hard knock, its flash is derived from its
own synth parameters.

Code:

- `flash-patterns.ts`: `flashPatternForCategory(c)`, built from
  `synthParamsFor(c)` (a strike at 0, plus one at `second.delayMs`), with
  intensity from `categoryStrikeLevelDb`.
- `synth-fallback.ts` / `sound-player.ts`: `playSynthFallback()` and
  `SoundPlayer.play()` return the context time the sound was scheduled at (null
  when not primed). The asset path returns it too.
- `sound-service.ts`: the bus subscriber emits the flash right after
  `player.play()`, so past every gate the sound passed (master switch, per-event
  toggle, focus suppression, per-id coalesce, replay). It uses the same
  audible-time delay as tool tones (§3.6) and fires unprimed with delay 0.
  Sounds with no `sourceBlockId` play but can't flash anything.
- **Setting (Q6, as proposed in §3.7):** the one `notify:tooltones:flash` key now
  covers all sounds. It is relabeled "Flash the tab and pane when a sound plays"
  and moved out of the tool-call-tones block, which hides it when tones are off,
  to sit under the master sound switch. The settings row id and search keywords
  are kept. The schema description, the Rust doc comment and the settings
  template say it covers all sounds.

No departures from §3.5. Note that the flash follows focus suppression, so the focused
pane in a focused window doesn't flash for its own turn-complete, because the sound
is suppressed too. That matches "flash exactly when the sound plays".

Known limit: patterns come from the synth. No event sound ships an asset file
today (`sounds.ts`). If one ever does, its pattern must come from the recording,
and the comment on `flashPatternForCategory` says so.

Tests: `flash-patterns.test.ts` (category strike timing and intensity, the hard
knock about 1.8× a tool knock) and a new "event-sound flash" suite in
`sound-service.test.ts` (per-sound pattern, every gate, focus suppression, the
shared toggle, no-source events, and the audible delay when primed).
