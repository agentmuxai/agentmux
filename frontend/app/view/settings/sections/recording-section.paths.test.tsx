// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Characterisation tests for RecordingSection: which rows show for which
 * engine, how the model picker and its custom path interact, the microphone
 * picker, and the "Test your microphone" flow (capture, transcribe, cancel,
 * stale continuations).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    setConfig: vi.fn(),
    checkPath: vi.fn(),
    dev: { on: false },
    meterStart: vi.fn(),
    meterStop: vi.fn(),
    level: { v: 0 },
    pickMime: vi.fn(() => "audio/webm"),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetConfigCommand: (...a: unknown[]) => h.setConfig(...a),
        CheckPathCommand: (...a: unknown[]) => h.checkPath(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", async () => {
    const { createSignal } = await import("solid-js");
    const [get, setGet] = createSignal<Record<string, unknown>>({});
    return { settingsAtom: () => get(), __setSettings: (v: Record<string, unknown>) => setGet(v) };
});
vi.mock("@/app/store/misc-utils", () => ({ isDev: () => h.dev.on }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv.test" }));
vi.mock("@/app/store/auth-headers", () => ({
    authHeaders: (extra: Record<string, string>) => ({ ...extra, Authorization: "Bearer k" }),
}));
vi.mock("@/app/hook/whisperVoiceEngine", () => ({ pickMime: () => h.pickMime() }));
vi.mock("@/app/hook/useMicLevelMeter", () => ({
    createMicLevelMeter: () => ({
        level: () => h.level.v,
        start: (s: MediaStream) => h.meterStart(s),
        stop: () => h.meterStop(),
    }),
}));

import * as globalMod from "@/app/store/global";
import { RECORDING_SETTINGS, RecordingSection } from "./recording-section";

const setSettings = (globalMod as unknown as { __setSettings: (v: Record<string, unknown>) => void }).__setSettings;

// ── Fakes ────────────────────────────────────────────────────────────────

class FakeRecorder {
    static instances: FakeRecorder[] = [];
    static sticky = false;
    state: "inactive" | "recording" = "inactive";
    ondataavailable: ((e: { data: Blob }) => void) | null = null;
    onstop: (() => void) | null = null;
    started = 0;
    stopped = 0;
    stopThrows = false;
    constructor(
        public stream: MediaStream,
        public opts: { mimeType: string }
    ) {
        FakeRecorder.instances.push(this);
    }
    start() {
        this.started++;
        this.state = "recording";
    }
    stop() {
        this.stopped++;
        if (this.stopThrows) throw new Error("boom");
        // A real recorder goes inactive asynchronously; sticky keeps it "recording" to expose a second stop().
        if (!FakeRecorder.sticky) this.state = "inactive";
        this.onstop?.();
    }
}

function fakeStream() {
    const tracks = [{ stop: vi.fn() }, { stop: vi.fn() }];
    return { tracks, stream: { getTracks: () => tracks } as unknown as MediaStream };
}

const getUserMedia = vi.fn();
const enumerateDevices = vi.fn();
const fetchMock = vi.fn();

function mic(deviceId: string, label: string, kind = "audioinput") {
    return { deviceId, label, kind } as MediaDeviceInfo;
}

const flush = async (ms = 0) => {
    await vi.advanceTimersByTimeAsync(ms);
};

function row(label: string): HTMLElement {
    const r = screen.getByText(label).closest(".setting-row");
    expect(r).not.toBeNull();
    return r as HTMLElement;
}
const hasRow = (label: string) => screen.queryByText(label) !== null;
const select = (label: string) => row(label).querySelector("select") as HTMLSelectElement;
const optionTexts = (sel: HTMLSelectElement) => Array.from(sel.options).map((o) => o.textContent);
const optionValues = (sel: HTMLSelectElement) => Array.from(sel.options).map((o) => o.value);
const lastConfig = () => h.setConfig.mock.calls[h.setConfig.mock.calls.length - 1][1];
const startBtn = () => screen.getByRole("button", { name: /Start test|Listening…|Transcribing…/ }) as HTMLButtonElement;
const cancelBtn = () => screen.queryByRole("button", { name: "Cancel" });
const result = () => document.querySelector(".setting-mic-test-result") as HTMLElement | null;

function change(sel: HTMLSelectElement, value: string) {
    sel.value = value;
    fireEvent.change(sel);
}

beforeEach(() => {
    vi.useFakeTimers();
    h.setConfig.mockReset();
    h.checkPath.mockReset();
    h.checkPath.mockResolvedValue({ exists: true });
    h.dev.on = false;
    h.meterStart.mockReset();
    h.meterStop.mockReset();
    h.level.v = 0;
    h.pickMime.mockClear();
    FakeRecorder.instances = [];
    FakeRecorder.sticky = false;
    getUserMedia.mockReset();
    enumerateDevices.mockReset();
    enumerateDevices.mockResolvedValue([]);
    fetchMock.mockReset();
    setSettings({});
    vi.stubGlobal("MediaRecorder", FakeRecorder);
    vi.stubGlobal("fetch", fetchMock);
    Object.defineProperty(navigator, "mediaDevices", {
        configurable: true,
        value: { getUserMedia, enumerateDevices },
    });
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
});

async function renderSection(settings: Record<string, unknown> = {}) {
    setSettings(settings);
    render(() => <RecordingSection />);
    await flush();
}

/**
 * What a picker shows for `target`. A Select given <option> children shows
 * its first option on first render whatever its value is (the value is set
 * before the options exist), so the only way to see the value the section
 * computed is to render with `sentinel` first and then change to `target`.
 */
async function shownAfter(label: string, sentinel: Record<string, unknown>, target: Record<string, unknown>) {
    await renderSection(sentinel);
    setSettings(target);
    await flush();
    const value = select(label).value;
    cleanup();
    return value;
}
const LOCAL = { "voice:engine": "whisper-local" };

// ── Layout ───────────────────────────────────────────────────────────────

describe("RecordingSection layout", () => {
    it("shows every group when voice input is on by default, with Groq as the default engine", async () => {
        await renderSection();
        expect(hasRow("Enable voice input")).toBe(true);
        expect(hasRow("Transcription engine")).toBe(true);
        expect(hasRow("Groq API key")).toBe(true);
        expect(hasRow("whisper-cli path")).toBe(false);
        expect(hasRow("Model")).toBe(false);
        expect(hasRow("Microphone")).toBe(true);
        expect(hasRow("Input device")).toBe(true);
        expect(hasRow("Test your microphone")).toBe(true);
    });

    it("hides everything below the toggle when voice input is off", async () => {
        await renderSection({ "voice:enabled": false });
        expect(hasRow("Enable voice input")).toBe(true);
        expect(hasRow("Engine")).toBe(false);
        expect(hasRow("Input device")).toBe(false);
        expect(hasRow("Test your microphone")).toBe(false);
        expect(screen.queryByRole("button", { name: "Start test" })).toBeNull();
    });

    it("shows the toggle on by default and writes voice:enabled when it is flipped", async () => {
        await renderSection();
        const sw = row("Enable voice input").querySelector('[role="switch"]') as HTMLElement;
        expect(sw.getAttribute("aria-checked")).toBe("true");
        fireEvent.click(sw);
        expect(lastConfig()).toEqual({ "voice:enabled": false });
    });

    it("shows the toggle off when voice:enabled is false", async () => {
        await renderSection({ "voice:enabled": false });
        const sw = row("Enable voice input").querySelector('[role="switch"]') as HTMLElement;
        expect(sw.getAttribute("aria-checked")).toBe("false");
    });

    // The ids are written out here, not read from RECORDING_SETTINGS, so a
    // renamed id fails: the search index and the rendered rows must agree.
    it("gives each row the setting-<id> anchor the search scrolls to", async () => {
        const local = [
            "recording.enable_voice_input",
            "recording.engine",
            "recording.whisper_cli_path",
            "recording.model",
            "recording.model_file_path",
            "recording.input_device",
        ];
        await renderSection({ "voice:engine": "whisper-local", "voice:whisperModelPath": "/m.bin" });
        for (const id of local) expect(document.getElementById(`setting-${id}`), id).not.toBeNull();
        expect(document.querySelectorAll('[id^="setting-recording."]')).toHaveLength(local.length);
        cleanup();
        await renderSection();
        expect(document.getElementById("setting-recording.groq_api_key")).not.toBeNull();
        expect(
            Object.values(RECORDING_SETTINGS)
                .map((e) => e.id)
                .sort()
        ).toEqual([...local, "recording.groq_api_key"].sort());
    });

    it("shows the description under the rows that have one", async () => {
        await renderSection({ "voice:engine": "whisper-local", "voice:whisperModelPath": "/m.bin" });
        expect(row("Enable voice input").textContent).toContain(
            "Show the microphone button on agent and terminal panes"
        );
        expect(row("Engine").textContent).toContain("whisper.cpp runs fully offline; Groq sends audio to Groq's API");
        expect(row("Model").textContent).toContain(
            "Auto-downloaded on first use. Only one of Model or Model file path applies at a time"
        );
    });

    it("shows the Groq key description on the Groq row", async () => {
        await renderSection();
        expect(row("Groq API key").textContent).toContain(
            "directly to api.groq.com — never to any other AgentMux service."
        );
    });

    it("renders with defaults while the settings have not loaded", async () => {
        setSettings(undefined as unknown as Record<string, unknown>);
        render(() => <RecordingSection />);
        await flush();
        expect(hasRow("Groq API key")).toBe(true);
        expect(startBtn().textContent).toBe("Start test");
    });
});

// ── Engine picker ────────────────────────────────────────────────────────

describe("RecordingSection engine picker", () => {
    it("offers Groq and whisper.cpp, and Web Speech only in dev builds", async () => {
        await renderSection();
        expect(optionValues(select("Engine"))).toEqual(["groq", "whisper-local"]);
        cleanup();
        h.dev.on = true;
        await renderSection();
        expect(optionValues(select("Engine"))).toEqual(["groq", "whisper-local", "webspeech"]);
        expect(optionTexts(select("Engine"))[2]).toBe("Web Speech (dev only)");
    });

    it("labels the two engines for people", async () => {
        await renderSection();
        expect(optionTexts(select("Engine"))).toEqual(["Groq (cloud)", "whisper.cpp (local, offline)"]);
    });

    it("writes voice:engine when the engine changes", async () => {
        await renderSection();
        change(select("Engine"), "whisper-local");
        expect(lastConfig()).toEqual({ "voice:engine": "whisper-local" });
    });

    it("shows the engine the settings name, and Groq when none is stored", async () => {
        expect(await shownAfter("Engine", { "voice:engine": "x" }, LOCAL)).toBe("whisper-local");
        expect(await shownAfter("Engine", { "voice:engine": "x" }, {})).toBe("groq");
    });

    it("shows the key row only for Groq, and the whisper rows only for whisper-local", async () => {
        await renderSection({ "voice:engine": "whisper-local" });
        expect(hasRow("Groq API key")).toBe(false);
        expect(hasRow("whisper-cli path")).toBe(true);
        expect(hasRow("Model")).toBe(true);
        cleanup();
        await renderSection({ "voice:engine": "webspeech" });
        expect(hasRow("Groq API key")).toBe(false);
        expect(hasRow("whisper-cli path")).toBe(false);
        expect(hasRow("Model")).toBe(false);
    });

    it("saves the Groq key through MaskedKeyField into voice:groqApiKey", async () => {
        await renderSection();
        const input = row("Groq API key").querySelector("input") as HTMLInputElement;
        expect(input.placeholder).toBe("paste key — never displayed again after saving");
        fireEvent.input(input, { target: { value: "  gsk_abc " } });
        fireEvent.click(screen.getByRole("button", { name: "Save" }));
        expect(lastConfig()).toEqual({ "voice:groqApiKey": "gsk_abc" });
    });

    it("shows a stored Groq key masked, never its value", async () => {
        await renderSection({ "voice:groqApiKey": "gsk_secret" });
        const r = row("Groq API key");
        expect(r.textContent).toContain("••••••••");
        expect(r.textContent).not.toContain("gsk_secret");
        expect(r.querySelector("input")).toBeNull();
    });
});

// ── whisper-local rows ───────────────────────────────────────────────────

describe("RecordingSection whisper-local model picker", () => {
    const local = { "voice:engine": "whisper-local" };

    it("lists the three named models and a custom path option", async () => {
        await renderSection(local);
        expect(optionValues(select("Model"))).toEqual(["base.en", "small.en", "medium.en", "custom"]);
        expect(optionTexts(select("Model"))[3]).toBe("custom path…");
    });

    const sentinel = { ...local, "voice:whisperModel": "medium.en" };

    it("selects base.en when nothing is set, and the stored named model when it is one", async () => {
        expect(await shownAfter("Model", sentinel, local)).toBe("base.en");
        expect(await shownAfter("Model", local, { ...local, "voice:whisperModel": "medium.en" })).toBe("medium.en");
        expect(await shownAfter("Model", sentinel, { ...local, "voice:whisperModel": "small.en" })).toBe("small.en");
    });

    it("treats an unknown stored model name as custom and shows the path row", async () => {
        const tiny = { ...local, "voice:whisperModel": "tiny" };
        expect(await shownAfter("Model", sentinel, tiny)).toBe("custom");
        await renderSection(tiny);
        expect(hasRow("Model file path")).toBe(true);
    });

    it("treats an explicit model path as custom even when a named model is set too", async () => {
        const both = { ...local, "voice:whisperModel": "small.en", "voice:whisperModelPath": "/m/ggml.bin" };
        expect(await shownAfter("Model", sentinel, both)).toBe("custom");
        await renderSection(both);
        expect(hasRow("Model file path")).toBe(true);
    });

    it("ignores a whitespace-only model path", async () => {
        const blank = { ...local, "voice:whisperModel": "small.en", "voice:whisperModelPath": "   " };
        expect(await shownAfter("Model", sentinel, blank)).toBe("small.en");
        await renderSection(blank);
        expect(hasRow("Model file path")).toBe(false);
    });

    it("hides the path row for a named model", async () => {
        await renderSection(local);
        expect(hasRow("Model file path")).toBe(false);
    });

    it("picking custom reveals the path row without writing anything", async () => {
        await renderSection(local);
        change(select("Model"), "custom");
        expect(h.setConfig).not.toHaveBeenCalled();
        expect(hasRow("Model file path")).toBe(true);
        expect(select("Model").value).toBe("custom");
    });

    it("picking a named model clears the stored path first, then writes the model", async () => {
        await renderSection({ ...local, "voice:whisperModelPath": "/m.bin" });
        change(select("Model"), "small.en");
        expect(h.setConfig.mock.calls.map((c) => c[1])).toEqual([
            { "voice:whisperModelPath": null },
            { "voice:whisperModel": "small.en" },
        ]);
    });

    it("picking a named model after custom hides the path row again", async () => {
        await renderSection(local);
        change(select("Model"), "custom");
        expect(hasRow("Model file path")).toBe(true);
        change(select("Model"), "medium.en");
        // The settings atom is mocked static, so the clear does not come back;
        // the row hides because explicitCustom was reset.
        expect(hasRow("Model file path")).toBe(false);
    });
});

// ── PathField ────────────────────────────────────────────────────────────

describe("RecordingSection path fields", () => {
    const local = { "voice:engine": "whisper-local", "voice:whisperCliPath": "/bin/whisper-cli" };
    const cliInput = () => row("whisper-cli path").querySelector("input") as HTMLInputElement;
    const cliStatus = () => row("whisper-cli path").querySelector(".setting-path-status") as HTMLElement;

    it("shows the stored value and the placeholder", async () => {
        await renderSection(local);
        expect(cliInput().value).toBe("/bin/whisper-cli");
        expect(cliInput().placeholder).toBe("/usr/local/bin/whisper-cli");
    });

    it("checks the stored path on mount and shows the found tick", async () => {
        await renderSection(local);
        expect(h.checkPath).toHaveBeenCalledWith({}, { path: "/bin/whisper-cli" });
        expect(cliStatus().querySelector(".fa-check")).not.toBeNull();
        expect(cliStatus().classList.contains("setting-path-status--found")).toBe(true);
        expect(cliStatus().classList.contains("setting-path-status--not-found")).toBe(false);
    });

    it("trims the path before checking it", async () => {
        await renderSection({ ...local, "voice:whisperCliPath": "  /bin/x  " });
        expect(h.checkPath).toHaveBeenCalledWith({}, { path: "/bin/x" });
    });

    it("shows the cross when the path does not exist", async () => {
        h.checkPath.mockResolvedValue({ exists: false });
        await renderSection(local);
        expect(cliStatus().querySelector(".fa-xmark")).not.toBeNull();
        expect(cliStatus().querySelector(".fa-check")).toBeNull();
        expect(cliStatus().classList.contains("setting-path-status--not-found")).toBe(true);
        expect(cliStatus().classList.contains("setting-path-status--found")).toBe(false);
    });

    it("shows a spinner while the check is in flight", async () => {
        let resolve!: (v: { exists: boolean }) => void;
        h.checkPath.mockReturnValue(new Promise((r) => (resolve = r)));
        await renderSection(local);
        expect(cliStatus().querySelector(".fa-spinner")).not.toBeNull();
        expect(cliStatus().querySelector(".fa-check")).toBeNull();
        resolve({ exists: true });
        await flush();
        expect(cliStatus().querySelector(".fa-spinner")).toBeNull();
        expect(cliStatus().querySelector(".fa-check")).not.toBeNull();
    });

    it("shows no status when the check fails", async () => {
        h.checkPath.mockRejectedValue(new Error("rpc down"));
        await renderSection(local);
        expect(cliStatus().querySelector("i")).toBeNull();
    });

    it("does not check an empty or unset path", async () => {
        await renderSection({ "voice:engine": "whisper-local" });
        expect(h.checkPath).not.toHaveBeenCalled();
        expect(cliStatus().querySelector("i")).toBeNull();
        cleanup();
        await renderSection({ "voice:engine": "whisper-local", "voice:whisperCliPath": "   " });
        expect(h.checkPath).not.toHaveBeenCalled();
    });

    it("rechecks 400 ms after the last keystroke, not before, and only once", async () => {
        await renderSection(local);
        h.checkPath.mockClear();
        fireEvent.input(cliInput(), { target: { value: "/a" } });
        await flush(399);
        expect(h.checkPath).not.toHaveBeenCalled();
        fireEvent.input(cliInput(), { target: { value: "/ab" } });
        await flush(399);
        expect(h.checkPath).not.toHaveBeenCalled();
        await flush(1);
        expect(h.checkPath).toHaveBeenCalledTimes(1);
        expect(h.checkPath).toHaveBeenCalledWith({}, { path: "/ab" });
    });

    it("clears the status when the typed path becomes empty", async () => {
        await renderSection(local);
        fireEvent.input(cliInput(), { target: { value: "  " } });
        await flush(400);
        expect(cliStatus().querySelector("i")).toBeNull();
    });

    it("saves on blur with the raw text, not on every keystroke", async () => {
        await renderSection(local);
        fireEvent.input(cliInput(), { target: { value: "/new/path" } });
        expect(h.setConfig).not.toHaveBeenCalled();
        fireEvent.blur(cliInput(), { target: { value: "/new/path" } });
        expect(lastConfig()).toEqual({ "voice:whisperCliPath": "/new/path" });
    });

    it("drops a pending recheck when the section goes away", async () => {
        await renderSection(local);
        h.checkPath.mockClear();
        fireEvent.input(cliInput(), { target: { value: "/zzz" } });
        cleanup();
        await flush(1000);
        expect(h.checkPath).not.toHaveBeenCalled();
    });

    it("uses the model-file placeholder and key on the custom path row", async () => {
        await renderSection({ ...local, "voice:whisperModelPath": "/m.bin" });
        const input = row("Model file path").querySelector("input") as HTMLInputElement;
        expect(input.value).toBe("/m.bin");
        expect(input.placeholder).toBe("/path/to/ggml-model.bin");
        fireEvent.blur(input, { target: { value: "/m2.bin" } });
        expect(lastConfig()).toEqual({ "voice:whisperModelPath": "/m2.bin" });
        expect(row("Model file path").classList.contains("setting-row--indent")).toBe(true);
    });
});

// ── Device picker ────────────────────────────────────────────────────────

describe("RecordingSection microphone picker", () => {
    const sel = () => select("Input device");

    it("offers the system default first, then every audio input", async () => {
        enumerateDevices.mockResolvedValue([
            mic("a", "USB Mic"),
            mic("v", "Camera", "videoinput"),
            mic("b", "Built-in"),
        ]);
        await renderSection();
        expect(optionValues(sel())).toEqual(["default", "a", "b"]);
        expect(optionTexts(sel())).toEqual(["System default", "USB Mic", "Built-in"]);
    });

    it("numbers devices with no label, from 1", async () => {
        enumerateDevices.mockResolvedValue([mic("a", ""), mic("b", "")]);
        await renderSection();
        expect(optionTexts(sel())).toEqual(["System default", "Microphone 1", "Microphone 2"]);
    });

    it("selects default when no device is stored, and the stored id otherwise", async () => {
        enumerateDevices.mockResolvedValue([mic("a", "USB")]);
        const away = { "voice:inputDeviceId": "zzz" };
        expect(await shownAfter("Input device", away, {})).toBe("default");
        expect(await shownAfter("Input device", away, { "voice:inputDeviceId": "a" })).toBe("a");
    });

    it("writes voice:inputDeviceId when a device is picked", async () => {
        enumerateDevices.mockResolvedValue([mic("a", "USB")]);
        await renderSection();
        change(sel(), "a");
        expect(lastConfig()).toEqual({ "voice:inputDeviceId": "a" });
    });

    it("keeps only the default when enumeration fails", async () => {
        enumerateDevices.mockRejectedValue(new Error("denied"));
        await renderSection();
        expect(optionValues(sel())).toEqual(["default"]);
    });

    it("keeps only the default when the browser has no enumerateDevices", async () => {
        Object.defineProperty(navigator, "mediaDevices", { configurable: true, value: {} });
        await renderSection();
        expect(optionValues(sel())).toEqual(["default"]);
        cleanup();
        Object.defineProperty(navigator, "mediaDevices", { configurable: true, value: undefined });
        await renderSection();
        expect(optionValues(sel())).toEqual(["default"]);
    });

    it("empties the list when the refresh after the microphone grant fails", async () => {
        enumerateDevices.mockResolvedValueOnce([mic("a", "USB Mic")]);
        getUserMedia.mockResolvedValue(fakeStream().stream);
        await renderSection();
        expect(optionValues(sel())).toEqual(["default", "a"]);
        enumerateDevices.mockRejectedValueOnce(new Error("revoked"));
        fireEvent.click(startBtn());
        await flush();
        expect(optionValues(sel())).toEqual(["default"]);
    });

    it("lists devices once on mount, without asking for the microphone", async () => {
        await renderSection();
        expect(enumerateDevices).toHaveBeenCalledTimes(1);
        expect(getUserMedia).not.toHaveBeenCalled();
    });
});

// ── Test your microphone ─────────────────────────────────────────────────

describe("RecordingSection microphone test", () => {
    let st: ReturnType<typeof fakeStream>;
    beforeEach(() => {
        st = fakeStream();
        getUserMedia.mockResolvedValue(st.stream);
    });

    async function start(settings: Record<string, unknown> = {}) {
        await renderSection(settings);
        fireEvent.click(startBtn());
        await flush();
    }

    it("starts idle: a Start test button, no Cancel, no result, and an empty meter", async () => {
        await renderSection();
        expect(startBtn().textContent).toBe("Start test");
        expect(startBtn().disabled).toBe(false);
        expect(cancelBtn()).toBeNull();
        expect(result()).toBeNull();
        expect(getUserMedia).not.toHaveBeenCalled();
        const fill = document.querySelector(".setting-mic-test-meter-fill") as HTMLElement;
        expect(fill.style.width).toBe("0%");
    });

    it("scales the meter by 220 and caps it at 100%", async () => {
        h.level.v = 0.25;
        await renderSection();
        const fill = document.querySelector(".setting-mic-test-meter-fill") as HTMLElement;
        expect(fill.style.width).toBe("55%");
        cleanup();
        h.level.v = 0.9;
        await renderSection();
        expect((document.querySelector(".setting-mic-test-meter-fill") as HTMLElement).style.width).toBe("100%");
    });

    it("asks for the default microphone with audio: true, and feeds the meter", async () => {
        await start();
        expect(getUserMedia).toHaveBeenCalledWith({ audio: true });
        expect(h.meterStart).toHaveBeenCalledWith(st.stream);
        expect(startBtn().textContent).toBe("Listening…");
        expect(startBtn().disabled).toBe(true);
        expect(cancelBtn()).not.toBeNull();
    });

    it("asks for the picked device exactly, but not for the literal 'default'", async () => {
        await start({ "voice:inputDeviceId": "dev-7" });
        expect(getUserMedia).toHaveBeenCalledWith({ audio: { deviceId: { exact: "dev-7" } } });
        cleanup();
        getUserMedia.mockClear();
        await start({ "voice:inputDeviceId": "default" });
        expect(getUserMedia).toHaveBeenCalledWith({ audio: true });
    });

    it("refreshes the device list once the microphone is granted", async () => {
        await renderSection();
        expect(enumerateDevices).toHaveBeenCalledTimes(1);
        fireEvent.click(startBtn());
        await flush();
        expect(enumerateDevices).toHaveBeenCalledTimes(2);
    });

    it("clears an earlier result when a new test starts", async () => {
        getUserMedia.mockRejectedValueOnce(Object.assign(new Error("x"), { name: "NotAllowedError" }));
        await start();
        expect(result()).not.toBeNull();
        fireEvent.click(startBtn());
        await flush(1);
        expect(result()).toBeNull();
    });

    describe("permission errors", () => {
        it.each(["NotAllowedError", "SecurityError"])("says access is blocked for %s", async (name) => {
            getUserMedia.mockRejectedValue(Object.assign(new Error("x"), { name }));
            await start();
            expect(result()!.textContent).toBe("Microphone access blocked — check your OS privacy settings.");
            expect(result()!.classList.contains("setting-mic-test-result--error")).toBe(true);
            expect(startBtn().textContent).toBe("Start test");
            expect(cancelBtn()).toBeNull();
        });

        it("says no microphone for any other failure", async () => {
            getUserMedia.mockRejectedValue(Object.assign(new Error("x"), { name: "NotFoundError" }));
            await start();
            expect(result()!.textContent).toBe("No microphone available.");
            cleanup();
            getUserMedia.mockRejectedValue(undefined);
            await start();
            expect(result()!.textContent).toBe("No microphone available.");
        });

        it("ignores a refusal that arrives after Cancel", async () => {
            let reject!: (e: unknown) => void;
            getUserMedia.mockReturnValue(new Promise((_, r) => (reject = r)));
            await start();
            fireEvent.click(cancelBtn()!);
            reject(Object.assign(new Error("x"), { name: "NotAllowedError" }));
            await flush();
            expect(result()).toBeNull();
            expect(startBtn().textContent).toBe("Start test");
        });
    });

    describe("cancelled while the permission prompt is open", () => {
        it("stops the late stream's tracks and never starts the meter or a recorder", async () => {
            let grant!: (s: MediaStream) => void;
            getUserMedia.mockReturnValue(new Promise((r) => (grant = r)));
            await start();
            fireEvent.click(cancelBtn()!);
            grant(st.stream);
            await flush(5000);
            expect(st.tracks[0].stop).toHaveBeenCalledTimes(1);
            expect(st.tracks[1].stop).toHaveBeenCalledTimes(1);
            expect(h.meterStart).not.toHaveBeenCalled();
            expect(FakeRecorder.instances).toHaveLength(0);
            expect(startBtn().textContent).toBe("Start test");
        });

        it("is also superseded by a second Start", async () => {
            let grant!: (s: MediaStream) => void;
            getUserMedia.mockReturnValueOnce(new Promise((r) => (grant = r)));
            const second = fakeStream();
            getUserMedia.mockResolvedValueOnce(second.stream);
            await renderSection();
            fireEvent.click(startBtn());
            await flush();
            // Start is disabled while listening, so supersede through the section's own Cancel then Start.
            fireEvent.click(cancelBtn()!);
            fireEvent.click(startBtn());
            await flush();
            grant(st.stream);
            await flush();
            expect(st.tracks[0].stop).toHaveBeenCalled();
            expect(second.tracks[0].stop).not.toHaveBeenCalled();
            expect(h.meterStart).toHaveBeenCalledTimes(1);
            expect(h.meterStart).toHaveBeenCalledWith(second.stream);
        });
    });

    describe("recording the clip", () => {
        it("waits 1200 ms, then records with the picked mime type, and stops after 2500 ms more", async () => {
            h.pickMime.mockReturnValue("audio/ogg");
            await start();
            await flush(1199);
            expect(FakeRecorder.instances).toHaveLength(0);
            await flush(1);
            const rec = FakeRecorder.instances[0];
            expect(rec.stream).toBe(st.stream);
            expect(rec.opts).toEqual({ mimeType: "audio/ogg" });
            expect(rec.started).toBe(1);
            await flush(2499);
            expect(rec.stopped).toBe(0);
            await flush(1);
            expect(rec.stopped).toBe(1);
        });

        it("moves to Transcribing and stops the meter when the clip ends", async () => {
            fetchMock.mockReturnValue(new Promise(() => {}));
            await start();
            await flush(1200);
            h.meterStop.mockClear();
            await flush(2500);
            expect(h.meterStop).toHaveBeenCalled();
            expect(startBtn().textContent).toBe("Transcribing…");
            expect(startBtn().disabled).toBe(true);
            expect(cancelBtn()).not.toBeNull();
        });

        it("does not stop a recorder that already stopped itself", async () => {
            fetchMock.mockReturnValue(new Promise(() => {}));
            await start();
            await flush(1200);
            const rec = FakeRecorder.instances[0];
            rec.stop(); // onstop runs, state becomes inactive
            expect(rec.stopped).toBe(1);
            await flush(2500);
            expect(rec.stopped).toBe(1);
        });

        it("keeps only non-empty chunks and posts them as one blob of the mime type", async () => {
            h.pickMime.mockReturnValue("audio/webm;codecs=opus");
            let body: Blob | undefined;
            fetchMock.mockImplementation(async (_u: string, init: RequestInit) => {
                body = init.body as Blob;
                return { ok: true, json: async () => ({ text: "hi" }) };
            });
            await start();
            await flush(1200);
            const rec = FakeRecorder.instances[0];
            rec.ondataavailable!({ data: new Blob(["abc"]) });
            rec.ondataavailable!({ data: new Blob([]) });
            rec.ondataavailable!({ data: new Blob(["de"]) });
            await flush(2500);
            expect(body!.size).toBe(5);
            expect(body!.type).toBe("audio/webm;codecs=opus");
        });

        it("tells the user the local engine is not covered, and releases the microphone", async () => {
            await start({ "voice:engine": "whisper-local" });
            await flush(1200);
            expect(FakeRecorder.instances).toHaveLength(0);
            expect(result()!.textContent).toContain("Quick mic test only validates capture + Groq for now.");
            expect(result()!.textContent).toContain("Save a whisper-cli path above");
            expect(result()!.classList.contains("setting-mic-test-result--error")).toBe(true);
            expect(st.tracks[0].stop).toHaveBeenCalledTimes(1);
            expect(startBtn().textContent).toBe("Start test");
            expect(cancelBtn()).toBeNull();
        });

        it("reads the engine when Start is clicked, not when the clip starts", async () => {
            fetchMock.mockReturnValue(new Promise(() => {}));
            await start();
            setSettings({ "voice:engine": "whisper-local" });
            await flush(1200);
            expect(FakeRecorder.instances).toHaveLength(1);
        });

        it("does not start the clip if the stream was torn down during the wait", async () => {
            await start();
            fireEvent.click(cancelBtn()!);
            await flush(5000);
            expect(FakeRecorder.instances).toHaveLength(0);
        });
    });

    describe("transcribing", () => {
        const reach = async (settings: Record<string, unknown> = {}) => {
            await start(settings);
            await flush(1200 + 2500);
        };

        it("posts to the transcribe endpoint with the encoded mime and the auth headers", async () => {
            h.pickMime.mockReturnValue("audio/webm;codecs=opus");
            fetchMock.mockResolvedValue({ ok: true, json: async () => ({ text: "hello" }) });
            await reach();
            expect(fetchMock).toHaveBeenCalledTimes(1);
            const [url, init] = fetchMock.mock.calls[0];
            expect(url).toBe("http://srv.test/api/v1/voice/transcribe?mime=audio%2Fwebm%3Bcodecs%3Dopus");
            expect(init.method).toBe("POST");
            expect(init.headers).toEqual({ "Content-Type": "audio/webm;codecs=opus", Authorization: "Bearer k" });
            expect(init.body).toBeInstanceOf(Blob);
        });

        it("shows the trimmed transcript and goes back to Start test", async () => {
            fetchMock.mockResolvedValue({ ok: true, json: async () => ({ text: "  hello there \n" }) });
            await reach();
            expect(result()!.textContent).toBe("hello there");
            expect(result()!.classList.contains("setting-mic-test-result--error")).toBe(false);
            expect(startBtn().textContent).toBe("Start test");
            expect(startBtn().disabled).toBe(false);
            expect(cancelBtn()).toBeNull();
        });

        it.each([{ text: "   " }, { text: "" }, {}])("says no speech was detected for %j", async (body) => {
            fetchMock.mockResolvedValue({ ok: true, json: async () => body });
            await reach();
            expect(result()!.textContent).toBe("(no speech detected)");
        });

        it("shows the server's error text when the request is refused", async () => {
            fetchMock.mockResolvedValue({ ok: false, status: 502, json: async () => ({ error: "groq key rejected" }) });
            await reach();
            expect(result()!.textContent).toBe("groq key rejected");
            expect(result()!.classList.contains("setting-mic-test-result--error")).toBe(true);
            expect(startBtn().textContent).toBe("Start test");
        });

        it("falls back to the status code when the error body has no text or is not JSON", async () => {
            fetchMock.mockResolvedValue({ ok: false, status: 503, json: async () => ({}) });
            await reach();
            expect(result()!.textContent).toBe("Transcription failed (503).");
            cleanup();
            fetchMock.mockResolvedValue({
                ok: false,
                status: 500,
                json: async () => {
                    throw new Error("not json");
                },
            });
            await reach();
            expect(result()!.textContent).toBe("Transcription failed (500).");
        });

        it("shows the message of a network failure, or a generic one for a non-Error", async () => {
            fetchMock.mockRejectedValue(new Error("offline"));
            await reach();
            expect(result()!.textContent).toBe("offline");
            expect(result()!.classList.contains("setting-mic-test-result--error")).toBe(true);
            cleanup();
            fetchMock.mockRejectedValue("nope");
            await reach();
            expect(result()!.textContent).toBe("Request failed.");
        });

        it("drops a response that arrives after Cancel", async () => {
            let resolve!: (v: unknown) => void;
            fetchMock.mockReturnValue(new Promise((r) => (resolve = r)));
            await reach();
            fireEvent.click(cancelBtn()!);
            resolve({ ok: true, json: async () => ({ text: "late" }) });
            await flush();
            expect(result()).toBeNull();
            expect(startBtn().textContent).toBe("Start test");
        });

        it("drops a failure response that arrives after Cancel", async () => {
            let resolve!: (v: unknown) => void;
            fetchMock.mockReturnValue(new Promise((r) => (resolve = r)));
            await reach();
            fireEvent.click(cancelBtn()!);
            resolve({ ok: false, status: 500, json: async () => ({ error: "late failure" }) });
            await flush();
            expect(result()).toBeNull();
        });

        it("drops a failure whose body is still being read when Cancel is clicked", async () => {
            let resolveBody!: (v: unknown) => void;
            fetchMock.mockResolvedValue({ ok: false, status: 500, json: () => new Promise((r) => (resolveBody = r)) });
            await reach();
            fireEvent.click(cancelBtn()!);
            resolveBody({ error: "late detail" });
            await flush();
            expect(result()).toBeNull();
        });

        it("drops a success body that is still being read when Cancel is clicked", async () => {
            let resolveBody!: (v: unknown) => void;
            fetchMock.mockResolvedValue({ ok: true, json: () => new Promise((r) => (resolveBody = r)) });
            await reach();
            fireEvent.click(cancelBtn()!);
            resolveBody({ text: "late text" });
            await flush();
            expect(result()).toBeNull();
        });

        it("drops a network failure that arrives after Cancel", async () => {
            let reject!: (e: unknown) => void;
            fetchMock.mockReturnValue(new Promise((_, r) => (reject = r)));
            await reach();
            fireEvent.click(cancelBtn()!);
            reject(new Error("late network"));
            await flush();
            expect(result()).toBeNull();
        });
    });

    describe("Cancel and teardown", () => {
        it("while listening: stops the meter and tracks, and never transcribes", async () => {
            await start();
            h.meterStop.mockClear();
            fireEvent.click(cancelBtn()!);
            expect(h.meterStop).toHaveBeenCalled();
            expect(st.tracks[0].stop).toHaveBeenCalledTimes(1);
            expect(startBtn().textContent).toBe("Start test");
            expect(cancelBtn()).toBeNull();
            await flush(10000);
            expect(fetchMock).not.toHaveBeenCalled();
        });

        it("while recording: stops the recorder without letting its stop start a transcription", async () => {
            await start();
            await flush(1200);
            const rec = FakeRecorder.instances[0];
            fireEvent.click(cancelBtn()!);
            expect(rec.stopped).toBe(1);
            expect(rec.onstop).toBeNull();
            expect(startBtn().textContent).toBe("Start test");
            await flush(10000);
            expect(fetchMock).not.toHaveBeenCalled();
            expect(rec.stopped).toBe(1);
        });

        it("survives a recorder that throws on stop and still releases the microphone", async () => {
            await start();
            await flush(1200);
            FakeRecorder.instances[0].stopThrows = true;
            fireEvent.click(cancelBtn()!);
            expect(st.tracks[0].stop).toHaveBeenCalledTimes(1);
            expect(startBtn().textContent).toBe("Start test");
        });

        it("leaves an inactive recorder alone", async () => {
            await start();
            await flush(1200);
            const rec = FakeRecorder.instances[0];
            rec.state = "inactive";
            fireEvent.click(cancelBtn()!);
            expect(rec.stopped).toBe(0);
        });

        it("a second test after Cancel runs normally", async () => {
            fetchMock.mockResolvedValue({ ok: true, json: async () => ({ text: "second" }) });
            await start();
            fireEvent.click(cancelBtn()!);
            getUserMedia.mockResolvedValue(fakeStream().stream);
            fireEvent.click(startBtn());
            await flush(1200 + 2500);
            expect(result()!.textContent).toBe("second");
        });

        it("a second test after a finished one releases the first stream before opening another", async () => {
            fetchMock.mockResolvedValue({ ok: true, json: async () => ({ text: "one" }) });
            await start();
            await flush(1200 + 2500);
            expect(st.tracks[0].stop).not.toHaveBeenCalled();
            getUserMedia.mockResolvedValue(fakeStream().stream);
            fireEvent.click(startBtn());
            expect(st.tracks[0].stop).toHaveBeenCalledTimes(1);
            await flush();
        });

        it("unmounting during a test releases the microphone and silences the timers", async () => {
            await start();
            await flush(1200);
            const rec = FakeRecorder.instances[0];
            cleanup();
            expect(st.tracks[0].stop).toHaveBeenCalledTimes(1);
            expect(rec.stopped).toBe(1);
            await flush(10000);
            expect(fetchMock).not.toHaveBeenCalled();
        });

        it("unmounting before the clip starts means no recorder is ever made", async () => {
            await start();
            cleanup();
            await flush(10000);
            expect(FakeRecorder.instances).toHaveLength(0);
        });

        it("a cancelled test's clip timer does nothing once another test is running", async () => {
            await start();
            fireEvent.click(cancelBtn()!);
            const second = fakeStream();
            getUserMedia.mockResolvedValue(second.stream);
            fireEvent.click(startBtn());
            await flush(1200);
            expect(FakeRecorder.instances).toHaveLength(1);
            expect(FakeRecorder.instances[0].stream).toBe(second.stream);
        });

        it("starting again after Cancel does not stop the first stream and recorder a second time", async () => {
            FakeRecorder.sticky = true;
            await start();
            await flush(1200);
            const first = FakeRecorder.instances[0];
            fireEvent.click(cancelBtn()!);
            getUserMedia.mockResolvedValue(fakeStream().stream);
            fireEvent.click(startBtn());
            await flush(5000);
            expect(first.stopped).toBe(1);
            expect(st.tracks[0].stop).toHaveBeenCalledTimes(1);
        });

        it("a clip stopped from outside after Cancel does not transcribe", async () => {
            await start();
            await flush(1200);
            const rec = FakeRecorder.instances[0];
            const onstop = rec.onstop!;
            fireEvent.click(cancelBtn()!);
            onstop();
            await flush();
            expect(fetchMock).not.toHaveBeenCalled();
            expect(startBtn().textContent).toBe("Start test");
        });
    });
});
