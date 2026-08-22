// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings -> Recording — voice input is a fully-shipped feature (3 STT
 * engines, per-pane mic button) with zero prior Settings UI; every knob was
 * settings.json-only. See docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md
 * for the full design this section implements.
 */
import { createSignal, onCleanup, onMount, Show, For, type JSX } from "solid-js";

import { settingsAtom } from "@/app/store/global";
import { isDev } from "@/app/store/misc-utils";
import { getWebServerEndpoint } from "@/util/endpoints";
import { getApi } from "@/app/store/app-api";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createMicLevelMeter } from "@/app/hook/useMicLevelMeter";
import { pickMime } from "@/app/hook/whisperVoiceEngine";
import { MaskedKeyField, SectionHeader, set, SettingRow, ToggleControl } from "../settings-controls";

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
            <input
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

export function RecordingSection(): JSX.Element {
    const s = () => settingsAtom() ?? ({} as any);

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

    // ── Device picker (§5) ───────────────────────────────────────────────────
    const [devices, setDevices] = createSignal<MediaDeviceInfo[]>([]);
    const refreshDevices = async () => {
        if (typeof navigator === "undefined" || !navigator.mediaDevices?.enumerateDevices) return;
        try {
            const all = await navigator.mediaDevices.enumerateDevices();
            setDevices(all.filter((d) => d.kind === "audioinput"));
        } catch {
            setDevices([]);
        }
    };
    onMount(() => void refreshDevices());

    // ── Test your microphone (§4) ────────────────────────────────────────────
    // Explicit-click only (not an auto-probe on section mount) so opening
    // Settings never triggers a surprise OS mic-permission dialog by itself.
    const meter = createMicLevelMeter();
    const [testState, setTestState] = createSignal<"idle" | "listening" | "transcribing" | "done" | "error">("idle");
    const [testResult, setTestResult] = createSignal<string | null>(null);
    let testStream: MediaStream | null = null;
    let testRecorder: MediaRecorder | null = null;
    // Bumped by every runTest()/cancelTest() call. Async continuations
    // (setTimeout callbacks, the fetch round-trip) capture the generation
    // active when they were scheduled and check it's still current before
    // touching state — otherwise a Cancel during an in-flight test could not
    // stop it: stopping the recorder still fired its installed onstop (which
    // then proceeded to transcribe), and a stale fetch response could
    // overwrite state a new test had already moved past. See PR #2751 review.
    let testGeneration = 0;

    const stopTest = () => {
        meter.stop();
        if (testRecorder) {
            testRecorder.onstop = null; // don't let a deliberate stop trigger transcription
            if (testRecorder.state !== "inactive") {
                try { testRecorder.stop(); } catch { /* ignore */ }
            }
        }
        testRecorder = null;
        if (testStream) {
            testStream.getTracks().forEach((t) => t.stop());
            testStream = null;
        }
    };
    onCleanup(stopTest);

    const runTest = async () => {
        stopTest();
        const gen = ++testGeneration;
        setTestResult(null);
        setTestState("listening");
        let stream: MediaStream;
        try {
            const deviceId = s()["voice:inputDeviceId"] as string | undefined;
            const constraint: boolean | MediaTrackConstraints =
                deviceId && deviceId !== "default" ? { deviceId: { exact: deviceId } } : true;
            stream = await navigator.mediaDevices.getUserMedia({ audio: constraint });
        } catch (e: any) {
            if (gen !== testGeneration) return; // cancelled while awaiting the permission prompt
            setTestState("error");
            setTestResult(
                e?.name === "NotAllowedError" || e?.name === "SecurityError"
                    ? "Microphone access blocked — check your OS privacy settings."
                    : "No microphone available.",
            );
            return;
        }
        if (gen !== testGeneration) {
            // Cancelled while awaiting getUserMedia — this stream was never
            // handed to stopTest(), so it's the only one that can leak it.
            stream.getTracks().forEach((t) => t.stop());
            return;
        }
        testStream = stream;
        // Unlocks real device labels for the picker (getUserMedia grant makes
        // enumerateDevices() return non-empty labels from here on).
        void refreshDevices();
        meter.start(testStream);

        // Record a short fixed-duration clip, encoded for whichever engine is
        // currently configured, then run it through the real transcribe
        // endpoint — this is the only way to genuinely validate an engine
        // end-to-end (misconfiguration is server-side-only).
        const isLocal = engine() === "whisper-local";
        window.setTimeout(() => {
            if (gen !== testGeneration || !testStream) return; // stopped/cancelled before the clip started
            if (isLocal) {
                // whisper-local expects 16kHz mono WAV; recording that path's
                // exact encoder here would duplicate whisperVoiceEngine.ts's
                // ScriptProcessor pipeline. Keep the test flow's own capture
                // simple (MediaRecorder) and let the user know local-engine
                // testing isn't wired into this quick check yet. Full
                // stopTest() (not just meter.stop()) so the still-open mic
                // stream from getUserMedia above is actually released —
                // Cancel isn't shown once we're in the "error" state.
                setTestState("error");
                setTestResult(
                    "Quick mic test only validates capture + Groq for now. Save a whisper-cli path above, " +
                    "then try recording from an actual agent pane to validate the local engine end-to-end.",
                );
                stopTest();
                return;
            }
            const mime = pickMime();
            const chunks: Blob[] = [];
            const recorder = new MediaRecorder(testStream, { mimeType: mime });
            testRecorder = recorder;
            recorder.ondataavailable = (e) => { if (e.data.size > 0) chunks.push(e.data); };
            recorder.onstop = () => {
                if (gen !== testGeneration) return; // stopTest() cleared onstop before calling stop(); belt-and-suspenders
                meter.stop();
                setTestState("transcribing");
                void transcribeTestClip(gen, new Blob(chunks, { type: mime }), mime);
            };
            recorder.start();
            window.setTimeout(() => {
                if (gen === testGeneration && recorder.state !== "inactive") recorder.stop();
            }, 2500);
        }, 1200);
    };

    const transcribeTestClip = async (gen: number, blob: Blob, mime: string) => {
        try {
            const base = getWebServerEndpoint();
            const url = `${base}/api/v1/voice/transcribe?mime=${encodeURIComponent(mime)}`;
            const resp = await fetch(url, {
                method: "POST",
                headers: { "X-AuthKey": getApi()?.getAuthKey?.() ?? "", "Content-Type": mime },
                body: blob,
            });
            if (gen !== testGeneration) return; // cancelled (or superseded by a new test) while the request was in flight
            if (!resp.ok) {
                const detail = await resp.json().then((d: any) => d?.error).catch(() => null);
                if (gen !== testGeneration) return;
                setTestState("error");
                setTestResult(detail ?? `Transcription failed (${resp.status}).`);
                return;
            }
            const data = (await resp.json()) as { text?: string };
            if (gen !== testGeneration) return;
            setTestState("done");
            setTestResult((data.text || "").trim() || "(no speech detected)");
        } catch (e) {
            if (gen !== testGeneration) return;
            setTestState("error");
            setTestResult(e instanceof Error ? e.message : "Request failed.");
        }
    };

    const cancelTest = () => {
        testGeneration++; // invalidate any in-flight continuation before tearing down
        stopTest();
        setTestState("idle");
    };

    return (
        <div class="settings-section-body">
            <SettingRow
                label="Enable voice input"
                description="Show the microphone button on agent and terminal panes"
                control={<ToggleControl checked={enabled()} onChange={(v) => set("voice:enabled", v)} />}
            />
            <Show when={enabled()}>
                <SectionHeader label="Transcription engine" />
                <SettingRow
                    label="Engine"
                    control={
                        <select class="setting-select" value={engine()} onChange={(e) => set("voice:engine", e.currentTarget.value)}>
                            <option value="groq">Groq (cloud)</option>
                            <option value="whisper-local">whisper.cpp (local, offline)</option>
                            <Show when={isDev()}>
                                <option value="webspeech">Web Speech (dev only)</option>
                            </Show>
                        </select>
                    }
                    description="whisper.cpp runs fully offline; Groq sends audio to Groq's API"
                />

                <Show when={engine() === "groq"}>
                    <SettingRow
                        label="Groq API key"
                        control={
                            <MaskedKeyField
                                value={s()["voice:groqApiKey"] as string | undefined}
                                onSave={(key) => set("voice:groqApiKey", key)}
                                placeholder="paste key — never displayed again after saving"
                            />
                        }
                        description="Sent once, over HTTPS, from the AgentMux backend on this machine directly to api.groq.com — never to any other AgentMux service."
                    />
                </Show>

                <Show when={engine() === "whisper-local"}>
                    <SettingRow
                        label="whisper-cli path"
                        control={
                            <PathField
                                value={s()["voice:whisperCliPath"] as string | undefined}
                                onChange={(v) => set("voice:whisperCliPath", v)}
                                placeholder="/usr/local/bin/whisper-cli"
                            />
                        }
                    />
                    <SettingRow
                        label="Model"
                        control={
                            <select
                                class="setting-select"
                                value={modelChoice()}
                                onChange={(e) => {
                                    const v = e.currentTarget.value;
                                    if (v === "custom") {
                                        setExplicitCustom(true);
                                    } else {
                                        setExplicitCustom(false);
                                        // A named model takes over — clear any stale explicit
                                        // path so the two settings can't silently disagree
                                        // about which one the backend actually uses.
                                        set("voice:whisperModelPath", null);
                                        set("voice:whisperModel", v);
                                    }
                                }}
                            >
                                <For each={WHISPER_MODEL_CHOICES}>{(m) => <option value={m}>{m}</option>}</For>
                                <option value="custom">custom path…</option>
                            </select>
                        }
                        description="Auto-downloaded on first use. Only one of Model or Model file path applies at a time — file path takes precedence if both are set."
                    />
                    <Show when={modelChoice() === "custom"}>
                        <SettingRow
                            indent
                            label="Model file path"
                            control={
                                <PathField
                                    value={s()["voice:whisperModelPath"] as string | undefined}
                                    onChange={(v) => set("voice:whisperModelPath", v)}
                                    placeholder="/path/to/ggml-model.bin"
                                />
                            }
                        />
                    </Show>
                </Show>

                <SectionHeader label="Microphone" />
                <SettingRow
                    label="Input device"
                    control={
                        <select
                            class="setting-select"
                            value={(s()["voice:inputDeviceId"] as string) ?? "default"}
                            onChange={(e) => set("voice:inputDeviceId", e.currentTarget.value)}
                        >
                            <option value="default">System default</option>
                            <For each={devices()}>
                                {(d, i) => <option value={d.deviceId}>{d.label || `Microphone ${i() + 1}`}</option>}
                            </For>
                        </select>
                    }
                />

                <SectionHeader label="Test your microphone" />
                <div class="setting-mic-test">
                    <button
                        type="button"
                        class="setting-masked-key-btn setting-masked-key-btn--primary"
                        disabled={testState() === "listening" || testState() === "transcribing"}
                        onClick={() => void runTest()}
                    >
                        {testState() === "listening" ? "Listening…" : testState() === "transcribing" ? "Transcribing…" : "Start test"}
                    </button>
                    <Show when={testState() === "listening" || testState() === "transcribing"}>
                        <button type="button" class="setting-masked-key-btn" onClick={cancelTest}>Cancel</button>
                    </Show>
                    <div class="setting-mic-test-meter">
                        <div class="setting-mic-test-meter-fill" style={{ width: `${Math.min(100, meter.level() * 220)}%` }} />
                    </div>
                </div>
                <Show when={testResult()}>
                    <div class="setting-mic-test-result" classList={{ "setting-mic-test-result--error": testState() === "error" }}>
                        {testResult()}
                    </div>
                </Show>
            </Show>
        </div>
    );
}
