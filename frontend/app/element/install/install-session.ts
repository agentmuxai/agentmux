// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * InstallSession — the reactive model behind every install surface
 * (SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md §5.1). It starts one
 * install, subscribes to its `install:<sessionId>` scope, and is the ONLY
 * code that parses `install_chunk` events. A `StepTracker` turns the lines
 * into Layer 1 steps; the retained log feeds Layer 2.
 *
 * Must be created inside a Solid owner (a component): it registers its
 * own cleanup.
 */

import { createMemo, createSignal, onCleanup, type Accessor } from "solid-js";

import { muxEventSubscribe } from "@/app/store/mps";

import type { InstallFailure, InstallStep, LineTone, StepTracker } from "./install-types";

export type InstallState = "idle" | "running" | "done" | "failed";

export interface LogLine {
    text: string;
    tone: LineTone;
}

/** An `install_chunk` event payload: a log line, or the final `done`. */
interface InstallChunk {
    line?: string;
    stream?: "stdout" | "stderr";
    op?: "done";
    ok?: boolean;
    error?: unknown;
}

// Frontend view cap (spec §4.2). The backend keeps the complete log.
export const MAX_LOG_LINES = 20_000;
const TRIM_CHUNK = 1_000;
const TICK_MS = 250;

export interface InstallSessionOptions {
    /**
     * Builds a tracker. Called reactively for the idle plan preview (so a
     * name that resolves later updates it) and once per run.
     */
    tracker: () => StepTracker;
    /** Starts the install on the backend; resolves with its session id. */
    begin: () => Promise<{ sessionId: string }>;
    /** Asks the backend to stop a session. Omit when the install can't be stopped safely. */
    cancel?: (sessionId: string) => Promise<unknown>;
    /**
     * Cancel a running install when the owner unmounts. True for npm into
     * an isolated directory; false for package managers, which must run to
     * completion (SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md §3.4).
     */
    cancelOnDispose?: boolean;
    /** Fires once per run when the `done` event arrives. */
    onDone?: (ok: boolean) => void;
    /** Clock, injectable for tests. */
    now?: () => number;
}

export interface InstallSession {
    state: Accessor<InstallState>;
    steps: Accessor<InstallStep[]>;
    /** Set when the run failed; the Layer 1 explanation. */
    failure: Accessor<InstallFailure | null>;
    /** The raw `done` error (string or typed AgentMuxError), for callers that show it. */
    error: Accessor<unknown>;
    elapsedMs: Accessor<number>;
    sessionId: Accessor<string | null>;
    /** The retained log. Re-read on every append. */
    lines: Accessor<readonly LogLine[]>;
    /** How many earlier lines were dropped to stay under MAX_LOG_LINES. */
    trimmedLines: Accessor<number>;
    /** Whether `cancel()` can stop the current run. */
    canCancel: () => boolean;
    start(): Promise<void>;
    cancel(): Promise<void>;
    /** The retained log as plain text, for Copy all. */
    logText(): string;
}

