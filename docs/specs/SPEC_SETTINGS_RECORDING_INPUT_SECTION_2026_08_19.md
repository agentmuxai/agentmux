# SPEC — Settings: new "Recording / Input" section (mic setup, engine config, test-your-mic)

**Date:** 2026-08-19
**Type:** Feature
**Status:** Implemented — merged in #2751 (2026-08-22), plus two follow-up review rounds fixing a renderer-side plaintext-key leak (`voice:groqApiKey` was reaching the renderer unredacted despite this doc's own "never sent to the renderer" claim below — see `agentmux-srv/src/backend/wconfig/redact.rs`), an engine-switch bug affecting already-open panes, and the whisper-local/model-path selector logic. See `frontend/app/view/settings/sections/recording-section.tsx` for the shipped implementation; details below are the original design and may differ in small particulars from what landed.
**Scope:** New `frontend/app/view/settings/sections/recording-section.tsx` (+ registration in `settings-view.tsx`/`settings-model.ts`), a small new backend validation endpoint, and light refactors to `frontend/app/hook/whisperVoiceEngine.ts` and `frontend/app-init.ts`'s error-toast copy. No change to the capture/transcribe architecture itself.

## Problem

Voice input is a fully-shipped, real feature — three STT engines, a per-pane mic button (Agent composer + Terminal header), a hotkey, permission handling, error toasts — but it has **zero Settings UI**. Every knob (`voice:enabled`, `voice:engine`, `voice:groqApiKey`, `voice:whisperCliPath`, `voice:whisperModel`, `voice:whisperModelPath`) is `settings.json`-only: a user has to already know these keys exist, hand-edit raw JSON including a plaintext API key, and has **no way to find out whether it's configured correctly until they try to record and something fails** — at which point the error copy is generic and, in one case, actively wrong (see below).

This was a deliberate v1 scope cut, not an oversight — `SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md` §Phase 3 named `voice:enabled` as a planned Settings toggle and it was simply never built past the schema; the engine/credential keys added later never had a UI plan at all. This spec closes that gap.

## Current state (grounded — see research citations throughout)

