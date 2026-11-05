// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * The Whisper capture-and-send voice engine, one behaviour per test: when it is available, which
 * microphone it asks for and what each refusal reports, how both capture modes (webm from a
 * MediaRecorder, WAV from raw PCM) cut speech into utterances on silence and on the length cap,
 * what is posted to the transcribe endpoint and how its answers and failures reach the pane, that
 * utterances are applied in the order they were spoken, and what stopping releases. The microphone,
 * the audio graph, the recorder and `fetch` are fakes; the level reading is set by the test.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ settings: {} as Record<string, unknown>, rms: 0, polls: 0 }));

vi.mock("@/app/store/global", () => ({ getSettingsKeyAtom: (key: string) => () => h.settings[key] }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv:1" }));
vi.mock("@/app/store/auth-headers", () => ({
    authHeaders: (extra: Record<string, string> = {}) => ({ ...extra, "X-Auth": "key" }),
}));
vi.mock("./audioLevel", () => ({
    computeRms: () => {
        h.polls++;
        return h.rms;
    },
}));

import type { PaneVoiceHandle } from "./useVoiceInput";
import { createWhisperVoiceSession, pickMime } from "./whisperVoiceEngine";

class FakeTrack {
    stop = vi.fn();
}
class FakeProcessor {
    onaudioprocess: ((e: AudioProcessingEvent) => void) | null = null;
    connect = vi.fn();
    disconnect = vi.fn();
}
class FakeAnalyser {
    fftSize = 0;
    connect = vi.fn();
}
class FakeAudioContext {
    static instances: FakeAudioContext[] = [];
    static failConstruct = false;
    static failClose = false;
    options: { sampleRate?: number } | undefined;
    destination = {};
    analyser: FakeAnalyser | null = null;
    processor: FakeProcessor | null = null;
    processorArgs: number[] = [];
    source = { connect: vi.fn() };
    close = vi.fn(async () => {
        if (FakeAudioContext.failClose) throw new Error("close failed");
    });
    constructor(options?: { sampleRate?: number }) {
        if (FakeAudioContext.failConstruct) throw new Error("bad rate");
        this.options = options;
        FakeAudioContext.instances.push(this);
    }
    createMediaStreamSource = vi.fn(() => this.source);
    createAnalyser = vi.fn(() => (this.analyser = new FakeAnalyser()));
    createScriptProcessor = vi.fn((...args: number[]) => {
        this.processorArgs = args;
        return (this.processor = new FakeProcessor());
    });
}
class FakeRecorder {
    static instances: FakeRecorder[] = [];
    static supported: string[] = ["audio/webm;codecs=opus", "audio/webm", "audio/ogg;codecs=opus", "audio/mp4"];
    static stopThrows = false;
    static isTypeSupported(mime: string) {
        return FakeRecorder.supported.includes(mime);
    }
    state = "inactive";
    ondataavailable: ((e: { data: Blob }) => void) | null = null;
    onstop: (() => void) | null = null;
    starts = 0;
    constructor(
        public stream: unknown,
        public options: { mimeType: string }
    ) {
        FakeRecorder.instances.push(this);
    }
    start() {
        this.state = "recording";
        this.starts++;
    }
    stop() {
        if (FakeRecorder.stopThrows) throw new Error("stop failed");
        this.state = "inactive";
        this.onstop?.();
    }
    emit(text: string) {
        this.ondataavailable?.({ data: new Blob([text]) });
    }
}

let tracks: FakeTrack[];
const getUserMedia = vi.fn();
const fetchMock = vi.fn();
const errors = vi.fn();
const onErrorEvent = (e: Event) => errors((e as CustomEvent).detail);

const okJson = (body: unknown) => ({ ok: true, status: 200, json: async () => body });
const failJson = (status: number, body: unknown) => ({ ok: false, status, json: async () => body });

const setMediaDevices = (value: unknown) =>
    Object.defineProperty(navigator, "mediaDevices", { value, configurable: true });
const setLanguage = (value: string) => Object.defineProperty(navigator, "language", { value, configurable: true });

beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000_000);
    h.settings = {};
    h.rms = 0;
    h.polls = 0;
    tracks = [];
    FakeAudioContext.instances = [];
    FakeAudioContext.failConstruct = false;
    FakeAudioContext.failClose = false;
    FakeRecorder.instances = [];
    FakeRecorder.supported = ["audio/webm;codecs=opus", "audio/webm", "audio/ogg;codecs=opus", "audio/mp4"];
    FakeRecorder.stopThrows = false;
    getUserMedia.mockReset();
    getUserMedia.mockImplementation(async () => {
        tracks = [new FakeTrack(), new FakeTrack()];
        return { getTracks: () => tracks };
    });
    fetchMock.mockReset();
    fetchMock.mockImplementation(async () => okJson({ text: "hello" }));
    errors.mockReset();
    setMediaDevices({ getUserMedia });
    setLanguage("en-US");
    vi.stubGlobal("AudioContext", FakeAudioContext);
    vi.stubGlobal("MediaRecorder", FakeRecorder);
    vi.stubGlobal("fetch", fetchMock);
    window.addEventListener("voice-input-error", onErrorEvent);
});
afterEach(() => {
    window.removeEventListener("voice-input-error", onErrorEvent);
    vi.unstubAllGlobals();
    vi.useRealTimers();
    Reflect.deleteProperty(navigator, "mediaDevices");
});

