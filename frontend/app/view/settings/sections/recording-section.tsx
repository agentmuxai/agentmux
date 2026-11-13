// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings -> Recording — voice input is a fully-shipped feature (3 STT
 * engines, per-pane mic button) with zero prior Settings UI; every knob was
 * settings.json-only. See docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md
 * for the full design this section implements.
 */
import { t } from "@/app/i18n/i18n";
import { Button, Select, TextInput } from "@/app/element/ui";
import { authHeaders } from "@/app/store/auth-headers";
import { createSignal, onCleanup, onMount, Show, For, type Accessor, type JSX, type Setter } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { isDev } from "@/app/store/misc-utils";
import { getWebServerEndpoint } from "@/util/endpoints";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createMicLevelMeter, type MicLevelMeter } from "@/app/hook/useMicLevelMeter";
import { pickMime } from "@/app/hook/whisperVoiceEngine";
import { settingEntry, type SettingsIndexEntry } from "../settings-model";
import { MaskedKeyField, SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";

// ── Search index — see appearance-section.tsx's header comment for the pattern. ──

export const RECORDING_SETTINGS = {
    enableVoiceInput: settingEntry({
        id: "recording.enable_voice_input",
        labelKey: "settings.recording.enableVoiceInput.label",
        descriptionKey: "settings.recording.enableVoiceInput.description",
        section: "recording",
        keywords: ["microphone", "speech to text", "dictation", "voice:enabled"],
    }),
    engine: settingEntry({
        id: "recording.engine",
        labelKey: "settings.recording.engine.label",
        descriptionKey: "settings.recording.engine.description",
        section: "recording",
        groupKey: "settings.recording.group.transcriptionEngine",
        keywords: ["transcription engine", "stt engine", "whisper", "groq", "voice:engine"],
    }),
    groqApiKey: settingEntry({
        id: "recording.groq_api_key",
        labelKey: "settings.recording.groqApiKey.label",
        descriptionKey: "settings.recording.groqApiKey.description",
        section: "recording",
        groupKey: "settings.recording.group.transcriptionEngine",
        keywords: ["groq key", "api key", "voice:groqApiKey"],
    }),
    whisperCliPath: settingEntry({
        id: "recording.whisper_cli_path",
        labelKey: "settings.recording.whisperCliPath.label",
        section: "recording",
        groupKey: "settings.recording.group.transcriptionEngine",
        keywords: ["whisper path", "local transcription binary", "voice:whisperCliPath"],
    }),
    model: settingEntry({
        id: "recording.model",
        labelKey: "settings.recording.model.label",
        descriptionKey: "settings.recording.model.description",
        section: "recording",
        groupKey: "settings.recording.group.transcriptionEngine",
        keywords: ["whisper model", "transcription model", "voice:whisperModel"],
    }),
    modelFilePath: settingEntry({
        id: "recording.model_file_path",
        labelKey: "settings.recording.modelFilePath.label",
        section: "recording",
        groupKey: "settings.recording.group.transcriptionEngine",
        keywords: ["custom model path", "ggml model", "voice:whisperModelPath"],
    }),
    inputDevice: settingEntry({
        id: "recording.input_device",
        labelKey: "settings.recording.inputDevice.label",
        section: "recording",
        groupKey: "settings.recording.group.microphone",
        keywords: ["microphone device", "audio input", "voice:inputDeviceId"],
    }),
} satisfies Record<string, SettingsIndexEntry>;

type PathStatus = "idle" | "checking" | "found" | "not-found";

// ── PathField: text input + live existence status (voice.checkPath) ─────────

function PathField(p: { value: string | undefined; onChange: (v: string) => void; placeholder?: string }): JSX.Element {
    const [status, setStatus] = createSignal<PathStatus>("idle");
    let debounceTimer: ReturnType<typeof setTimeout> | null = null;
    onCleanup(() => { if (debounceTimer != null) clearTimeout(debounceTimer); });

    const checkNow = (path: string) => {
        const trimmed = path.trim();
        if (!trimmed) {
            setStatus("idle");
            return;
        }
        setStatus("checking");
        void RpcApi.CheckPathCommand(TabRpcClient, { path: trimmed })
            .then((r) => setStatus(r.exists ? "found" : "not-found"))
            .catch(() => setStatus("idle"));
    };

    onMount(() => checkNow(p.value ?? ""));

    return (
        <div class="setting-path-field">
            <TextInput
                class="setting-text"
                type="text"
                value={p.value ?? ""}
                placeholder={p.placeholder}
                onInput={(e) => {
                    const v = e.currentTarget.value;
                    if (debounceTimer != null) clearTimeout(debounceTimer);
                    debounceTimer = setTimeout(() => checkNow(v), 400);
                }}
                onBlur={(e) => p.onChange(e.currentTarget.value)}
            />
            <span
                class="setting-path-status"
                classList={{
                    "setting-path-status--found": status() === "found",
                    "setting-path-status--not-found": status() === "not-found",
                }}
            >
                <Show when={status() === "checking"}><i class="fa-solid fa-spinner fa-spin" /></Show>
                <Show when={status() === "found"}><i class="fa-solid fa-check" /></Show>
                <Show when={status() === "not-found"}><i class="fa-solid fa-xmark" /></Show>
            </span>
        </div>
    );
}

// ── Section ───────────────────────────────────────────────────────────────────

const WHISPER_MODEL_CHOICES = ["base.en", "small.en", "medium.en"];

type TestState = "idle" | "listening" | "transcribing" | "done" | "error";
type SettingsFn = () => Record<string, unknown>;

// ── Device picker (§5) ───────────────────────────────────────────────────────

function createDeviceList(): { devices: Accessor<MediaDeviceInfo[]>; refresh: () => Promise<void> } {
    const [devices, setDevices] = createSignal<MediaDeviceInfo[]>([]);
    const refresh = async () => {
        if (typeof navigator === "undefined" || !navigator.mediaDevices?.enumerateDevices) return;
        try {
            const all = await navigator.mediaDevices.enumerateDevices();
            setDevices(all.filter((d) => d.kind === "audioinput"));
        } catch {
            setDevices([]);
        }
    };
    onMount(() => void refresh());
    return { devices, refresh };
}

// ── Test your microphone (§4) ────────────────────────────────────────────────

interface MicTest {
    meter: MicLevelMeter;
    state: Accessor<TestState>;
    setState: Setter<TestState>;
    result: Accessor<string | null>;
    setResult: Setter<string | null>;
    engine: () => string;
    deviceId: () => string | undefined;
    refreshDevices: () => Promise<void>;
    stream: MediaStream | null;
    recorder: MediaRecorder | null;
    // Bumped by every runMicTest()/cancelMicTest() call. Async continuations
    // (setTimeout callbacks, the fetch round-trip) capture the generation
    // active when they were scheduled and check it's still current before
    // touching state — otherwise a Cancel during an in-flight test could not
    // stop it: stopping the recorder still fired its installed onstop (which
    // then proceeded to transcribe), and a stale fetch response could
    // overwrite state a new test had already moved past. See PR #2751 review.
    generation: number;
}

// Explicit-click only (not an auto-probe on section mount) so opening
// Settings never triggers a surprise OS mic-permission dialog by itself.
function createMicTest(engine: () => string, deviceId: () => string | undefined, refreshDevices: () => Promise<void>): MicTest {
    const [state, setState] = createSignal<TestState>("idle");
    const [result, setResult] = createSignal<string | null>(null);
    const t: MicTest = {
        meter: createMicLevelMeter(),
        state,
        setState,
        result,
        setResult,
        engine,
        deviceId,
        refreshDevices,
        stream: null,
        recorder: null,
        generation: 0,
    };
    onCleanup(() => stopMicTest(t));
    return t;
}

function stopMicTest(t: MicTest): void {
    t.meter.stop();
    if (t.recorder) {
        t.recorder.onstop = null; // don't let a deliberate stop trigger transcription
        if (t.recorder.state !== "inactive") {
            try { t.recorder.stop(); } catch { /* ignore */ }
        }
    }
    t.recorder = null;
    if (t.stream) {
        t.stream.getTracks().forEach((track) => track.stop());
        t.stream = null;
    }
}

async function runMicTest(t: MicTest): Promise<void> {
    stopMicTest(t);
    const gen = ++t.generation;
    t.setResult(null);
    t.setState("listening");
    let stream: MediaStream;
    try {
        const deviceId = t.deviceId();
        const constraint: boolean | MediaTrackConstraints =
            deviceId && deviceId !== "default" ? { deviceId: { exact: deviceId } } : true;
        stream = await navigator.mediaDevices.getUserMedia({ audio: constraint });
    } catch (e: any) {
        if (gen !== t.generation) return; // cancelled while awaiting the permission prompt
        t.setState("error");
        t.setResult(
            e?.name === "NotAllowedError" || e?.name === "SecurityError"
                ? "Microphone access blocked — check your OS privacy settings."
                : "No microphone available.",
        );
        return;
    }
    if (gen !== t.generation) {
        // Cancelled while awaiting getUserMedia — this stream was never
        // handed to stopMicTest(), so it's the only one that can leak it.
        stream.getTracks().forEach((track) => track.stop());
        return;
    }
    t.stream = stream;
    // Unlocks real device labels for the picker (getUserMedia grant makes
    // enumerateDevices() return non-empty labels from here on).
    void t.refreshDevices();
    t.meter.start(t.stream);

    // Record a short fixed-duration clip, encoded for whichever engine is
    // currently configured, then run it through the real transcribe
    // endpoint — this is the only way to genuinely validate an engine
    // end-to-end (misconfiguration is server-side-only).
    const isLocal = t.engine() === "whisper-local";
    window.setTimeout(() => recordClip(t, gen, isLocal), 1200);
}

function recordClip(t: MicTest, gen: number, isLocal: boolean): void {
    if (gen !== t.generation || !t.stream) return; // stopped/cancelled before the clip started
    if (isLocal) {
        // whisper-local expects 16kHz mono WAV; recording that path's
        // exact encoder here would duplicate whisperVoiceEngine.ts's
        // ScriptProcessor pipeline. Keep the test flow's own capture
        // simple (MediaRecorder) and let the user know local-engine
        // testing isn't wired into this quick check yet. Full
        // stopMicTest() (not just meter.stop()) so the still-open mic
        // stream from getUserMedia above is actually released —
        // Cancel isn't shown once we're in the "error" state.
        t.setState("error");
        t.setResult(
            "Quick mic test only validates capture + Groq for now. Save a whisper-cli path above, " +
            "then try recording from an actual agent pane to validate the local engine end-to-end.",
        );
        stopMicTest(t);
        return;
    }
    const mime = pickMime();
    const chunks: Blob[] = [];
    const recorder = new MediaRecorder(t.stream, { mimeType: mime });
    t.recorder = recorder;
    recorder.ondataavailable = (e) => { if (e.data.size > 0) chunks.push(e.data); };
    recorder.onstop = () => {
        if (gen !== t.generation) return; // stopMicTest() cleared onstop before calling stop(); belt-and-suspenders
        t.meter.stop();
        t.setState("transcribing");
        void transcribeMicClip(t, gen, new Blob(chunks, { type: mime }), mime);
    };
    recorder.start();
    window.setTimeout(() => {
        if (gen === t.generation && recorder.state !== "inactive") recorder.stop();
    }, 2500);
}

async function transcribeMicClip(t: MicTest, gen: number, blob: Blob, mime: string): Promise<void> {
    try {
        const base = getWebServerEndpoint();
        const url = `${base}/api/v1/voice/transcribe?mime=${encodeURIComponent(mime)}`;
        const resp = await fetch(url, {
            method: "POST",
            headers: authHeaders({ "Content-Type": mime }),
            body: blob,
        });
        if (gen !== t.generation) return; // cancelled (or superseded by a new test) while the request was in flight
        if (!resp.ok) {
            const detail = await resp.json().then((d: any) => d?.error).catch(() => null);
            if (gen !== t.generation) return;
            t.setState("error");
            t.setResult(detail ?? `Transcription failed (${resp.status}).`);
            return;
        }
        const data = (await resp.json()) as { text?: string };
        if (gen !== t.generation) return;
        t.setState("done");
        t.setResult((data.text || "").trim() || "(no speech detected)");
    } catch (e) {
        if (gen !== t.generation) return;
        t.setState("error");
        t.setResult(e instanceof Error ? e.message : "Request failed.");
    }
}

function cancelMicTest(t: MicTest): void {
    t.generation++; // invalidate any in-flight continuation before tearing down
    stopMicTest(t);
    t.setState("idle");
}

// ── Rows ──────────────────────────────────────────────────────────────────────

function WhisperRows(p: {
    s: SettingsFn;
    modelChoice: () => string;
    setExplicitCustom: Setter<boolean>;
}): JSX.Element {
    return (
        <>
            <SettingRow
                id={RECORDING_SETTINGS.whisperCliPath.id}
                label={RECORDING_SETTINGS.whisperCliPath.label}
                control={
                    <PathField
                        value={p.s()["voice:whisperCliPath"] as string | undefined}
                        onChange={(v) => set("voice:whisperCliPath", v)}
                        placeholder="/usr/local/bin/whisper-cli"
                    />
                }
            />
            <SettingRow
                id={RECORDING_SETTINGS.model.id}
                label={RECORDING_SETTINGS.model.label}
                control={
                    <Select
                        class="setting-select"
                        value={p.modelChoice()}
                        onChange={(v) => {
                            if (v === "custom") {
                                p.setExplicitCustom(true);
                            } else {
                                p.setExplicitCustom(false);
                                // A named model takes over — clear any stale explicit
                                // path so the two settings can't silently disagree
                                // about which one the backend actually uses.
                                set("voice:whisperModelPath", null);
                                set("voice:whisperModel", v);
                            }
                        }}
                    >
                        <For each={WHISPER_MODEL_CHOICES}>{(m) => <option value={m}>{m}</option>}</For>
                        <option value="custom">{t("settings.recording.whisperModel.custom")}</option>
                    </Select>
                }
                description={RECORDING_SETTINGS.model.description}
            />
            <Show when={p.modelChoice() === "custom"}>
                <SettingRow
                    id={RECORDING_SETTINGS.modelFilePath.id}
                    indent
                    label={RECORDING_SETTINGS.modelFilePath.label}
                    control={
                        <PathField
                            value={p.s()["voice:whisperModelPath"] as string | undefined}
                            onChange={(v) => set("voice:whisperModelPath", v)}
                            placeholder="/path/to/ggml-model.bin"
                        />
                    }
                />
            </Show>
        </>
    );
}

function EngineRows(p: {
    s: SettingsFn;
    engine: () => string;
    modelChoice: () => string;
    setExplicitCustom: Setter<boolean>;
}): JSX.Element {
    return (
        <>
            <SectionHeader label={t("settings.recording.group.transcriptionEngine")} />
            <SettingRow
                id={RECORDING_SETTINGS.engine.id}
                label={RECORDING_SETTINGS.engine.label}
                control={
                    <Select class="setting-select" value={p.engine()} onChange={(v) => set("voice:engine", v)}>
                        <option value="groq">{t("settings.recording.engine.groq")}</option>
                        <option value="whisper-local">{t("settings.recording.engine.whisperLocal")}</option>
                        <Show when={isDev()}>
                            <option value="webspeech">{t("settings.recording.engine.webSpeech")}</option>
                        </Show>
                    </Select>
                }
                description={RECORDING_SETTINGS.engine.description}
            />

            <Show when={p.engine() === "groq"}>
                <SettingRow
                    id={RECORDING_SETTINGS.groqApiKey.id}
                    label={RECORDING_SETTINGS.groqApiKey.label}
                    control={
                        <MaskedKeyField
                            value={p.s()["voice:groqApiKey"] as string | undefined}
                            onSave={(key) => set("voice:groqApiKey", key)}
                            placeholder={t("settings.recording.groqKey.placeholder")}
                        />
                    }
                    description={RECORDING_SETTINGS.groqApiKey.description}
                />
            </Show>

            <Show when={p.engine() === "whisper-local"}>
                <WhisperRows s={p.s} modelChoice={p.modelChoice} setExplicitCustom={p.setExplicitCustom} />
            </Show>
        </>
    );
}

function MicrophoneRows(p: { s: SettingsFn; devices: Accessor<MediaDeviceInfo[]> }): JSX.Element {
    return (
        <>
            <SectionHeader label={t("settings.recording.group.microphone")} />
            <SettingRow
                id={RECORDING_SETTINGS.inputDevice.id}
                label={RECORDING_SETTINGS.inputDevice.label}
                control={
                    <Select
                        class="setting-select"
                        value={(p.s()["voice:inputDeviceId"] as string) ?? "default"}
                        onChange={(v) => set("voice:inputDeviceId", v)}
                    >
                        <option value="default">{t("settings.recording.microphone.systemDefault")}</option>
                        <For each={p.devices()}>
                            {(d, i) => <option value={d.deviceId}>{d.label || t("settings.recording.microphone.numbered", { n: i() + 1 })}</option>}
                        </For>
                    </Select>
                }
            />
        </>
    );
}

function MicTestRows(p: { t: MicTest }): JSX.Element {
    const listeningOrTranscribing = () => p.t.state() === "listening" || p.t.state() === "transcribing";
    return (
        <>
            <SectionHeader label={t("settings.recording.group.testMicrophone")} />
            <div class="setting-mic-test">
                <Button tone="accent" disabled={listeningOrTranscribing()} onClick={() => void runMicTest(p.t)}>
                    {p.t.state() === "listening" ? t("settings.recording.test.listening") : p.t.state() === "transcribing" ? t("settings.recording.test.transcribing") : t("settings.recording.test.start")}
                </Button>
                <Show when={listeningOrTranscribing()}>
                    <Button onClick={() => cancelMicTest(p.t)}>{t("settings.recording.test.cancel")}</Button>
                </Show>
                <div class="setting-mic-test-meter">
                    <div class="setting-mic-test-meter-fill" style={{ width: `${Math.min(100, p.t.meter.level() * 220)}%` }} />
                </div>
            </div>
            <Show when={p.t.result()}>
                <div class="setting-mic-test-result" classList={{ "setting-mic-test-result--error": p.t.state() === "error" }}>
                    {p.t.result()}
                </div>
            </Show>
        </>
    );
}

export function RecordingSection(): JSX.Element {
    const s: SettingsFn = () => settingsAtom() ?? ({} as any);

    const enabled = () => (s()["voice:enabled"] as boolean) ?? true;
    const engine = () => (s()["voice:engine"] as string) ?? "groq";
    const whisperModel = () => (s()["voice:whisperModel"] as string) ?? "base.en";
    // An explicit voice:whisperModelPath always wins server-side (voice.rs),
    // so treat its presence as "custom" regardless of whisperModel's value —
    // otherwise a hand-edited settings.json with both keys set would silently
    // hide the path that's actually in effect. `explicitCustom` covers the
    // gap where the user has just clicked "custom path…" but hasn't typed a
    // path yet (whisperModelPath is still empty at that point, so the above
    // check alone wouldn't reveal the field).
    const [explicitCustom, setExplicitCustom] = createSignal(false);
    const modelChoice = () => {
        if ((s()["voice:whisperModelPath"] as string | undefined)?.trim()) return "custom";
        if (explicitCustom()) return "custom";
        return WHISPER_MODEL_CHOICES.includes(whisperModel()) ? whisperModel() : "custom";
    };

    const { devices, refresh } = createDeviceList();
    const test = createMicTest(engine, () => s()["voice:inputDeviceId"] as string | undefined, refresh);

    return (
        <div class="settings-section-body">
            <SettingRow
                id={RECORDING_SETTINGS.enableVoiceInput.id}
                label={RECORDING_SETTINGS.enableVoiceInput.label}
                description={RECORDING_SETTINGS.enableVoiceInput.description}
                control={<ToggleControl checked={enabled()} onChange={(v) => set("voice:enabled", v)} />}
            />
            <Show when={enabled()}>
                <EngineRows s={s} engine={engine} modelChoice={modelChoice} setExplicitCustom={setExplicitCustom} />
                <MicrophoneRows s={s} devices={devices} />
                <MicTestRows t={test} />
            </Show>
        </div>
    );
}