- **Schema** (`schema/settings.json:188-212`): `voice:enabled` (bool, default true), `voice:engine` (enum `groq`/`whisper-local`/`webspeech`, default `groq`), `voice:groqApiKey` (string, server-only, `AGENTMUX_GROQ_API_KEY` env takes precedence), `voice:whisperCliPath` (string, `AGENTMUX_WHISPER_CLI` override), `voice:whisperModel` (string, default `base.en`), `voice:whisperModelPath` (string, explicit model file, skips auto-download).
- **Engines**: `groq` (cloud, needs an API key, `agentmux-srv/src/server/voice.rs:31-32,144-213`) and `whisper-local` (offline, needs a real `whisper-cli` binary + a GGML model — auto-downloaded by default, `voice.rs:236-312`) are both fully implemented in one file, `frontend/app/hook/whisperVoiceEngine.ts`. `webspeech` is a dev-only escape hatch that cannot function in a packaged build at all (closed-source Google service bound to real Chrome, `SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md`'s Phase-4 pivot) — `useVoiceInput.ts:161-163` already silently falls through to the whisper path if picked and the browser API is absent, so this is truly a "leave it alone unless you're a dev running `task dev:standalone` in real Chrome" option.
- **No device picker** — the only `getUserMedia` call in the app (`whisperVoiceEngine.ts:313`) has no `deviceId` constraint; `navigator.mediaDevices.enumerateDevices()` is not used anywhere. AgentMux always gets whichever mic the OS resolves as default input.
- **No pre-flight validation** — `whisperCliPath`/`whisperModelPath` are only existence-checked at the moment of an actual transcription attempt (`voice.rs:328-330`, `246-252`), server-side, and the specific reason ("not found at {path}" vs. a crashed/timed-out subprocess) is discarded before reaching the frontend — every failure mode collapses to the same generic `service-not-allowed` toast (`whisperVoiceEngine.ts:171-175`).
- **Known-stale error copy**: the `service-not-allowed` toast (`frontend/app-init.ts:956-960`) still reads *"Speech recognition isn't available in this build yet. Server-side transcription is in progress."* — accurate when only the dead Web Speech engine existed, actively misleading now that Groq/whisper-local are live (this toast today usually means "your API key/CLI path is wrong," not "the feature doesn't exist yet").
- **No existing "test X" pattern anywhere in the app** to imitate (Sounds section has sliders but no "play test sound" button either) — this section introduces the pattern for the first time.
- **A real, reusable implementation precedent exists for the live-level meter**: `whisperVoiceEngine.ts`'s `pollLevel()` (lines 230-242) already computes RMS from an `AnalyserNode` via `getByteTimeDomainData` — today purely to drive silence-detection VAD, its number is never rendered. A test-mic meter is substantially "expose this existing computation," not new capture code.
- **A real UI precedent exists for a masked credential field**: `frontend/app/view/identity/identity-account-form.tsx` (Armory account form) — masked display + "Replace key" unlock, `type="password"` entry, an egress-transparency note naming the exact destination URL, dual "Validate & Save" / "Save without validating" actions. `voice:groqApiKey` is a flat `settings.json` string (not keychain-backed like Armory's), so this spec borrows the *shape* (masked-by-default, egress note, validate action) without the full keychain lifecycle.

## Design

### 1. Section placement and top-level layout

New rail entry **"Recording"** in `settings-view.tsx`'s `RAIL` array (`frontend/app/view/settings/settings-view.tsx:48-54`), positioned after "Sounds" (same "hardware/IO" cluster) and before "Advanced". New file `frontend/app/view/settings/sections/recording-section.tsx`, following the existing section-file convention (`SectionHeader`/`SettingRow`/`ToggleControl` from `../settings-controls`, a local `s()` settings-atom accessor — see `sounds-section.tsx` for the exact shape to mirror).

Row order (top to bottom):

1. **Enable voice input** — `ToggleControl` on `voice:enabled` (default true). Gates everything below, same `<Show when={enabled()}>` pattern `sounds-section.tsx` uses for its own master toggle.
2. **Transcription engine** — a `<select>` (same pattern as the existing `notify:tooltones:scope` select, `sounds-section.tsx:120-131`) bound to `voice:engine`, options **Groq (cloud) / whisper.cpp (local, offline) / Web Speech (dev only)**. The third option is only rendered when `isDev()` is true (mirrors the existing `isDev()` gate already used elsewhere in this codebase, e.g. `StatusBar.tsx`'s DEV badge) — hiding a non-functional-in-production option is a straightforward, low-risk UX improvement over letting a packaged-build user pick something that can never work.
3. **Engine-specific config**, shown/hidden based on the selected engine (`<Show when={engine() === "groq"}>` / `<Show when={engine() === "whisper-local"}>`):
   - **Groq**: one masked API-key field. See §2 below.
   - **whisper-local**: CLI path + model config. See §3 below.
4. **Test your microphone** — a self-contained subsection with a live level meter and a Start/Stop test button, independent of engine choice (it validates capture + the currently-selected engine end-to-end). See §4 below.
5. **Microphone (input device)** — a `<select>` of available audio input devices. See §5 below (new capability, no existing infra).

### 2. Groq API key field

```tsx
<SettingRow
  label="Groq API key"
  control={<MaskedKeyField
    value={s()["voice:groqApiKey"] as string | undefined}
    onSave={(key) => set("voice:groqApiKey", key)}
    placeholder="paste key — never displayed again after saving"
  />}
/>
```

`MaskedKeyField` is a small new shared control (add to `settings-controls.tsx` alongside `SliderControl`/`ToggleControl`, since a masked-secret input is a reasonable general-purpose primitive other settings — messaging-bridge bot tokens, see the companion audit spec — will want too):
- **At rest** (a key is already saved): shows `••••••••` (no partial/tail hint — unlike Armory's keychain-backed masked_tail, this is a flat settings.json value with no separate "masked_tail" metadata field returned by the backend, so don't invent one) plus a small "Replace" link/button that reveals the entry state.
- **Entry state**: `<input type="password" autocomplete="off" spellcheck={false} />`, save/cancel.
- **Egress note**, directly under the field, same convention as `identity-account-form.tsx:362-380`: *"Sent once, over HTTPS, from the AgentMux backend on this machine directly to `api.groq.com` — never to any other AgentMux service."*
- No "Validate & Save" probe call for v1 (unlike Armory, there's no existing lightweight "list models" endpoint to validate against cheaply) — saving just persists the value. The "Test your microphone" flow in §4 doubles as the validation step (a real transcription round-trip against whichever engine is currently selected), so a separate synchronous key-validation call isn't necessary to get an accurate signal.
- Also surface the effective-override note: if `AGENTMUX_GROQ_API_KEY` is set in the process environment, show `(overridden by AGENTMUX_GROQ_API_KEY env var)` next to the label and disable the field — mirrors the existing "env wins" precedence documented in `voice.rs:125-133` and avoids a user changing a setting that's silently ignored.

### 3. whisper-local config

```tsx
<SettingRow label="whisper-cli path" control={<PathField
  value={s()["voice:whisperCliPath"] as string | undefined}
  onChange={(v) => set("voice:whisperCliPath", v)}
  placeholder="/usr/local/bin/whisper-cli"
  status={cliStatus()}  // "checking" | "found" | "not-found" | "unknown"
/>} />
<SettingRow label="Model" control={<select ...>base.en / small.en / medium.en / custom path…</select>} />
<Show when={modelChoice() === "custom"}>
  <SettingRow label="Model file path" control={<PathField value={...} onChange={(v) => set("voice:whisperModelPath", v)} status={modelStatus()} />} />
</Show>
```

- **`PathField` needs a new backend check to show live ✓/✗ status** — there is no existing endpoint for this (confirmed §5 of the research: validation today only happens at actual transcribe time). Add a small new RPC, e.g. `voice.checkPath { kind: "cli" | "model", path: string } -> { exists: bool }`, implemented as a thin wrapper around the exact same `Path::new(&p).exists()` check `voice.rs` already does inline (`voice.rs:328-330`, `246-252`) — no new validation LOGIC, just exposing the existing check proactively instead of only at first-recording-attempt. Debounce the frontend call (300-500ms after the user stops typing, matching `SliderControl`'s existing 180ms-debounce convention at `settings-controls.tsx:50-71` for the general shape) so it doesn't fire on every keystroke.
- **No auto-detection of a local whisper.cpp install in v1** — confirmed no existing scan-common-locations logic anywhere in `voice.rs`; building one (checking `$PATH`, common Homebrew/scoop/apt install dirs per platform) is a reasonable follow-up but out of scope here. The path field's placeholder text and a "Don't have whisper.cpp? ↗" help link (to wherever the project's own install docs live, if any exist — verify before shipping) partially substitutes for it.
- Model dropdown options (`base.en`/`small.en`/`medium.en`/"custom path…") reflect `voice:whisperModel`'s "auto-download on first use" behavior (`voice.rs:236-312`) — selecting a named model does NOT require the user to already have it; picking "custom path…" switches to the explicit `voice:whisperModelPath` field, which DOES require an existing file (per §2 of the research, model-name and model-path share one env-override variable, `AGENTMUX_WHISPER_MODEL` — a footnote sentence under the model row, *"Only one of Model or Model file path applies at a time — file path takes precedence if both are set,"* prevents the ambiguity from being silent).

### 4. Test your microphone

The centerpiece the user specifically asked for, and the first "test X" interaction pattern in the app — designed fresh (no precedent to copy), informed by the "action button + live feedback" shape `identity-account-form.tsx`'s Validate & Save uses.

```
┌─────────────────────────────────────────────────────┐
│  Test your microphone                                │
│                                                        │
│  [ ● Start test ]   ▁▂▅▇▆▃▁▂▃▅▇█▆▃▁  (live level bar) │
│                                                        │
│  "Say something…" → transcribed text appears here     │
└─────────────────────────────────────────────────────┘
```

- **Level meter**: a horizontal bar (CSS width driven by a signal, no canvas needed — cheapest correct implementation) fed by a small new hook `useMicLevelMeter()` that factors OUT the existing RMS-from-`AnalyserNode` logic already in `whisperVoiceEngine.ts:230-242` (`pollLevel()`) into a shared utility both the VAD and this new meter call, rather than duplicating the Web Audio boilerplate. This is a refactor-and-reuse, not new capture code.
- **Start test** calls `getUserMedia` with the device selected in §5 (or default if none picked), attaches the level meter, and after ~1s of live meter movement (proving capture works) automatically also runs one real segment through whichever engine is currently configured (reusing `whisperVoiceEngine.ts`'s existing segment-capture-and-post path, not a separate mock path — this is the ONLY way to genuinely validate an engine end-to-end, since engine misconfiguration is a server-side-only failure mode per §2/§5 of the research) and displays the transcribed text (or the specific error).
- **Error display fixes the exact gap the research found**: instead of the generic `service-not-allowed` collapse, the test flow's own result panel shows the server's actual error string (the 501/502 body text `voice.rs` already produces — `"whisper-cli not found at {cli}"`, `"whisper model not found at {p}"`, the CLI's stderr snippet, the 120s-timeout message) rather than routing through the lossy generic toast path. This requires a minor `whisperVoiceEngine.ts` change: today `postSegment()` (`whisperVoiceEngine.ts:151-184`) discards the response body on non-OK status; thread it through as `lastError`'s detail (additive — the existing coarse `lastError` categories used by `MicButton.tsx`'s tooltip logic stay unchanged, this only adds a detail string for surfaces that want it, like this test panel).
- Two independent partial-success states are distinguishable and both useful: level meter moves but transcription fails (capture fine, engine misconfigured — point the user at the engine config fields above) vs. level meter never moves (no mic / permission denied — point the user at OS privacy settings, reusing the exact copy `app-init.ts`'s `not-allowed` toast already has).
- **Also fix the stale toast** (`app-init.ts:956-960`): replace *"Speech recognition isn't available in this build yet. Server-side transcription is in progress"* with something that points at this new section, e.g. *"Voice transcription isn't configured — open Settings → Recording to set it up."* Small, independently shippable, and removes an actively wrong claim regardless of whether the rest of this spec lands in the same PR.

### 5. Microphone (input device) selection

New capability — confirmed nothing like it exists today.

```tsx
<SettingRow label="Microphone" control={<select
  value={s()["voice:inputDeviceId"] as string ?? "default"}
  onChange={(e) => set("voice:inputDeviceId", e.currentTarget.value)}
>
  <option value="default">System default</option>
  <For each={devices()}>{(d) => <option value={d.deviceId}>{d.label}</option>}</For>
</select>} />
```

- New setting `voice:inputDeviceId` (string, absent/`"default"` = current behavior, no schema change needed beyond adding this one key to `schema/settings.json` + `types.rs`, following the exact pattern every other `voice:*` key already uses).
- Populated via `navigator.mediaDevices.enumerateDevices()`, filtered to `kind === "audioinput"` — **note**: per the Web platform's own privacy model, device `label`s are empty strings until a `getUserMedia` permission grant has happened at least once in this origin; the dropdown should show generic labels ("Microphone 1", "Microphone 2") before first grant and the real labels after, and/or trigger a one-time silent permission probe (`getUserMedia({audio:true})` then immediately stop the track) when the Recording section first mounts, purely to unlock real device labels for this picker — a small, self-contained addition, not a change to when the app normally asks for mic permission (that still only happens on first real mic-button use, unchanged).
- Threading `deviceId` through to actual capture: `whisperVoiceEngine.ts:313`'s `getUserMedia({ audio: true })` call becomes `getUserMedia({ audio: deviceId && deviceId !== "default" ? { deviceId: { exact: deviceId } } : true })` — one-line change, reads the setting at capture-start time (same place `startCapture()` already reads other settings).
- The §4 test flow should use whatever's currently selected here, so "test your mic" and "actually recording in a pane" are guaranteed to exercise the identical device path — no separate device-selection UI inside the test panel itself.

## Non-goals

- **No automatic engine fallback** (e.g. auto-retry on `whisper-local` if `groq` fails) — the research confirmed this doesn't exist today and changing failure *behavior* (vs. failure *visibility*, which this spec is about) is a separate, riskier design decision or omitted from this spec.
- **No auto-bundling of the `whisper-cli` binary** — explicitly out of scope per `voice.rs:17-19`'s own v1-scope comment ("per-platform binary fetch is fragile"); this spec only makes the user-provided-path flow easier to configure and diagnose, not obsolete.
- **No auto-detection/scan-for-whisper.cpp** — noted as a reasonable follow-up in §3, not built here.
- **No change to the underlying capture/transcribe architecture, hotkey, or per-pane button placement** — this spec is additive Settings UI + two small, independently-valuable bug fixes (stale toast copy, discarded server error detail), not a re-architecture.
- **No re-litigation of the `groq`-vs-`whisper-local` default** — `SPEC_VOICE_STT_ENGINE_2026_06_20.md` originally planned `whisper-local` as the eventual default and that never happened; this spec's job is to make whichever engine is configured easy to verify, not to change the default. Flagging it in Settings copy (e.g. a one-line note next to the engine picker: *"whisper.cpp runs fully offline; Groq sends audio to Groq's API"*) is in scope as a transparency improvement, changing the default value itself is not.

## Open questions

1. **`MaskedKeyField` reuse** — this spec designs it for `voice:groqApiKey`, but the companion audit spec (`SPEC_SETTINGS_AUDIT_GOOD_PICKINGS_2026_08_19.md`) independently identifies the messaging-bridge bot tokens as needing the identical control. Recommend building it as a shared `settings-controls.tsx` primitive from the start rather than a Recording-section-local component, so the second consumer doesn't duplicate it — sequencing between the two specs' implementations is a scheduling call, not a design one.
2. **New `voice.checkPath` RPC's exact shape** — sketched functionally above; naming/wire-format should follow whatever convention `agentmux-srv/src/server/app_api/` already uses for the newest-added simple RPCs (check a recent small addition, e.g. the MCP/skill RPCs from `SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md`, for the current house style before implementing).
3. **Silent permission probe for device labels (§5)** — confirm this is acceptable UX (a mic-permission prompt appearing the first time a user opens Settings → Recording, before they've clicked anything) versus gating it behind an explicit "Show device list" affordance instead. Leaning toward the explicit-click version to avoid a surprise OS permission dialog from just opening a settings page, but flagging as a product call.
4. **Level-meter visual** — a CSS bar is the cheapest correct v1; a small canvas waveform (closer to what the "Test your microphone" mockup above suggests visually) is a nicer follow-up once the underlying `useMicLevelMeter()` hook exists, since the hook's output (a stream of RMS values) works for either rendering.

## References

- `docs/specs/SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md` — original per-pane mic button design; §Phase 3 named the never-built `voice:enabled` Settings toggle; §Phase 4 explains why Web Speech can't work in packaged builds.
- `docs/specs/SPEC_VOICE_STT_ENGINE_2026_06_20.md` — the Groq/whisper-local capture-and-send architecture; §3.2 documents the reversed default-engine decision.
- `docs/specs/SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md`, `SPEC_COMPOSER_STRIP_LAYOUT_MIC_CENTER_MODEL_DEFAULTS_2026_07_10.md` — current mic-button placement, unaffected by this spec.
- `frontend/app/hook/whisperVoiceEngine.ts`, `frontend/app/hook/useVoiceInput.ts`, `frontend/app/element/MicButton.tsx`, `frontend/app-init.ts` (voice error toast) — current implementation this spec builds on top of.
- `agentmux-srv/src/server/voice.rs` — server-side engine resolution, validation, and error responses this spec's new endpoint and error-detail plumbing depend on.
- `frontend/app/view/identity/identity-account-form.tsx` — masked-credential-field UX precedent.
- `frontend/app/view/settings/sections/sounds-section.tsx`, `frontend/app/view/settings/settings-controls.tsx` — Settings-section and shared-control conventions this spec follows.