const makeHandle = () => ({ appendFinal: vi.fn(), setInterim: vi.fn() }) satisfies PaneVoiceHandle;
const flush = () => vi.advanceTimersByTimeAsync(0);
const tick = (ms: number) => vi.advanceTimersByTimeAsync(ms);

function open(local = false, withHandle = true) {
    h.settings["voice:engine"] = local ? "whisper-local" : "groq";
    const session = createWhisperVoiceSession();
    const handle = makeHandle();
    if (withHandle) session.registerPane("blk", handle);
    return { session, handle };
}
async function listen(local = false, withHandle = true) {
    const o = open(local, withHandle);
    o.session.toggleListening();
    await flush();
    return o;
}
const recorder = () => FakeRecorder.instances[FakeRecorder.instances.length - 1];
const ctx = () => FakeAudioContext.instances[FakeAudioContext.instances.length - 1];

/** Feeds the WAV capture one audio block of the given samples. */
function block(samples: Float32Array | number[]) {
    const data = samples instanceof Float32Array ? samples : new Float32Array(samples);
    ctx().processor!.onaudioprocess!({
        inputBuffer: { getChannelData: (channel: number) => (channel === 0 ? data : new Float32Array(0)) },
    } as unknown as AudioProcessingEvent);
}
/** One spoken utterance in WAV mode: speech, a pause long enough to cut, then the cut. */
function wavUtterance(speech: number[] = [0.5, 0.5], pauseMs = 800) {
    block(speech);
    vi.advanceTimersByTime(400);
    block([0]);
    vi.advanceTimersByTime(pauseMs);
    block([0]);
}
const readBytes = async (blob: Blob) => {
    vi.useRealTimers();
    try {
        return await new Promise<DataView>((resolve) => {
            const reader = new FileReader();
            reader.onload = () => resolve(new DataView(reader.result as ArrayBuffer));
            reader.readAsArrayBuffer(blob);
        });
    } finally {
        vi.useFakeTimers();
    }
};
const ascii = (view: DataView, off: number, n: number) =>
    String.fromCharCode(...Array.from({ length: n }, (_, i) => view.getUint8(off + i)));

describe("availability", () => {
    it("is unavailable without a microphone API, and then does nothing", () => {
        for (const devices of [undefined, {}]) {
            setMediaDevices(devices);
            const session = createWhisperVoiceSession();
            expect(session.isAvailable()).toBe(false);
            session.registerPane("blk", makeHandle());
            session.toggleListening();
            expect(session.isListening()).toBe(false);
            expect(session.currentTargetId()).toBeNull();
            expect(session.lastError()).toBeNull();
            expect(session.lastErrorDetail()).toBeNull();
        }
        expect(getUserMedia).not.toHaveBeenCalled();
    });

    it("is unavailable where there is no navigator", () => {
        vi.stubGlobal("navigator", undefined);
        expect(createWhisperVoiceSession().isAvailable()).toBe(false);
    });

    it("is unavailable without an audio context", () => {
        vi.stubGlobal("AudioContext", undefined);
        expect(createWhisperVoiceSession().isAvailable()).toBe(false);
    });

    it("needs a media recorder for the webm engine, but not for the local WAV engine", () => {
        vi.stubGlobal("MediaRecorder", undefined);
        h.settings["voice:engine"] = "groq";
        expect(createWhisperVoiceSession().isAvailable()).toBe(false);
        h.settings["voice:engine"] = "whisper-local";
        expect(createWhisperVoiceSession().isAvailable()).toBe(true);
    });

    it("is available with everything present, and registering a pane targets it", () => {
        const { session } = open();
        expect(session.isAvailable()).toBe(true);
        expect(session.currentTargetId()).toBe("blk");
        expect(session.isListening()).toBe(false);
    });
});