export function createInstallSession(opts: InstallSessionOptions): InstallSession {
    const now = opts.now ?? Date.now;
    const [state, setState] = createSignal<InstallState>("idle");
    const [failure, setFailure] = createSignal<InstallFailure | null>(null);
    const [error, setError] = createSignal<unknown>(null);
    const [elapsedMs, setElapsedMs] = createSignal(0);
    const [sessionId, setSessionId] = createSignal<string | null>(null);
    const [runSteps, setRunSteps] = createSignal<InstallStep[] | null>(null);
    const [logVersion, bumpLog] = createSignal(0);
    const [trimmedLines, setTrimmedLines] = createSignal(0);

    // Before the first run the plan is shown with every step pending, so
    // the user sees what will happen before starting.
    const idleSteps = createMemo(() => opts.tracker().snapshot());
    const steps = () => runSteps() ?? idleSteps();

    let tracker: StepTracker | null = null;
    let log: LogLine[] = [];
    let unsub: (() => void) | null = null;
    let tickHandle: ReturnType<typeof setInterval> | null = null;
    let startedAt = 0;
    let disposed = false;
    let run = 0;

    const sync = () => {
        if (tracker) setRunSteps(tracker.snapshot());
    };

    const stopTicking = () => {
        if (tickHandle != null) {
            clearInterval(tickHandle);
            tickHandle = null;
        }
    };

    const append = (text: string, tone: LineTone) => {
        log.push({ text, tone });
        if (log.length > MAX_LOG_LINES) {
            log = log.slice(TRIM_CHUNK);
            setTrimmedLines((n) => n + TRIM_CHUNK);
        }
        bumpLog((v) => v + 1);
    };

    const finish = (ok: boolean, err: unknown) => {
        stopTicking();
        setElapsedMs(now() - startedAt);
        if (ok) {
            tracker?.succeed();
            sync();
            setState("done");
        } else {
            setError(err);
            // The backend's own reason goes into Details as a final error
            // line. When the start itself fails nothing else ran, so
            // without this Details would be empty and the cause invisible.
            // Appended after every tracked line, so the tracker's
            // first-error index still points at the tool's own first error.
            if (typeof err === "string" && err !== "cancelled") append(`Install failed: ${err}`, "error");
            if (tracker) {
                setFailure(tracker.fail(err));
                sync();
            }
            setState("failed");
        }
        opts.onDone?.(ok);
    };

    const start = async () => {
        const thisRun = ++run;
        unsub?.();
        unsub = null;
        stopTicking();
        log = [];
        setTrimmedLines(0);
        bumpLog((v) => v + 1);
        setSessionId(null);
        setFailure(null);
        setError(null);
        tracker = opts.tracker();
        tracker.start();
        sync();
        setState("running");
        startedAt = now();
        setElapsedMs(0);
        tickHandle = setInterval(() => {
            setElapsedMs(now() - startedAt);
            tracker?.tick(now());
            sync();
        }, TICK_MS);

        let sid: string;
        try {
            sid = (await opts.begin()).sessionId;
        } catch (e) {
            if (!disposed && thisRun === run) finish(false, (e as Error)?.message ?? String(e));
            return;
        }
        // Unmounted (or restarted) while the start RPC was in flight.
        if (disposed || thisRun !== run) {
            if (disposed && opts.cancelOnDispose) void opts.cancel?.(sid).catch(() => {});
            return;
        }
        setSessionId(sid);
        unsub = muxEventSubscribe({
            eventType: "install_chunk",
            scope: `install:${sid}`,
            handler: (event: { data?: InstallChunk }) => {
                const data = event?.data;
                if (!data || typeof data !== "object" || state() !== "running") return;
                if (typeof data.line === "string") {
                    append(data.line, tracker?.line(data.line, now()) ?? "normal");
                    sync();
                } else if (data.op === "done") {
                    finish(!!data.ok, data.error ?? "install failed");
                }
            },
        });
    };

    const cancel = async () => {
        const sid = sessionId();
        if (sid && opts.cancel && state() === "running") {
            try {
                await opts.cancel(sid);
            } catch {
                /* best-effort */
            }
        }
    };

    onCleanup(() => {
        disposed = true;
        unsub?.();
        unsub = null;
        stopTicking();
        const sid = sessionId();
        if (opts.cancelOnDispose && sid && state() === "running") {
            void opts.cancel?.(sid).catch(() => {});
        }
    });

    return {
        state,
        steps,
        failure,
        error,
        elapsedMs,
        sessionId,
        lines: () => {
            logVersion();
            return log;
        },
        trimmedLines,
        canCancel: () => !!opts.cancel && state() === "running",
        start,
        cancel,
        logText: () => {
            const text = log.map((l) => l.text);
            if (trimmedLines() > 0) text.unshift(`[${trimmedLines()} earlier lines trimmed]`);
            return text.join("\n");
        },
    };
}
