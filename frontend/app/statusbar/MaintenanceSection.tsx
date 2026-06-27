// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, getApi } from "@/store/global";
import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import "./_maintenance-section.scss";

// ── Types ────────────────────────────────────────────────────────────────────

type MigStep = {
    id: string;
    label: string;
    status: "running" | "done";
    durationMs: number | null;
    startedAtMs: number;
};

type MigState =
    | { kind: "idle"; pendingCount: number }
    | { kind: "running"; steps: MigStep[] }
    | { kind: "complete"; steps: MigStep[]; applied: number }
    | { kind: "failed"; steps: MigStep[]; error: string; failedId: string | null };

type VacState =
    | { kind: "idle"; lastRunMs: number | null }
    | { kind: "running" }
    | { kind: "done"; rowsDeleted: number; ranAtMs: number };

// ── Helpers ──────────────────────────────────────────────────────────────────

function fmtDate(ms: number): string {
    return new Date(ms).toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

function elapsedSecs(startedAtMs: number, nowMs: number): string {
    return `${((nowMs - startedAtMs) / 1000).toFixed(1)}s`;
}

function fmtDuration(ms: number): string {
    return `${(ms / 1000).toFixed(1)}s`;
}

// ── Component ────────────────────────────────────────────────────────────────

export const MaintenanceSection = (): JSX.Element => {
    const updaterStatus = atoms.updaterStatusAtom;
    const updaterVersion = atoms.updaterVersionAtom;

    const [migState, setMigState] = createSignal<MigState>({ kind: "idle", pendingCount: 0 });
    const [vacState, setVacState] = createSignal<VacState>({ kind: "idle", lastRunMs: null });
    const [nowMs, setNowMs] = createSignal(Date.now());

    // Fetch pending_migrations count from backend info on mount
    onMount(() => {
        getApi().getBackendInfo().then((info) => {
            const pending = info?.pending_migrations ?? 0;
            setMigState({ kind: "idle", pendingCount: pending });
        }).catch(() => {});
    });

    // Timer — ticks every 500ms only while a migration is running to drive elapsed timers.
    onMount(() => {
        const id = setInterval(() => {
            if (migState().kind === "running") {
                setNowMs(Date.now());
            }
        }, 500);
        onCleanup(() => clearInterval(id));
    });

    // Subscribe to migration progress events (from CEF, not WPS WebSocket)
    onMount(() => {
        let unlisten: (() => void) | null = null;
        getApi().listen("upgrade:migration-event", (payload: any) => {
            const kind: string = payload?.kind ?? "";
            if (kind === "start") {
                const id: string = payload.id ?? "";
                const label: string = payload.label ?? id;
                setMigState((prev) => {
                    const steps: MigStep[] = prev.kind === "running" ? [...prev.steps] : [];
                    steps.push({ id, label, status: "running", durationMs: null, startedAtMs: Date.now() });
                    return { kind: "running", steps };
                });
            } else if (kind === "done") {
                const id: string = payload.id ?? "";
                const durationMs: number | null = payload.duration_ms ?? null;
                setMigState((prev) => {
                    if (prev.kind !== "running") return prev;
                    const steps = prev.steps.map((s) =>
                        s.id === id ? { ...s, status: "done" as const, durationMs } : s
                    );
                    return { ...prev, steps };
                });
            } else if (kind === "complete") {
                const applied: number = payload.applied ?? 0;
                setMigState((prev) => {
                    const steps = prev.kind === "running" ? prev.steps : [];
                    return { kind: "complete", steps, applied };
                });
            } else if (kind === "error") {
                const error: string = payload.error ?? "Unknown error";
                const failedId: string | null = payload.id ?? null;
                setMigState((prev) => {
                    const steps = prev.kind === "running"
                        ? prev.steps.map((s) =>
                            s.id === failedId ? { ...s, status: "done" as const } : s
                          )
                        : [];
                    return { kind: "failed", steps, error, failedId };
                });
            }
        }).then((fn) => { unlisten = fn; });

        onCleanup(() => unlisten?.());
    });

    onMount(() => {
        let unlisten: (() => void) | null = null;
        getApi().listen("upgrade:migrations-complete", (_payload: any) => {
            setMigState((prev) => {
                const steps = prev.kind === "running" ? prev.steps : [];
                const applied = prev.kind === "running" ? steps.filter((s) => s.status === "done").length : 0;
                return { kind: "complete", steps, applied };
            });
        }).then((fn) => { unlisten = fn; });

        onCleanup(() => unlisten?.());
    });

    onMount(() => {
        let unlisten: (() => void) | null = null;
        getApi().listen("upgrade:migrations-failed", (payload: any) => {
            const error: string = payload?.error ?? "Migration failed";
            const failedId: string | null = payload?.failedId ?? null;
            setMigState((prev) => {
                const steps = prev.kind === "running" ? prev.steps : [];
                return { kind: "failed", steps, error, failedId };
            });
        }).then((fn) => { unlisten = fn; });

        onCleanup(() => unlisten?.());
    });

    onMount(() => {
        let unlisten: (() => void) | null = null;
        getApi().listen("upgrade:saga-vacuum-done", (payload: any) => {
            const rowsDeleted: number = payload?.rows_deleted ?? 0;
            const ranAtMs = Date.now();
            setVacState({ kind: "done", rowsDeleted, ranAtMs });
            setTimeout(() => {
                setVacState({ kind: "idle", lastRunMs: ranAtMs });
            }, 3000);
        }).then((fn) => { unlisten = fn; });

        onCleanup(() => unlisten?.());
    });

    // ── Actions ───────────────────────────────────────────────────────────────

    const handleRunMigrations = async () => {
        if (migState().kind === "running") return;
        setMigState({ kind: "running", steps: [] });
        try {
            await getApi().runMigrations();
        } catch (e: any) {
            setMigState({ kind: "failed", steps: [], error: String(e?.message ?? e), failedId: null });
        }
    };

    const handleRunVacuum = async () => {
        if (vacState().kind === "running") return;
        setVacState({ kind: "running" });
        try {
            await getApi().runSagaVacuum();
        } catch {
            setVacState({ kind: "idle", lastRunMs: null });
        }
    };

    const handleInstallUpdate = () => {
        getApi().installAppUpdate();
    };

    // ── Derived ───────────────────────────────────────────────────────────────

    const badge = (): string | null => {
        const mig = migState();
        if (mig.kind === "failed") return `✗ failed`;
        if (mig.kind === "idle" && mig.pendingCount > 0) return `⚠ ${mig.pendingCount} pending`;
        if (mig.kind === "running") return "·running";
        const upd = updaterStatus();
        if (upd === "available") {
            const ver = updaterVersion();
            return ver ? `↓ ${ver}` : "↓ update";
        }
        if (upd === "downloading") return "↓ …";
        if (upd === "ready") return "↑ ready";
        return null;
    };

    // ── Render ────────────────────────────────────────────────────────────────

    return (
        <>
            <div class="instance-panel-section-title">
                Maintenance
                <Show when={badge() != null}>
                    <span class={`maintenance-badge${migState().kind === "failed" ? " maintenance-badge--error" : ""}`}>{badge()}</span>
                </Show>
            </div>

            {/* ── Update row ─────────────────────────────────────────────── */}
            <Show when={updaterStatus() !== "up-to-date" && updaterStatus() !== "checking"}>
                <div class="maintenance-update-row">
                    <Show when={updaterStatus() === "available"}>
                        <span class="maintenance-icon maintenance-icon--update">↓</span>
                        <span class="maintenance-row-text">
                            {updaterVersion() ? `v${updaterVersion()} available` : "Update available"}
                        </span>
                        <button
                            type="button"
                            class="maintenance-btn maintenance-btn--primary"
                            onClick={handleInstallUpdate}
                        >
                            Download
                        </button>
                    </Show>
                    <Show when={updaterStatus() === "downloading"}>
                        <span class="maintenance-icon maintenance-icon--working">↓</span>
                        <span class="maintenance-row-text maintenance-row-text--dim">Downloading…</span>
                    </Show>
                    <Show when={updaterStatus() === "ready"}>
                        <span class="maintenance-icon maintenance-icon--update">↑</span>
                        <span class="maintenance-row-text">
                            {updaterVersion() ? `v${updaterVersion()} ready` : "Ready to install"}
                        </span>
                        <button
                            type="button"
                            class="maintenance-btn maintenance-btn--primary"
                            onClick={handleInstallUpdate}
                        >
                            Restart
                        </button>
                    </Show>
                    <Show when={updaterStatus() === "error"}>
                        <span class="maintenance-icon maintenance-icon--error">✗</span>
                        <span class="maintenance-row-text maintenance-row-text--error">Update failed</span>
                    </Show>
                </div>
            </Show>

            {/* ── Migration rows ─────────────────────────────────────────── */}
            <Show when={migState().kind === "idle" && (migState() as Extract<MigState, {kind: "idle"}>).pendingCount > 0}>
                <div class="maintenance-alert-row">
                    <span class="maintenance-icon maintenance-icon--warn">⚠</span>
                    <span class="maintenance-row-text maintenance-row-text--warn">
                        {(migState() as Extract<MigState, {kind: "idle"}>).pendingCount} migration{
                            (migState() as Extract<MigState, {kind: "idle"}>).pendingCount !== 1 ? "s" : ""
                        } pending
                    </span>
                    <button
                        type="button"
                        class="maintenance-btn maintenance-btn--primary"
                        onClick={handleRunMigrations}
                    >
                        Run
                    </button>
                </div>
            </Show>

            <Show when={migState().kind === "running" || migState().kind === "complete" || migState().kind === "failed"}>
                <div class="maintenance-migration-block">
                    <Show when={migState().kind === "running"}>
                        <div class="maintenance-stage-header">
                            <span class="maintenance-icon maintenance-icon--working maintenance-pulse">●</span>
                            <span>Migrations running…</span>
                        </div>
                    </Show>
                    <Show when={migState().kind === "complete"}>
                        <div class="maintenance-stage-header">
                            <span class="maintenance-icon maintenance-icon--ok">✓</span>
                            <span>
                                Migrations complete
                                {" — "}
                                {(migState() as Extract<MigState, {kind: "complete"}>).applied} applied
                            </span>
                        </div>
                    </Show>
                    <Show when={migState().kind === "failed"}>
                        <div class="maintenance-stage-header">
                            <span class="maintenance-icon maintenance-icon--error">✗</span>
                            <span class="maintenance-row-text--error">
                                Migration failed
                            </span>
                        </div>
                        <div class="maintenance-error-detail">
                            {(migState() as Extract<MigState, {kind: "failed"}>).error}
                        </div>
                    </Show>
                    <For each={(migState() as Extract<MigState, {kind: "running" | "complete" | "failed"}>).steps ?? []}>
                        {(step) => (
                            <div class="maintenance-sub-row">
                                <span class={`maintenance-icon ${step.status === "done" ? "maintenance-icon--ok" : "maintenance-icon--working maintenance-pulse"}`}>
                                    {step.status === "done" ? "✓" : "·"}
                                </span>
                                <span class="maintenance-sub-id">{step.id}</span>
                                <span class="maintenance-sub-label">{step.label}</span>
                                <span class="maintenance-timer">
                                    {step.durationMs != null
                                        ? fmtDuration(step.durationMs)
                                        : elapsedSecs(step.startedAtMs, nowMs())}
                                </span>
                            </div>
                        )}
                    </For>
                    <Show when={migState().kind === "failed"}>
                        <button
                            type="button"
                            class="maintenance-btn maintenance-btn--primary maintenance-retry-btn"
                            onClick={handleRunMigrations}
                        >
                            Retry
                        </button>
                    </Show>
                </div>
            </Show>

            {/* ── Summary rows (shown when idle / after-complete) ─────────── */}
            <Show when={migState().kind === "idle"}>
                <div class="maintenance-summary-row">
                    <Show
                        when={(migState() as Extract<MigState, {kind: "idle"}>).pendingCount === 0}
                        fallback={
                            <>
                                <span class="maintenance-icon maintenance-icon--warn">⚠</span>
                                <span class="maintenance-row-label">Migrations</span>
                                <span class="maintenance-row-text--warn">
                                    {(migState() as Extract<MigState, {kind: "idle"}>).pendingCount} pending
                                </span>
                            </>
                        }
                    >
                        <span class="maintenance-icon maintenance-icon--ok">✓</span>
                        <span class="maintenance-row-label">Migrations</span>
                        <span class="maintenance-row-text--dim">up to date</span>
                    </Show>
                </div>
            </Show>

            {/* ── Saga vacuum row ────────────────────────────────────────── */}
            <div class="maintenance-summary-row">
                <Show when={vacState().kind === "idle"}>
                    <span class="maintenance-icon maintenance-icon--neutral">○</span>
                    <span class="maintenance-row-label">Vacuum</span>
                    <span class="maintenance-row-text--dim">
                        {(vacState() as Extract<VacState, {kind: "idle"}>).lastRunMs
                            ? `last run: ${fmtDate((vacState() as Extract<VacState, {kind: "idle"}>).lastRunMs!)}`
                            : "not run"}
                    </span>
                    <button
                        type="button"
                        class="maintenance-btn"
                        onClick={handleRunVacuum}
                    >
                        Run
                    </button>
                </Show>
                <Show when={vacState().kind === "running"}>
                    <span class="maintenance-icon maintenance-icon--working maintenance-pulse">·</span>
                    <span class="maintenance-row-label">Vacuum</span>
                    <span class="maintenance-row-text--dim">running…</span>
                </Show>
                <Show when={vacState().kind === "done"}>
                    <span class="maintenance-icon maintenance-icon--ok">✓</span>
                    <span class="maintenance-row-label">Vacuum</span>
                    <span class="maintenance-row-text--dim">
                        {(vacState() as Extract<VacState, {kind: "done"}>).rowsDeleted === 0
                            ? "nothing to clean"
                            : `${(vacState() as Extract<VacState, {kind: "done"}>).rowsDeleted} rows removed`}
                    </span>
                    <button type="button" class="maintenance-btn" onClick={handleRunVacuum}>Run</button>
                </Show>
            </div>
        </>
    );
};

MaintenanceSection.displayName = "MaintenanceSection";