describe("pickMime", () => {
    it("prefers opus in webm, then falls down the list", () => {
        expect(pickMime()).toBe("audio/webm;codecs=opus");
        FakeRecorder.supported = ["audio/webm", "audio/mp4"];
        expect(pickMime()).toBe("audio/webm");
        FakeRecorder.supported = ["audio/ogg;codecs=opus", "audio/mp4"];
        expect(pickMime()).toBe("audio/ogg;codecs=opus");
        FakeRecorder.supported = ["audio/mp4"];
        expect(pickMime()).toBe("audio/mp4");
    });

    it("answers audio/webm when nothing is supported, or there is no recorder", () => {
        FakeRecorder.supported = [];
        expect(pickMime()).toBe("audio/webm");
        vi.stubGlobal("MediaRecorder", undefined);
        expect(pickMime()).toBe("audio/webm");
    });
});

describe("starting capture", () => {
    it("asks for the default microphone, and says it is listening", async () => {
        const { session } = await listen();
        expect(getUserMedia).toHaveBeenCalledWith({ audio: true });
        expect(session.isListening()).toBe(true);
    });

    it("asks for the chosen device exactly, but treats 'default' and none as no choice", async () => {
        h.settings["voice:inputDeviceId"] = "mic-2";
        await listen();
        expect(getUserMedia).toHaveBeenLastCalledWith({ audio: { deviceId: { exact: "mic-2" } } });
        h.settings["voice:inputDeviceId"] = "default";
        await listen();
        expect(getUserMedia).toHaveBeenLastCalledWith({ audio: true });
        h.settings["voice:inputDeviceId"] = "";
        await listen();
        expect(getUserMedia).toHaveBeenLastCalledWith({ audio: true });
    });

    it("clears the last error and detail it showed", async () => {
        const { session } = open();
        getUserMedia.mockRejectedValueOnce(Object.assign(new Error("x"), { name: "NotAllowedError" }));
        session.toggleListening();
        await flush();
        expect(session.lastError()).toBe("not-allowed");
        session.toggleListening();
        await flush();
        expect(session.lastError()).toBeNull();
        expect(session.isListening()).toBe(true);
    });

    it("webm mode: records in the chosen mime from a plain audio context, metering through an analyser", async () => {
        FakeRecorder.supported = ["audio/webm"];
        await listen();
        expect(ctx().options).toBeUndefined();
        expect(ctx().analyser!.fftSize).toBe(2048);
        expect(ctx().source.connect).toHaveBeenCalledWith(ctx().analyser);
        expect(recorder().options).toEqual({ mimeType: "audio/webm" });
        expect(recorder().state).toBe("recording");
        expect(recorder().starts).toBe(1);
    });

    it("WAV mode: captures 16 kHz mono PCM through a script processor wired to the output", async () => {
        await listen(true);
        expect(ctx().options).toEqual({ sampleRate: 16000 });
        expect(ctx().processorArgs).toEqual([4096, 1, 1]);
        expect(ctx().source.connect).toHaveBeenCalledWith(ctx().processor);
        expect(ctx().processor!.connect).toHaveBeenCalledWith(ctx().destination);
        expect(typeof ctx().processor!.onaudioprocess).toBe("function");
        expect(FakeRecorder.instances.length).toBe(0);
    });

    it("toggling while listening stops, and toggling again starts again", async () => {
        const { session } = await listen();
        session.toggleListening();
        await flush();
        expect(session.isListening()).toBe(false);
        session.toggleListening();
        await flush();
        expect(session.isListening()).toBe(true);
        expect(FakeAudioContext.instances.length).toBe(2);
    });
});

describe("refusals", () => {
    const refuse = async (name: string | undefined) => {
        const { session } = open();
        getUserMedia.mockRejectedValueOnce(name === undefined ? {} : Object.assign(new Error("x"), { name }));
        session.toggleListening();
        await flush();
        return session;
    };

    it.each([
        ["NotAllowedError", "not-allowed"],
        ["SecurityError", "not-allowed"],
        ["NotFoundError", "audio-capture"],
        ["OverconstrainedError", "audio-capture"],
        ["AbortError", "service-not-allowed"],
        [undefined, "service-not-allowed"],
    ])("a %s refusal is reported as %s, once, and nothing starts", async (name, code) => {
        const session = await refuse(name);
        expect(session.lastError()).toBe(code);
        expect(errors).toHaveBeenCalledTimes(1);
        expect(errors).toHaveBeenCalledWith(code);
        expect(session.isListening()).toBe(false);
        expect(FakeAudioContext.instances.length).toBe(0);
    });

    it("closes the microphone and stops listening when the audio graph can't be built", async () => {
        FakeAudioContext.failConstruct = true;
        const { session } = await listen();
        expect(session.lastError()).toBe("service-not-allowed");
        expect(errors).toHaveBeenCalledWith("service-not-allowed");
        expect(session.isListening()).toBe(false);
        expect(tracks.every((t) => t.stop.mock.calls.length === 1)).toBe(true);
    });
});

describe("cutting utterances (webm)", () => {
    const poll = async (rms: number, ms = 100) => {
        h.rms = rms;
        await tick(ms);
    };

    it("reads the level once every 100 ms", async () => {
        await listen();
        await tick(99);
        expect(h.polls).toBe(0);
        await tick(1);
        expect(h.polls).toBe(1);
        await tick(100);
        expect(h.polls).toBe(2);
        await tick(300);
        expect(h.polls).toBe(5);
    });

    it("cuts after 800 ms of silence following speech, and not a moment before", async () => {
        await listen();
        await poll(0.5, 400);
        await poll(0, 100); // silence noticed
        await poll(0, 700);
        expect(recorder().state).toBe("recording");
        await poll(0, 100);
        expect(FakeRecorder.instances.length).toBe(2);
    });

    it("counts a level exactly at the threshold as speech", async () => {
        await listen();
        await poll(0.012, 400);
        await poll(0, 100);
        await poll(0, 800);
        expect(FakeRecorder.instances.length).toBe(2);
    });

    it("counts a level just under the threshold as silence", async () => {
        await listen();
        await poll(0.012, 400);
        await poll(0.0119, 100);
        await poll(0.0119, 800);
        expect(FakeRecorder.instances.length).toBe(2);
    });

    it("starts the silence over when speech comes back", async () => {
        await listen();
        await poll(0.5, 400);
        await poll(0, 500);
        await poll(0.5, 100);
        await poll(0, 100);
        await poll(0, 600);
        expect(FakeRecorder.instances.length).toBe(1);
        await poll(0, 200);
        expect(FakeRecorder.instances.length).toBe(2);
    });

    it("waits for speech: silence alone never cuts before the length cap", async () => {
        await listen();
        await poll(0, 11_900);
        expect(FakeRecorder.instances.length).toBe(1);
    });

    it("cuts at the 12 s cap whatever was heard, and sends only a segment with speech", async () => {
        const { handle } = await listen();
        await poll(0, 12_000);
        expect(FakeRecorder.instances.length).toBe(2);
        expect(fetchMock).not.toHaveBeenCalled();
        recorder().emit("voice");
        await poll(0.5, 12_000);
        expect(fetchMock).toHaveBeenCalledTimes(1);
        expect(handle.appendFinal).toHaveBeenCalledWith("hello");
    });

    it("does not send what was recorded in a segment with no speech", async () => {
        await listen();
        recorder().emit("noise");
        await poll(0, 12_000);
        expect(FakeRecorder.instances.length).toBe(2);
        expect(fetchMock).not.toHaveBeenCalled();
    });

    it("ignores a piece of recording that has no data", async () => {
        await listen();
        recorder().ondataavailable!({ data: undefined as unknown as Blob });
        recorder().emit("x");
        await poll(0.5, 400);
        await poll(0, 900);
        expect((fetchMock.mock.calls[0][1].body as Blob).size).toBe(1);
    });

    it("posts the recorded data as one blob in the capture mime, then records the next utterance", async () => {
        FakeRecorder.supported = ["audio/webm"];
        await listen();
        recorder().emit("ab");
        recorder().emit("cde");
        await poll(0.5, 400);
        await poll(0, 900);
        const [, init] = fetchMock.mock.calls[0];
        expect((init.body as Blob).size).toBe(5);
        expect((init.body as Blob).type).toBe("audio/webm");
        expect(FakeRecorder.instances.length).toBe(2);
        expect(recorder().state).toBe("recording");
    });

    it("leaves out empty pieces of recording", async () => {
        await listen();
        recorder().emit("");
        recorder().emit("x");
        await poll(0.5, 400);
        await poll(0, 900);
        expect((fetchMock.mock.calls[0][1].body as Blob).size).toBe(1);
    });

    it("drops a blip shorter than 350 ms, and keeps one of exactly 350 ms", async () => {
        vi.setSystemTime(1_000_000);
        const { session } = await listen();
        recorder().emit("blip");
        // A cut at < 350 ms: the cap can't be used, so stop the recorder directly.
        h.rms = 0.5;
        await tick(100);
        recorder().stop();
        await flush();
        expect(fetchMock).not.toHaveBeenCalled();
        recorder().emit("edge");
        h.rms = 0.5;
        await tick(300);
        h.rms = 0.5;
        vi.advanceTimersByTime(50);
        recorder().stop();
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(1);
        expect(session.isListening()).toBe(true);
        recorder().emit("short");
        h.rms = 0.5;
        await tick(349);
        recorder().stop();
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(1);
    });

    it("sends nothing when the speech segment recorded no data", async () => {
        await listen();
        await poll(0.5, 400);
        await poll(0, 900);
        expect(fetchMock).not.toHaveBeenCalled();
        expect(FakeRecorder.instances.length).toBe(2);
    });

    it("does not stop a recorder that is already stopped", async () => {
        await listen();
        const first = recorder();
        const stop = vi.spyOn(first, "stop");
        first.state = "inactive";
        await poll(0.5, 400);
        await poll(0, 900);
        expect(stop).not.toHaveBeenCalled();
    });

    it("stops recording when stopped, without restarting or sending the cut utterance", async () => {
        const { session } = await listen();
        recorder().emit("voice");
        await poll(0.5, 400);
        session.toggleListening();
        await flush();
        expect(FakeRecorder.instances.length).toBe(1);
        expect(recorder().state).toBe("inactive");
        expect(fetchMock).not.toHaveBeenCalled();
    });
});

describe("cutting utterances (WAV)", () => {
    it("sends a spoken utterance as a WAV blob, then starts the next one clean", async () => {
        const { handle } = await listen(true);
        wavUtterance();
        await flush();
        const [, init] = fetchMock.mock.calls[0];
        expect((init.body as Blob).type).toBe("audio/wav");
        expect(handle.appendFinal).toHaveBeenCalledWith("hello");
        wavUtterance();
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(2);
    });

    it("writes a 16 kHz mono 16-bit PCM WAV header and the clamped samples", async () => {
        await listen(true);
        wavUtterance([0, 1, -1, 2, -2, 0.5, -0.5]);
        await flush();
        const blob = fetchMock.mock.calls[0][1].body as Blob;
        const v = await readBytes(blob);
        const samples = 7 + 1 + 1;
        expect(blob.size).toBe(44 + samples * 2);
        expect(ascii(v, 0, 4)).toBe("RIFF");
        expect(v.getUint32(4, true)).toBe(36 + samples * 2);
        expect(ascii(v, 8, 4)).toBe("WAVE");
        expect(ascii(v, 12, 4)).toBe("fmt ");
        expect(v.getUint32(16, true)).toBe(16);
        expect(v.getUint16(20, true)).toBe(1);
        expect(v.getUint16(22, true)).toBe(1);
        expect(v.getUint32(24, true)).toBe(16000);
        expect(v.getUint32(28, true)).toBe(32000);
        expect(v.getUint16(32, true)).toBe(2);
        expect(v.getUint16(34, true)).toBe(16);
        expect(ascii(v, 36, 4)).toBe("data");
        expect(v.getUint32(40, true)).toBe(samples * 2);
        const pcm = Array.from({ length: 7 }, (_, i) => v.getInt16(44 + i * 2, true));
        expect(pcm).toEqual([0, 32767, -32768, 32767, -32768, Math.trunc(0.5 * 0x7fff), -16384]);
    });

    it("copies each block, so the engine reusing its buffer can't change what was captured", async () => {
        await listen(true);
        const reused = new Float32Array([0.5, 0.5]);
        block(reused);
        reused.fill(0);
        vi.advanceTimersByTime(400);
        block([0]);
        vi.advanceTimersByTime(800);
        block([0]);
        await flush();
        const v = await readBytes(fetchMock.mock.calls[0][1].body as Blob);
        expect(v.getInt16(44, true)).toBe(Math.trunc(0.5 * 0x7fff));
    });

    it("joins the blocks of an utterance in the order they arrived", async () => {
        await listen(true);
        block([0.25]);
        block([0.5]);
        vi.advanceTimersByTime(400);
        block([0]);
        vi.advanceTimersByTime(800);
        block([0]);
        await flush();
        const v = await readBytes(fetchMock.mock.calls[0][1].body as Blob);
        expect([v.getInt16(44, true), v.getInt16(46, true)]).toEqual([
            Math.trunc(0.25 * 0x7fff),
            Math.trunc(0.5 * 0x7fff),
        ]);
    });

    it("measures the level of each block as its root mean square", async () => {
        await listen(true);
        block([0.011, -0.011, 0.011, -0.011]);
        vi.advanceTimersByTime(12_000);
        block([0]);
        await flush();
        expect(fetchMock).not.toHaveBeenCalled();
        block([0.0121, -0.0121, 0.0121, -0.0121]);
        vi.advanceTimersByTime(400);
        block([0]);
        vi.advanceTimersByTime(800);
        block([0]);
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(1);
    });

    it("reads the first channel", async () => {
        await listen(true);
        const read = vi.fn((channel: number) => (channel === 0 ? new Float32Array([0.5]) : new Float32Array(0)));
        ctx().processor!.onaudioprocess!({ inputBuffer: { getChannelData: read } } as unknown as AudioProcessingEvent);
        expect(read).toHaveBeenCalledWith(0);
        expect(read).toHaveBeenCalledTimes(1);
    });

    it("cuts when the silence has lasted 800 ms, not 799", async () => {
        await listen(true);
        block([0.5]);
        vi.advanceTimersByTime(400);
        block([0]);
        vi.advanceTimersByTime(799);
        block([0]);
        await flush();
        expect(fetchMock).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        block([0]);
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(1);
    });

    it("starts each utterance with no speech heard and the cap counted from the last cut", async () => {
        await listen(true);
        wavUtterance();
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(1);
        block([0]);
        vi.advanceTimersByTime(12_000);
        block([0]);
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(1);
        block([0.5]);
        vi.advanceTimersByTime(11_000);
        block([0.5]);
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(1);
    });

    it("leaves out what an earlier stop left half captured", async () => {
        const { session } = await listen(true);
        block([0.5, 0.5]);
        session.toggleListening();
        await flush();
        session.toggleListening();
        await flush();
        wavUtterance([0.25]);
        await flush();
        expect((fetchMock.mock.calls[0][1].body as Blob).size).toBe(44 + 3 * 2);
    });

    it("drops a segment with no speech", async () => {
        await listen(true);
        block([0]);
        vi.advanceTimersByTime(12_000);
        block([0]);
        await flush();
        expect(fetchMock).not.toHaveBeenCalled();
    });

    it("cuts at the 12 s cap even in unbroken speech", async () => {
        await listen(true);
        block([0.5]);
        vi.advanceTimersByTime(11_999);
        block([0.5]);
        await flush();
        expect(fetchMock).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        block([0.5]);
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(1);
    });

    it("stops detaching the processor from the callback and the graph when stopped", async () => {
        const { session } = await listen(true);
        const proc = ctx().processor!;
        session.toggleListening();
        await flush();
        expect(proc.onaudioprocess).toBeNull();
        expect(proc.disconnect).toHaveBeenCalledTimes(1);
    });
});

describe("posting an utterance", () => {
    const utter = async (local = true) => {
        const o = await listen(local);
        wavUtterance();
        await flush();
        return o;
    };

    it("posts the clip to the transcribe endpoint with the mime and language, and the auth headers", async () => {
        await utter();
        const [url, init] = fetchMock.mock.calls[0];
        expect(url).toBe("http://srv:1/api/v1/voice/transcribe?mime=audio%2Fwav&lang=en");
        expect(init.method).toBe("POST");
        expect(init.headers).toEqual({ "Content-Type": "audio/wav", "X-Auth": "key" });
    });

    it("names the webm mime the recorder chose", async () => {
        FakeRecorder.supported = ["audio/ogg;codecs=opus"];
        await listen();
        recorder().emit("x");
        h.rms = 0.5;
        await tick(400);
        h.rms = 0;
        await tick(900);
        const [url, init] = fetchMock.mock.calls[0];
        expect(url).toContain("mime=audio%2Fogg%3Bcodecs%3Dopus");
        expect(init.headers["Content-Type"]).toBe("audio/ogg;codecs=opus");
    });

    it("sends an empty language when the browser has none", async () => {
        setLanguage("");
        await utter();
        expect(fetchMock.mock.calls[0][0]).toMatch(/&lang=$/);
    });

    it("encodes the language it sends", async () => {
        setLanguage("x y-Z");
        await utter();
        expect(fetchMock.mock.calls[0][0]).toMatch(/&lang=x%20y$/);
    });

    it("keeps posting later utterances after a pane's handle throws while one is being reported", async () => {
        const { handle } = await listen(true);
        fetchMock.mockImplementationOnce(async () => {
            throw new Error("offline");
        });
        handle.setInterim
            .mockImplementationOnce(() => {})
            .mockImplementationOnce(() => {
                throw new Error("pane gone");
            });
        wavUtterance();
        await flush();
        wavUtterance();
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(2);
        expect(handle.appendFinal).toHaveBeenCalledWith("hello");
    });

    it("shows a placeholder while it waits, then clears it", async () => {
        const { handle } = await utter();
        expect(handle.setInterim.mock.calls.map((c) => c[0])).toEqual(["…", ""]);
    });

    it("adds the transcript, trimmed, and clears the last error detail", async () => {
        const { session, handle } = await listen(true);
        fetchMock.mockImplementationOnce(async () => failJson(500, { error: "first" }));
        wavUtterance();
        await flush();
        expect(session.lastErrorDetail()).toBe("first");
        fetchMock.mockImplementationOnce(async () => okJson({ text: "  spoken words \n" }));
        wavUtterance();
        await flush();
        expect(handle.appendFinal).toHaveBeenCalledWith("spoken words");
        expect(session.lastErrorDetail()).toBeNull();
    });

    it.each([[{ text: "   " }], [{ text: "" }], [{}]])("adds nothing for the answer %j", async (body) => {
        fetchMock.mockImplementation(async () => okJson(body));
        const { handle } = await utter();
        expect(handle.appendFinal).not.toHaveBeenCalled();
    });

    it("sends nothing when no pane is targeted", async () => {
        await listen(true, false);
        wavUtterance();
        await flush();
        expect(fetchMock).not.toHaveBeenCalled();
    });

    it("reports a server error with its message, keeps listening, and carries on", async () => {
        fetchMock.mockImplementationOnce(async () => failJson(500, { error: "model missing" }));
        const { session, handle } = await utter();
        expect(session.lastErrorDetail()).toBe("model missing");
        expect(session.lastError()).toBe("service-not-allowed");
        expect(errors).toHaveBeenCalledWith("service-not-allowed");
        expect(session.isListening()).toBe(true);
        expect(handle.setInterim.mock.calls.map((c) => c[0])).toEqual(["…", ""]);
        expect(handle.appendFinal).not.toHaveBeenCalled();
    });

    it("has no detail when the error body isn't readable or has no message", async () => {
        const { session } = await listen(true);
        fetchMock.mockImplementationOnce(async () => ({
            ok: false,
            status: 500,
            json: async () => {
                throw new Error("not json");
            },
        }));
        wavUtterance();
        await flush();
        expect(session.lastErrorDetail()).toBeNull();
        fetchMock.mockImplementationOnce(async () => failJson(502, {}));
        wavUtterance();
        await flush();
        expect(session.lastErrorDetail()).toBeNull();
    });

    it("stops listening when the service says it isn't configured (501)", async () => {
        fetchMock.mockImplementationOnce(async () => failJson(501, { error: "no backend" }));
        const { session } = await utter();
        expect(session.lastError()).toBe("service-not-allowed");
        expect(session.lastErrorDetail()).toBe("no backend");
        expect(errors).toHaveBeenCalledTimes(1);
        expect(session.isListening()).toBe(false);
        expect(tracks.every((t) => t.stop.mock.calls.length === 1)).toBe(true);
    });

    it("reports a failed request with its message, keeps listening, and clears the placeholder", async () => {
        fetchMock.mockImplementationOnce(async () => {
            throw new Error("offline");
        });
        const { session, handle } = await utter();
        expect(session.lastError()).toBe("service-not-allowed");
        expect(session.lastErrorDetail()).toBe("offline");
        expect(errors).toHaveBeenCalledWith("service-not-allowed");
        expect(session.isListening()).toBe(true);
        expect(handle.setInterim.mock.calls.map((c) => c[0])).toEqual(["…", ""]);
    });

    it("has no detail for a failure that isn't an Error", async () => {
        const { session } = await listen(true);
        fetchMock.mockImplementationOnce(async () => failJson(500, { error: "old" }));
        wavUtterance();
        await flush();
        fetchMock.mockImplementationOnce(async () => {
            throw "plain";
        });
        wavUtterance();
        await flush();
        expect(session.lastErrorDetail()).toBeNull();
    });

    it("applies utterances in the order they were spoken, one request at a time", async () => {
        const gates: ((v: unknown) => void)[] = [];
        const words = ["first", "second"];
        fetchMock.mockImplementation(
            () =>
                new Promise((resolve) => {
                    const word = words[gates.length];
                    gates.push(() => resolve(okJson({ text: word })));
                })
        );
        const { handle } = await listen(true);
        wavUtterance();
        wavUtterance();
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(1);
        gates[0](null);
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(2);
        gates[1](null);
        await flush();
        expect(handle.appendFinal.mock.calls.map((c) => c[0])).toEqual(["first", "second"]);
    });
});

describe("stopping", () => {
    it("releases everything: the level timer, the recorder, the graph and the microphone", async () => {
        const { session, handle } = await listen();
        const rec = recorder();
        const audio = ctx();
        h.rms = 0.5;
        session.toggleListening();
        await flush();
        expect(session.isListening()).toBe(false);
        expect(session.currentTargetId()).toBeNull();
        expect(rec.state).toBe("inactive");
        expect(rec.onstop).toBeNull();
        expect(audio.close).toHaveBeenCalledTimes(1);
        expect(tracks.map((t) => t.stop.mock.calls.length)).toEqual([1, 1]);
        expect(handle.setInterim).toHaveBeenLastCalledWith("");
        await tick(5_000);
        expect(FakeRecorder.instances.length).toBe(1);
    });

    it("clears the level timer", async () => {
        const { session } = await listen();
        expect(vi.getTimerCount()).toBe(1);
        session.toggleListening();
        await flush();
        expect(vi.getTimerCount()).toBe(0);
    });

    it("releases the microphone, the graph and the processor once however many errors stop it", async () => {
        fetchMock.mockImplementation(async () => failJson(501, { error: "no backend" }));
        await listen(true);
        const audio = ctx();
        const proc = audio.processor!;
        wavUtterance();
        wavUtterance();
        await flush();
        await flush();
        expect(fetchMock).toHaveBeenCalledTimes(2);
        expect(errors).toHaveBeenCalledTimes(2);
        expect(audio.close).toHaveBeenCalledTimes(1);
        expect(proc.disconnect).toHaveBeenCalledTimes(1);
        expect(tracks.map((t) => t.stop.mock.calls.length)).toEqual([1, 1]);
    });

    it("clears the target so the pane's mic button goes idle", async () => {
        const { session } = await listen();
        expect(session.currentTargetId()).toBe("blk");
        session.toggleListening();
        await flush();
        expect(session.currentTargetId()).toBeNull();
    });

    it("still releases the microphone when the recorder, the processor or the context fail to stop", async () => {
        const { session } = await listen();
        FakeRecorder.stopThrows = true;
        FakeAudioContext.failClose = true;
        session.toggleListening();
        await flush();
        expect(tracks.every((t) => t.stop.mock.calls.length === 1)).toBe(true);
        const local = await listen(true);
        ctx().processor!.disconnect.mockImplementation(() => {
            throw new Error("disconnect failed");
        });
        local.session.toggleListening();
        await flush();
        expect(tracks.every((t) => t.stop.mock.calls.length === 1)).toBe(true);
        expect(local.session.isListening()).toBe(false);
    });

    it("leaves a recorder that is already inactive alone", async () => {
        const { session } = await listen();
        const stop = vi.spyOn(recorder(), "stop");
        recorder().state = "inactive";
        session.toggleListening();
        await flush();
        expect(stop).not.toHaveBeenCalled();
    });

    it("can be stopped without a targeted pane", async () => {
        const { session } = await listen(false, false);
        session.toggleListening();
        await flush();
        expect(session.isListening()).toBe(false);
    });
});
